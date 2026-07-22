//! `lao-cli worker install/uninstall/start/stop/restart/status/logs` — turns the
//! worker from a manually launched process into a systemd-managed appliance: a
//! dedicated non-root service account, protected config/token files, auto-start after
//! boot/network/Tailscale, auto-restart on crash. Linux-only (systemd); every entry
//! point checks `Platform::is_linux()` first and fails with a clear error elsewhere.
//!
//! `install` grants the service account read access to whatever model/llama-server
//! paths the config references via a single minimal ACL entry (execute-only) on the
//! referenced files' home directory, rather than moving files or loosening broader
//! permissions — see the plan this was built from for the reasoning.

use lao_orchestrator_core::cross_platform::Platform;
use lao_orchestrator_core::model::ModelRegistry;
use lao_worker::config::WorkerConfig;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const SERVICE_ACCOUNT: &str = "lao-worker";
const SERVICE_HOME: &str = "/var/lib/lao-worker";
const SERVICE_NAME: &str = "lao-worker.service";
const SYSUSERS_CONF_PATH: &str = "/etc/sysusers.d/lao-worker.conf";
const BIN_DIR: &str = "/opt/lao-worker/bin";
const BIN_PATH: &str = "/opt/lao-worker/bin/lao-cli";
const ETC_DIR: &str = "/etc/lao-worker";
const CONFIG_DEST: &str = "/etc/lao-worker/lao.toml";
const ENV_FILE_PATH: &str = "/etc/lao-worker/worker.env";
const UNIT_PATH: &str = "/etc/systemd/system/lao-worker.service";

fn fail(message: &str) -> ! {
    eprintln!("[ERROR] {}", message);
    std::process::exit(1);
}

fn require_linux() {
    if !Platform::is_linux() {
        fail("worker lifecycle management requires systemd (Linux only)");
    }
}

fn require_root() {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok());
    if uid != Some(0) {
        fail("requires root - re-run with sudo");
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {}: {}", cmd, e))
}

fn run_checked(cmd: &str, args: &[&str]) -> Result<(), String> {
    let output = run(cmd, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} {} failed: {}",
            cmd,
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Run with inherited stdio (so the child's own output streams live, matching how
/// `systemctl`/`journalctl` are normally used) and return its exit code.
fn run_inherited(cmd: &str, args: &[&str]) -> i32 {
    match Command::new(cmd).args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("[ERROR] failed to run {}: {}", cmd, e);
            1
        }
    }
}

fn run_inherited_or_exit(cmd: &str, args: &[&str]) {
    let code = run_inherited(cmd, args);
    if code != 0 {
        std::process::exit(code);
    }
}

fn write_file(path: &str, contents: &str, mode: u32) -> std::io::Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

fn generate_sysusers_conf() -> String {
    format!(
        "u {account} - \"LAO worker service account\" {home} /usr/sbin/nologin\n",
        account = SERVICE_ACCOUNT,
        home = SERVICE_HOME,
    )
}

/// Pure so it can be unit-tested without touching the filesystem. `TimeoutStopSec` is
/// derived from the config's actual `shutdown_grace_seconds` (not hardcoded) so
/// systemd never SIGKILLs the process before its own graceful drain finishes.
fn generate_unit_file(cfg: &WorkerConfig) -> String {
    let timeout_stop = cfg.shutdown_grace_seconds + 10;
    format!(
        "[Unit]\n\
         Description=LAO model-inference worker ({id})\n\
         After=network-online.target tailscaled.service\n\
         Wants=network-online.target\n\
         StartLimitIntervalSec=60\n\
         StartLimitBurst=5\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={account}\n\
         Group={account}\n\
         EnvironmentFile=-{env_path}\n\
         ExecStart={bin_path} worker serve --config {config_path}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         TimeoutStopSec={timeout_stop}\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ProtectHome=read-only\n\
         PrivateTmp=true\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        id = cfg.id,
        account = SERVICE_ACCOUNT,
        env_path = ENV_FILE_PATH,
        bin_path = BIN_PATH,
        config_path = CONFIG_DEST,
        timeout_stop = timeout_stop,
    )
}

