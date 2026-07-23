//! Worker configuration: `[worker]` (identity, binding, concurrency, auth, backend
//! selection) plus the shared `[models.*]` registry format (see
//! `pig_core::model::registry`) read from the same file.

use serde::Deserialize;
use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "worker config: {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

fn err(message: impl Into<String>) -> ConfigError {
    ConfigError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Name of an environment variable holding the bearer token. Never logged; the
    /// token itself never appears in config files by design.
    pub token_env: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_server_executable() -> String {
    "llama-server".to_string()
}

fn default_mlx_server_executable() -> String {
    "mlx_lm.server".to_string()
}

fn default_startup_timeout() -> u64 {
    60
}

fn default_request_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlamaCppRuntimeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_server_executable")]
    pub server_executable: String,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
}

impl Default for LlamaCppRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            server_executable: default_server_executable(),
            startup_timeout_seconds: default_startup_timeout(),
            request_timeout_seconds: default_request_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MlxRuntimeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mlx_server_executable")]
    pub server_executable: String,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
}

impl Default for MlxRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_executable: default_mlx_server_executable(),
            startup_timeout_seconds: default_startup_timeout(),
            request_timeout_seconds: default_request_timeout(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub llama_cpp: LlamaCppRuntimeConfig,
    #[serde(default)]
    pub mlx: MlxRuntimeConfig,
}

fn default_bind() -> String {
    "127.0.0.1:9847".to_string()
}

fn default_max_concurrent_jobs() -> usize {
    1
}

fn default_max_queued_jobs() -> usize {
    16
}

fn default_shutdown_grace_seconds() -> u64 {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub id: String,
    /// Optional operator-declared owner of this worker pool. This is advisory only:
    /// the worker remains safe under multiple coordinators because admission is
    /// atomic locally, but status can make accidental competing control planes
    /// visible before they waste routing decisions.
    #[serde(default)]
    pub coordinator_id: Option<String>,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
    #[serde(default = "default_max_queued_jobs")]
    pub max_queued_jobs: usize,
    #[serde(default = "default_shutdown_grace_seconds")]
    pub shutdown_grace_seconds: u64,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
}

impl WorkerConfig {
    pub fn bind_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.bind
            .parse()
            .map_err(|e| err(format!("invalid bind address '{}': {}", self.bind, e)))
    }

    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_seconds)
    }

    pub fn llama_cpp_startup_timeout(&self) -> Duration {
        Duration::from_secs(self.runtime.llama_cpp.startup_timeout_seconds)
    }

    pub fn llama_cpp_request_timeout(&self) -> Duration {
        Duration::from_secs(self.runtime.llama_cpp.request_timeout_seconds)
    }

    /// Security defaults: binding to a non-loopback address is only allowed when auth
    /// is explicitly enabled, so a worker can never be exposed off-host with no
    /// authentication by a config mistake alone.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let addr = self.bind_addr()?;
        if !addr.ip().is_loopback() && !self.auth.enabled {
            return Err(err(format!(
                "refusing to bind non-loopback address '{}' without worker.auth.enabled = true",
                self.bind
            )));
        }
        if self.auth.enabled && self.auth.token_env.is_none() {
            return Err(err(
                "worker.auth.enabled = true requires worker.auth.token_env to name an environment variable",
            ));
        }
        if self.max_concurrent_jobs == 0 {
            return Err(err("worker.max_concurrent_jobs must be at least 1"));
        }
        Ok(())
    }

    pub fn resolve_auth_token(&self) -> Result<Option<String>, ConfigError> {
        if !self.auth.enabled {
            return Ok(None);
        }
        let var = self
            .auth
            .token_env
            .as_ref()
            .ok_or_else(|| err("worker.auth.token_env is not set"))?;
        std::env::var(var)
            .map(Some)
            .map_err(|_| err(format!("environment variable '{}' is not set", var)))
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| err(format!("failed to read {}: {}", path.display(), e)))?;
        Self::from_toml_str(&text)
    }

    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        struct Root {
            worker: WorkerConfig,
        }
        let root: Root = toml::from_str(text).map_err(|e| err(format!("invalid TOML: {}", e)))?;
        root.worker.validate()?;
        Ok(root.worker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[worker]
id = "macbook-worker"
coordinator_id = "spectre-coordinator"
bind = "127.0.0.1:9847"
max_concurrent_jobs = 1
max_queued_jobs = 16
shutdown_grace_seconds = 20

[worker.auth]
enabled = false

[worker.runtime.llama_cpp]
enabled = true
server_executable = "llama-server"
startup_timeout_seconds = 60
request_timeout_seconds = 600
"#;

    #[test]
    fn parses_the_example_config() {
        let cfg = WorkerConfig::from_toml_str(EXAMPLE).unwrap();
        assert_eq!(cfg.id, "macbook-worker");
        assert_eq!(cfg.coordinator_id.as_deref(), Some("spectre-coordinator"));
        assert_eq!(cfg.max_concurrent_jobs, 1);
        assert!(cfg.runtime.llama_cpp.enabled);
        assert_eq!(cfg.runtime.llama_cpp.server_executable, "llama-server");
    }

    #[test]
    fn defaults_apply_when_sections_are_omitted() {
        let cfg = WorkerConfig::from_toml_str("[worker]\nid = \"w\"\n").unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:9847");
        assert_eq!(cfg.max_concurrent_jobs, 1);
        assert_eq!(cfg.max_queued_jobs, 16);
        assert!(!cfg.auth.enabled);
        assert_eq!(cfg.coordinator_id, None);
    }

    #[test]
    fn non_loopback_bind_without_auth_is_rejected() {
        let toml = "[worker]\nid = \"w\"\nbind = \"0.0.0.0:9847\"\n";
        let err = WorkerConfig::from_toml_str(toml).unwrap_err();
        assert!(err.message.contains("non-loopback"));
    }

    #[test]
    fn non_loopback_bind_with_auth_enabled_is_accepted() {
        let toml = "[worker]\nid = \"w\"\nbind = \"0.0.0.0:9847\"\n[worker.auth]\nenabled = true\ntoken_env = \"PIG_TOKEN\"\n";
        assert!(WorkerConfig::from_toml_str(toml).is_ok());
    }

    #[test]
    fn auth_enabled_without_token_env_is_rejected() {
        let toml = "[worker]\nid = \"w\"\n[worker.auth]\nenabled = true\n";
        let err = WorkerConfig::from_toml_str(toml).unwrap_err();
        assert!(err.message.contains("token_env"));
    }

    #[test]
    fn zero_concurrency_is_rejected() {
        let toml = "[worker]\nid = \"w\"\nmax_concurrent_jobs = 0\n";
        assert!(WorkerConfig::from_toml_str(toml).is_err());
    }

    #[test]
    fn resolve_auth_token_reads_named_env_var() {
        std::env::set_var("PIG_TEST_WORKER_TOKEN_XYZ", "secret123");
        let toml = "[worker]\nid = \"w\"\nbind = \"127.0.0.1:9847\"\n[worker.auth]\nenabled = true\ntoken_env = \"PIG_TEST_WORKER_TOKEN_XYZ\"\n";
        let cfg = WorkerConfig::from_toml_str(toml).unwrap();
        assert_eq!(
            cfg.resolve_auth_token().unwrap().as_deref(),
            Some("secret123")
        );
        std::env::remove_var("PIG_TEST_WORKER_TOKEN_XYZ");
    }
}
