//! The `CodeIntelligenceProvider` abstraction: read-only structural code queries backed
//! by an external tool. No implementation lives here — this is the contract PR4's
//! `CodebaseMemoryCliProvider` implements by spawning `codebase-memory-mcp` directly.
//!
//! Synchronous by design: nothing else in this workspace uses an async runtime, and a
//! provider call is a blocking subprocess invocation either way (see PR4).

use crate::code_intelligence::error::ProviderError;
use crate::code_intelligence::operations::GraphOperation;
use crate::execution::CodeGraphArtifact;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderMetadata {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderHealth {
    pub available: bool,
    pub detail: String,
}

/// Implementations must treat the underlying code graph as read-only derived state:
/// never mutate it, never silently trigger a full re-index, and only execute operations
/// from the `GraphOperation` allowlist.
pub trait CodeIntelligenceProvider {
    fn health(&self) -> Result<ProviderHealth, ProviderError>;

    fn query(
        &self,
        operation: GraphOperation,
        args: serde_json::Value,
    ) -> Result<CodeGraphArtifact, ProviderError>;

    fn metadata(&self) -> ProviderMetadata;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct FakeProvider {
        available: bool,
    }

    impl CodeIntelligenceProvider for FakeProvider {
        fn health(&self) -> Result<ProviderHealth, ProviderError> {
            Ok(ProviderHealth {
                available: self.available,
                detail: if self.available {
                    "ok".to_string()
                } else {
                    "not installed".to_string()
                },
            })
        }

        fn query(
            &self,
            operation: GraphOperation,
            _args: serde_json::Value,
        ) -> Result<CodeGraphArtifact, ProviderError> {
            if !self.available {
                return Err(ProviderError::NotFound("fake provider".to_string()));
            }
            Ok(CodeGraphArtifact {
                provider: "fake".to_string(),
                provider_version: Some("0.0.0".to_string()),
                repo_root: PathBuf::from("/repo"),
                git_revision: Some("abc123".to_string()),
                dirty: false,
                indexed_at: None,
                operation: operation.tool_name().to_string(),
                payload: serde_json::json!({"ok": true}),
            })
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "fake".to_string(),
                version: Some("0.0.0".to_string()),
            }
        }
    }

    #[test]
    fn fake_provider_is_usable_as_a_trait_object() {
        let provider: Box<dyn CodeIntelligenceProvider> =
            Box::new(FakeProvider { available: true });
        assert!(provider.health().unwrap().available);
        let artifact = provider
            .query(GraphOperation::SearchGraph, serde_json::json!({}))
            .unwrap();
        assert_eq!(artifact.operation, "search_graph");
        assert_eq!(artifact.provider, "fake");
        assert_eq!(provider.metadata().name, "fake");
    }

    #[test]
    fn unavailable_provider_reports_unhealthy_and_fails_queries() {
        let provider: Box<dyn CodeIntelligenceProvider> =
            Box::new(FakeProvider { available: false });
        assert!(!provider.health().unwrap().available);
        let err = provider
            .query(GraphOperation::IndexStatus, serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }
}