/// For every referenced path that lives under `/home/<user>`, return that user's home
/// directory (deduplicated) — the one directory whose traversal bit needs the ACL
/// grant, since everything else this project needs is already world-readable in
/// practice (confirmed: model files and the llama-server build are `644`/`755`).
fn home_dirs_needing_acl(model_paths: &[PathBuf], server_executable: &str) -> BTreeSet<PathBuf> {
    let mut candidates: Vec<PathBuf> = model_paths.to_vec();
    let server_path = PathBuf::from(server_executable);
    if server_path.is_absolute() {
        candidates.push(server_path);
    }
    candidates
        .into_iter()
        .filter_map(|path| {
            let stripped = path.strip_prefix("/home").ok()?;
            let user = stripped.components().next()?;
            Some(PathBuf::from("/home").join(user.as_os_str()))
        })
        .collect()
}

/// Parses the same config text two ways (worker settings + model registry) to find
/// every path that needs ACL traversal access, used by both install and uninstall so
/// uninstall removes exactly what install granted.
fn home_dirs_from_config_text(text: &str) -> BTreeSet<PathBuf> {
    let server_executable = WorkerConfig::from_toml_str(text)
        .map(|c| c.runtime.llama_cpp.server_executable)
        .unwrap_or_default();
    let model_paths: Vec<PathBuf> = ModelRegistry::from_toml_str(text)
        .unwrap_or_default()
        .all_resolved()
        .into_iter()
        .map(|r| r.entry.path)
        .collect();
    home_dirs_needing_acl(&model_paths, &server_executable)
}

/// Reads the bearer token out of the installed `worker.env` (`VAR=value`, written by
/// `install`) so `status`'s own health probe can authenticate against a worker that
/// requires it, the same way every other worker-facing health check in this project
/// already does. Only ever readable by root or the `lao-worker` group in practice
/// (file mode `600`), so this silently yields nothing for an unprivileged caller
/// rather than erroring — matching how the config read above already behaves.
fn read_worker_token() -> Option<String> {
    let content = std::fs::read_to_string(ENV_FILE_PATH).ok()?;
    let line = content.lines().next()?;
    let (_, value) = line.split_once('=')?;
    Some(value.to_string())
}

