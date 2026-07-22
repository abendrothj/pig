use std::fmt;

/// Failure modes for a `CodeIntelligenceProvider` call. Distinguishes provider-process
/// failures (spawn, timeout, exit status) from provider-output failures (malformed
/// JSON/UTF-8) from host-side policy failures (denied capability, unsupported operation).
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    NotFound(String),
    SpawnFailed(String),
    Timeout,
    NonZeroExit { code: Option<i32>, stderr: String },
    MalformedOutput(String),
    OutputTooLarge,
    UnsupportedOperation(String),
    Denied(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::NotFound(msg) => write!(f, "provider executable not found: {}", msg),
            ProviderError::SpawnFailed(msg) => {
                write!(f, "failed to spawn provider process: {}", msg)
            }
            ProviderError::Timeout => write!(f, "provider query timed out"),
            ProviderError::NonZeroExit { code, stderr } => {
                write!(
                    f,
                    "provider process exited with status {:?}: {}",
                    code, stderr
                )
            }
            ProviderError::MalformedOutput(msg) => {
                write!(f, "provider returned malformed output: {}", msg)
            }
            ProviderError::OutputTooLarge => {
                write!(f, "provider output exceeded the configured size limit")
            }
            ProviderError::UnsupportedOperation(op) => write!(
                f,
                "operation '{}' is not in the code-graph operation allowlist",
                op
            ),
            ProviderError::Denied(msg) => write!(f, "code-graph access denied: {}", msg),
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_variants_render_readable_messages() {
        assert!(ProviderError::NotFound("x".into())
            .to_string()
            .contains("not found"));
        assert!(ProviderError::Timeout.to_string().contains("timed out"));
        assert!(ProviderError::OutputTooLarge
            .to_string()
            .contains("size limit"));
        assert!(
            ProviderError::UnsupportedOperation("index_repository".into())
                .to_string()
                .contains("index_repository")
        );
    }
}
