//! Explicit per-step execution result types.
//!
//! Distinct from `crate::workflow_state::{StepResult, StepStatus}`, which are a
//! persisted, post-hoc *checkpoint* record built once a workflow finishes. These types
//! are the live, in-run outcome of a single step invocation, threaded through
//! `StepExecutor` at the point the plugin's outcome is decided.

use crate::execution::artifact::Artifact;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepStatus {
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepErrorKind {
    /// Plugin rejected its input (ABI v2 `LAO_STATUS_VALIDATION_FAILED`, or ABI v1's
    /// `from_plugin_text` equivalent).
    ValidationFailed,
    /// Plugin reported a runtime failure.
    RuntimeError,
    /// Plugin reported success but produced empty/whitespace-only output.
    EmptyOutput,
    /// ABI v1 legacy convention: output text began with `error:`.
    ErrorOutput,
    /// Host-level: trust policy denied the step before the plugin was invoked.
    TrustDenied,
    /// Host-level: no plugin registered under the step's `run` name.
    PluginNotFound,
    /// Host-level: reading or writing the on-disk step cache failed.
    CacheError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepError {
    pub kind: StepErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepMetadata {
    pub plugin_name: String,
    pub plugin_version: Option<String>,
    pub attempt: u32,
    pub duration_ms: u64,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepResult {
    pub status: StepStatus,
    pub outputs: BTreeMap<String, Artifact>,
    pub error: Option<StepError>,
    pub metadata: StepMetadata,
}

impl StepResult {
    pub fn is_success(&self) -> bool {
        self.status == StepStatus::Success
    }

    /// The legacy single-output-string view: the `"output"` key's `Artifact::Text`,
    /// if present. Used to bridge into `StepLog`/`outputs: HashMap<String,String>`
    /// without widening those types in this PR.
    pub fn primary_output_text(&self) -> Option<String> {
        match self.outputs.get("output") {
            Some(Artifact::Text(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn display_error(&self) -> String {
        self.error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "unknown plugin error".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> StepMetadata {
        StepMetadata {
            plugin_name: "EchoPlugin".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            attempt: 1,
            duration_ms: 5,
            cache_hit: false,
        }
    }

    #[test]
    fn success_result_reports_success_and_output() {
        let mut outputs = BTreeMap::new();
        outputs.insert("output".to_string(), Artifact::Text("hi".to_string()));
        let r = StepResult {
            status: StepStatus::Success,
            outputs,
            error: None,
            metadata: meta(),
        };
        assert!(r.is_success());
        assert_eq!(r.primary_output_text().as_deref(), Some("hi"));
    }

    #[test]
    fn failed_result_reports_display_error() {
        let r = StepResult {
            status: StepStatus::Failed,
            outputs: BTreeMap::new(),
            error: Some(StepError {
                kind: StepErrorKind::RuntimeError,
                message: "boom".to_string(),
            }),
            metadata: meta(),
        };
        assert!(!r.is_success());
        assert_eq!(r.display_error(), "boom");
        assert_eq!(r.primary_output_text(), None);
    }

    #[test]
    fn failed_result_without_error_falls_back_to_default_message() {
        let r = StepResult {
            status: StepStatus::Failed,
            outputs: BTreeMap::new(),
            error: None,
            metadata: meta(),
        };
        assert_eq!(r.display_error(), "unknown plugin error");
    }
}