pub fn worker_install(config: Option<String>) {
    require_linux();
    require_root();

    let config_path = config.unwrap_or_else(|| "lao.toml".to_string());
    let config_text = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| fail(&format!("reading {}: {}", config_path, e)));
    let worker_config =
        WorkerConfig::from_toml_str(&config_text).unwrap_or_else(|e| fail(&format!("{}", e)));

    // Resolve the auth token from our own (root's) environment before touching
    // anything, so a missing token fails fast instead of leaving a half-installed
    // service. sudo does not forward the caller's environment by default, which is
    // the most likely reason this fails even when the operator "already set it".
    let token_value = worker_config.resolve_auth_token().unwrap_or_else(|e| {
        fail(&format!(
            "{}. Since this command must run as root, re-run as `sudo -E lao-cli worker install ...` \
             or `sudo <VAR>=<token> lao-cli worker install ...` so the variable is visible to root.",
            e
        ))
    });
    let token_line = token_value.map(|value| {
        let var = worker_config
            .auth
            .token_env
            .as_ref()
            .expect("resolve_auth_token returned Some without token_env set");
        format!("{}={}\n", var, value)
    });

    let home_dirs = home_dirs_from_config_text(&config_text);

    // 1. Dedicated service account, created idempotently via systemd-sysusers.
    write_file(SYSUSERS_CONF_PATH, &generate_sysusers_conf(), 0o644)
        .unwrap_or_else(|e| fail(&format!("writing {}: {}", SYSUSERS_CONF_PATH, e)));
    run_checked("systemd-sysusers", &[SYSUSERS_CONF_PATH]).unwrap_or_else(|e| fail(&e));

    // 2. Install the binary this command was invoked from. Copy to a temp file in
    // the same directory and rename() over the destination rather than overwriting
    // it directly - a plain overwrite fails with "Text file busy" whenever the
    // service is already running the file being replaced (the normal case for
    // reinstalling/upgrading an active worker), since rename() atomically repoints
    // the directory entry without needing the old inode to be unmapped, while a
    // direct write does.
    std::fs::create_dir_all(BIN_DIR)
        .unwrap_or_else(|e| fail(&format!("creating {}: {}", BIN_DIR, e)));
    let current_exe = std::env::current_exe()
        .unwrap_or_else(|e| fail(&format!("resolving current executable: {}", e)));
    let bin_tmp_path = format!("{}.new", BIN_PATH);
    std::fs::copy(&current_exe, &bin_tmp_path)
        .unwrap_or_else(|e| fail(&format!("installing binary to {}: {}", bin_tmp_path, e)));
    std::fs::set_permissions(&bin_tmp_path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|e| fail(&format!("setting permissions on {}: {}", bin_tmp_path, e)));
    std::fs::rename(&bin_tmp_path, BIN_PATH)
        .unwrap_or_else(|e| fail(&format!("installing binary to {}: {}", BIN_PATH, e)));

    // 3. Config, readable by the service account via group membership.
    std::fs::create_dir_all(ETC_DIR)
        .unwrap_or_else(|e| fail(&format!("creating {}: {}", ETC_DIR, e)));
    std::fs::write(CONFIG_DEST, &config_text)
        .unwrap_or_else(|e| fail(&format!("writing {}: {}", CONFIG_DEST, e)));
    std::fs::set_permissions(CONFIG_DEST, std::fs::Permissions::from_mode(0o640))
        .unwrap_or_else(|e| fail(&format!("setting permissions on {}: {}", CONFIG_DEST, e)));
    run_checked(
        "chown",
        &[&format!("root:{}", SERVICE_ACCOUNT), CONFIG_DEST],
    )
    .unwrap_or_else(|e| fail(&e));

    // 4. Token file — only created when auth is actually enabled. The unit's
    // `EnvironmentFile=-...` (leading '-') tolerates it being absent.
    if let Some(line) = &token_line {
        write_file(ENV_FILE_PATH, line, 0o600)
            .unwrap_or_else(|e| fail(&format!("writing {}: {}", ENV_FILE_PATH, e)));
        run_checked(
            "chown",
            &[
                &format!("{account}:{account}", account = SERVICE_ACCOUNT),
                ENV_FILE_PATH,
            ],
        )
        .unwrap_or_else(|e| fail(&e));
    }

    // 5. The unit itself.
    write_file(UNIT_PATH, &generate_unit_file(&worker_config), 0o644)
        .unwrap_or_else(|e| fail(&format!("writing {}: {}", UNIT_PATH, e)));

    // 6. Grant exactly the traversal access needed, nothing more.
    for dir in &home_dirs {
        run_checked(
            "setfacl",
            &[
                "-m",
                &format!("u:{}:--x", SERVICE_ACCOUNT),
                &dir.to_string_lossy(),
            ],
        )
        .unwrap_or_else(|e| fail(&e));
    }

    run_checked("systemctl", &["daemon-reload"]).unwrap_or_else(|e| fail(&e));
    run_checked("systemctl", &["enable", SERVICE_NAME]).unwrap_or_else(|e| fail(&e));

    let already_active = run("systemctl", &["is-active", "--quiet", SERVICE_NAME])
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !already_active {
        run_checked("systemctl", &["start", SERVICE_NAME]).unwrap_or_else(|e| fail(&e));
        println!("Installed and started {}.", SERVICE_NAME);
    } else {
        println!(
            "Installed {} (already running — use `lao-cli worker restart` to pick up changes).",
            SERVICE_NAME
        );
    }
}

