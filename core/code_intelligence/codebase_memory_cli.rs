//! `CodebaseMemoryCliProvider`: a `CodeIntelligenceProvider` backed by directly spawning
//! the `codebase-memory-mcp` executable's `cli <tool>` subcommand — never its MCP
//! (stdio JSON-RPC) server mode, and never a shell.
//!
//! Verified contract (`codebase-memory-mcp cli --help`): `codebase-memory-mcp cli
//! <tool> [json_args]` reads tool arguments from stdin as JSON, writes only the tool's
//! JSON result to stdout, and sends all logs/diagnostics to stderr with a non-zero exit
//! on failure. This lets the host tell "the provider process failed" apart from "the
//! provider reported malformed output" apart from "the tool call itself failed."

use crate::code_intelligence::error::ProviderError;
use crate::code_intelligence::operations::GraphOperation;
use crate::code_intelligence::provider::{
    CodeIntelligenceProvider, ProviderHealth, ProviderMetadata,
};
use crate::execution::CodeGraphArtifact;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub struct CodebaseMemoryCliProvider {
    binary_path: PathBuf,
    repo_root: PathBuf,
    timeout: Duration,
    max_output_bytes: usize,
    version: Option<String>,
}

impl CodebaseMemoryCliProvider {
    /// Resolve the binary via `LAO_CODEBASE_MEMORY_MCP_PATH` or `PATH`, with default
    /// timeout/output limits.
    pub fn discover(repo_root: PathBuf) -> Result<Self, ProviderError> {
        Self::new(repo_root, None, DEFAULT_TIMEOUT, DEFAULT_MAX_OUTPUT_BYTES)
    }

    pub fn new(
        repo_root: PathBuf,
        explicit_binary: Option<PathBuf>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self, ProviderError> {
        let binary_path = resolve_binary_path(explicit_binary)?;
        let version = resolve_version(&binary_path);
        Ok(Self {
            binary_path,
            repo_root,
            timeout,
            max_output_bytes,
            version,
        })
    }

    fn run_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let mut child = Command::new(&self.binary_path)
            .arg("cli")
            .arg(tool_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ProviderError::SpawnFailed(e.to_string()))?;

        let payload = serde_json::to_vec(args)
            .map_err(|e| ProviderError::SpawnFailed(format!("failed to encode args: {}", e)))?;
        if let Some(mut stdin) = child.stdin.take() {
            // Best-effort: if the child exits before reading stdin (e.g. unknown tool),
            // the write fails and we still proceed to read its (error) output.
            let _ = stdin.write_all(&payload);
        }

        let max_bytes = self.max_output_bytes;
        let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
        let stdout_thread = thread::spawn(move || read_capped(&mut stdout_pipe, max_bytes));
        let stderr_thread = thread::spawn(move || read_capped(&mut stderr_pipe, max_bytes));

        let status = wait_with_timeout(&mut child, self.timeout)?;

        let stdout_bytes = stdout_thread
            .join()
            .unwrap_or_else(|_| Err(ProviderError::SpawnFailed("stdout reader panicked".into())))?;
        let stderr_bytes = stderr_thread
            .join()
            .unwrap_or_else(|_| Err(ProviderError::SpawnFailed("stderr reader panicked".into())))?;

        if !status.success() {
            let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
            return Err(ProviderError::NonZeroExit {
                code: status.code(),
                stderr: stderr_text,
            });
        }

        let stdout_text = String::from_utf8(stdout_bytes).map_err(|e| {
            ProviderError::MalformedOutput(format!("stdout is not valid UTF-8: {}", e))
        })?;
        serde_json::from_str::<serde_json::Value>(&stdout_text)
            .map_err(|e| ProviderError::MalformedOutput(format!("stdout is not valid JSON: {}", e)))
    }
}