pub fn worker_uninstall(purge_user: bool) {
    require_linux();
    require_root();

    let home_dirs = std::fs::read_to_string(CONFIG_DEST)
        .map(|text| home_dirs_from_config_text(&text))
        .unwrap_or_default();

    if let Err(e) = run_checked("systemctl", &["disable", "--now", SERVICE_NAME]) {
        eprintln!("[WARN] {}", e);
    }

    for dir in &home_dirs {
        if let Err(e) = run_checked(
            "setfacl",
            &[
                "-x",
                &format!("u:{}", SERVICE_ACCOUNT),
                &dir.to_string_lossy(),
            ],
        ) {
            eprintln!("[WARN] {}", e);
        }
    }

    for path in [UNIT_PATH, SYSUSERS_CONF_PATH] {
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[WARN] removing {}: {}", path, e);
            }
        }
    }
    for dir in [ETC_DIR, "/opt/lao-worker"] {
        if let Err(e) = std::fs::remove_dir_all(dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[WARN] removing {}: {}", dir, e);
            }
        }
    }

    if let Err(e) = run_checked("systemctl", &["daemon-reload"]) {
        eprintln!("[WARN] {}", e);
    }

    if purge_user {
        if let Err(e) = run_checked("userdel", &[SERVICE_ACCOUNT]) {
            eprintln!("[WARN] {}", e);
        }
    }

    println!(
        "Uninstalled {}{}.",
        SERVICE_NAME,
        if purge_user {
            " and removed the service account"
        } else {
            ""
        }
    );
}

pub fn worker_start() {
    require_linux();
    require_root();
    run_inherited_or_exit("systemctl", &["start", SERVICE_NAME]);
    println!("Started.");
}

pub fn worker_stop() {
    require_linux();
    require_root();
    run_inherited_or_exit("systemctl", &["stop", SERVICE_NAME]);
    println!("Stopped.");
}

pub fn worker_restart() {
    require_linux();
    require_root();
    run_inherited_or_exit("systemctl", &["restart", SERVICE_NAME]);
    println!("Restarted.");
}

pub fn worker_status() {
    require_linux();
    let systemctl_code = run_inherited("systemctl", &["status", SERVICE_NAME, "--no-pager"]);

    if let Ok(text) = std::fs::read_to_string(CONFIG_DEST) {
        if let Ok(cfg) = WorkerConfig::from_toml_str(&text) {
            let url = format!("http://{}/v1/health", cfg.bind);
            let mut req = reqwest::blocking::Client::new()
                .get(&url)
                .timeout(std::time::Duration::from_secs(3));
            if let Some(token) = read_worker_token() {
                req = req.bearer_auth(token);
            }
            let health = req.send();
            match health {
                Ok(r) if r.status().is_success() => println!("backend: ok ({})", url),
                Ok(r) => println!("backend: unhealthy (HTTP {})", r.status()),
                Err(_) => println!("backend: unreachable ({})", url),
            }
        }
    }

    // std::process::exit() skips flushing Rust's own buffered stdout (unlike a
    // normal return from main, or the child processes above which write through
    // inherited stdio directly) — without this, the println!s above are silently
    // lost whenever stdout isn't a TTY, e.g. piped over SSH.
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(systemctl_code);
}

pub fn worker_logs(follow: bool, lines: Option<u32>) {
    require_linux();
    let lines_str = lines.map(|n| n.to_string());
    let mut args = vec!["-u", SERVICE_NAME, "--no-pager"];
    if let Some(n) = &lines_str {
        args.push("-n");
        args.push(n);
    }
    if follow {
        args.push("-f");
    }
    run_inherited_or_exit("journalctl", &args);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> WorkerConfig {
        WorkerConfig::from_toml_str(
            "[worker]\n\
             id = \"fedora-worker\"\n\
             bind = \"100.64.0.1:9847\"\n\
             shutdown_grace_seconds = 20\n\
             [worker.auth]\n\
             enabled = true\n\
             token_env = \"LAO_TEST_TOKEN\"\n",
        )
        .unwrap()
    }

    #[test]
    fn unit_file_uses_the_configured_worker_id() {
        let unit = generate_unit_file(&sample_config());
        assert!(unit.contains("Description=LAO model-inference worker (fedora-worker)"));
    }

    #[test]
    fn unit_file_timeout_stop_sec_tracks_shutdown_grace_seconds_with_margin() {
        let unit = generate_unit_file(&sample_config());
        assert!(
            unit.contains("TimeoutStopSec=30"),
            "expected shutdown_grace_seconds (20) + 10 margin; got:\n{}",
            unit
        );
    }

    #[test]
    fn unit_file_references_the_installed_binary_and_config_paths() {
        let unit = generate_unit_file(&sample_config());
        assert!(unit.contains(&format!(
            "ExecStart={} worker serve --config {}",
            BIN_PATH, CONFIG_DEST
        )));
    }

    #[test]
    fn unit_file_environment_file_tolerates_a_missing_token_file() {
        // Leading '-' means "load if present, don't fail if absent" - required since
        // no token file is written at all when worker.auth.enabled = false.
        let unit = generate_unit_file(&sample_config());
        assert!(unit.contains(&format!("EnvironmentFile=-{}", ENV_FILE_PATH)));
    }

    #[test]
    fn unit_file_protects_home_read_only_not_hidden() {
        // ProtectHome=true would hide /home entirely regardless of the ACL grant;
        // read-only is what actually allows the granted traversal to work.
        let unit = generate_unit_file(&sample_config());
        assert!(unit.contains("ProtectHome=read-only"));
        assert!(!unit.contains("ProtectHome=true"));
    }

    #[test]
    fn sysusers_conf_declares_a_nologin_system_account() {
        let conf = generate_sysusers_conf();
        assert_eq!(
            conf,
            "u lao-worker - \"LAO worker service account\" /var/lib/lao-worker /usr/sbin/nologin\n"
        );
    }

    #[test]
    fn home_dir_detection_finds_the_owning_users_home_for_each_referenced_path() {
        let paths = vec![
            PathBuf::from("/home/jakea/models/Qwen3-8B-Q4_K_M.gguf"),
            PathBuf::from("/home/jakea/src/llama.cpp/build/bin/llama-server"),
        ];
        let dirs =
            home_dirs_needing_acl(&paths, "/home/jakea/src/llama.cpp/build/bin/llama-server");
        assert_eq!(dirs.len(), 1);
        assert!(dirs.contains(&PathBuf::from("/home/jakea")));
    }

    #[test]
    fn home_dir_detection_ignores_paths_outside_home() {
        let paths = vec![PathBuf::from("/opt/lao-worker/models/model.gguf")];
        let dirs = home_dirs_needing_acl(&paths, "llama-server");
        assert!(
            dirs.is_empty(),
            "a bare command name and an /opt path need no ACL grant"
        );
    }

    #[test]
    fn home_dir_detection_dedupes_multiple_paths_under_the_same_home() {
        let paths = vec![
            PathBuf::from("/home/jakea/models/a.gguf"),
            PathBuf::from("/home/jakea/models/b.gguf"),
        ];
        let dirs = home_dirs_needing_acl(&paths, "llama-server");
        assert_eq!(dirs.len(), 1);
    }

    #[test]
    fn home_dir_detection_handles_multiple_distinct_users() {
        let paths = vec![
            PathBuf::from("/home/alice/models/a.gguf"),
            PathBuf::from("/home/bob/models/b.gguf"),
        ];
        let dirs = home_dirs_needing_acl(&paths, "llama-server");
        assert_eq!(dirs.len(), 2);
        assert!(dirs.contains(&PathBuf::from("/home/alice")));
        assert!(dirs.contains(&PathBuf::from("/home/bob")));
    }
}