impl CodeIntelligenceProvider for CodebaseMemoryCliProvider {
    fn health(&self) -> Result<ProviderHealth, ProviderError> {
        if !self.binary_path.is_file() {
            return Ok(ProviderHealth {
                available: false,
                detail: format!("{} not found", self.binary_path.display()),
            });
        }
        Ok(ProviderHealth {
            available: true,
            detail: format!(
                "{} ({})",
                self.binary_path.display(),
                self.version.as_deref().unwrap_or("unknown version")
            ),
        })
    }

    fn query(
        &self,
        operation: GraphOperation,
        args: serde_json::Value,
    ) -> Result<CodeGraphArtifact, ProviderError> {
        let payload = self.run_tool(operation.tool_name(), &args)?;
        Ok(CodeGraphArtifact {
            provider: "codebase-memory-mcp".to_string(),
            provider_version: self.version.clone(),
            repo_root: self.repo_root.clone(),
            git_revision: git_revision(&self.repo_root),
            dirty: git_is_dirty(&self.repo_root),
            indexed_at: None,
            operation: operation.tool_name().to_string(),
            payload,
        })
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "codebase-memory-mcp".to_string(),
            version: self.version.clone(),
        }
    }
}

fn resolve_binary_path(explicit: Option<PathBuf>) -> Result<PathBuf, ProviderError> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p)
        } else {
            Err(ProviderError::NotFound(format!(
                "{} does not exist",
                p.display()
            )))
        };
    }
    if let Ok(env_path) = std::env::var("LAO_CODEBASE_MEMORY_MCP_PATH") {
        let p = PathBuf::from(&env_path);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(ProviderError::NotFound(format!(
                "LAO_CODEBASE_MEMORY_MCP_PATH={} does not exist",
                p.display()
            )))
        };
    }
    let name = if cfg!(windows) {
        "codebase-memory-mcp.exe"
    } else {
        "codebase-memory-mcp"
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(ProviderError::NotFound(format!(
        "{} not found on PATH; set LAO_CODEBASE_MEMORY_MCP_PATH",
        name
    )))
}

fn resolve_version(binary_path: &Path) -> Option<String> {
    let output = Command::new(binary_path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.trim().rsplit(' ').next().map(|s| s.to_string())
}

/// LAO determines revision/dirty state itself via a direct (non-shell) `git`
/// invocation rather than trusting the provider's self-reported git block, which
/// reflects state as of the provider's last index, not the live working tree.
pub(crate) fn git_revision(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Fail-safe: if dirty state can't be determined (git missing, not a repo), treat the
/// worktree as dirty so caching is disabled rather than assuming a clean cache is safe.
pub(crate) fn git_is_dirty(repo_root: &Path) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("status")
        .arg("--porcelain")
        .output();
    match output {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        _ => true,
    }
}

fn read_capped(reader: &mut impl Read, cap: usize) -> Result<Vec<u8>, ProviderError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| ProviderError::SpawnFailed(format!("failed to read output: {}", e)))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > cap {
            return Err(ProviderError::OutputTooLarge);
        }
    }
    Ok(buf)
}

/// Poll `try_wait` rather than block, so the child can be killed the instant the
/// timeout elapses; the child is reaped (`wait()`) after `kill()` so it never lingers
/// as a zombie process.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, ProviderError> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProviderError::Timeout);
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(ProviderError::SpawnFailed(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_binary_path_rejects_missing_explicit_path() {
        let err = resolve_binary_path(Some(PathBuf::from("/definitely/not/a/real/binary_xyz")))
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[test]
    fn resolve_binary_path_accepts_existing_explicit_path() {
        // Any file on disk works for this check; it doesn't have to be executable.
        let cargo_toml = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(resolve_binary_path(Some(cargo_toml)).is_ok());
    }

    #[test]
    fn git_revision_and_dirty_are_none_and_true_outside_a_repo() {
        let dir = std::env::temp_dir().join("lao_provider_test_not_a_repo");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(git_revision(&dir), None);
        assert!(git_is_dirty(&dir));
    }
}
