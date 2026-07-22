//! Quarantines the ABI v1/v2 legacy outcome conventions behind the structured model.
//!
//! `PluginRunResult` (`crate::plugin_result`) already isolates the two conventions this
//! host has ever used to decide whether a plugin call succeeded:
//! - ABI v1 text: empty output, or output beginning with `error:` (case-insensitive),
//!   is a failure (`PluginRunResult::from_plugin_text`).
//! - ABI v2 status codes: an explicit status, with empty output on a "success" status
//!   still downgraded to a failure (`PluginRunResult::from_status_code`).
//!
//! `adapt` is the single place that turns that already-decided outcome into the
//! structured `StepResult` model. Nothing outside this module (and `plugin_result.rs`
//! itself) should ever branch on "output is empty" or "output starts with error:" again.

use crate::execution::artifact::Artifact;
use crate::execution::result::{StepError, StepErrorKind, StepMetadata, StepResult, StepStatus};
use crate::plugin_result::{PluginRunResult, PluginRunStatus};
use std::collections::BTreeMap;

pub fn adapt(result: PluginRunResult, metadata: StepMetadata) -> StepResult {
    match result.status {
        PluginRunStatus::Success => {
            let mut outputs = BTreeMap::new();
            outputs.insert(
                "output".to_string(),
                Artifact::Text(result.output.unwrap_or_default()),
            );
            StepResult {
                status: StepStatus::Success,
                outputs,
                error: None,
                metadata,
            }
        }
        PluginRunStatus::ValidationFailed
        | PluginRunStatus::RuntimeError
        | PluginRunStatus::EmptyOutput
        | PluginRunStatus::ErrorOutput => {
            let kind = match result.status {
                PluginRunStatus::ValidationFailed => StepErrorKind::ValidationFailed,
                PluginRunStatus::RuntimeError => StepErrorKind::RuntimeError,
                PluginRunStatus::EmptyOutput => StepErrorKind::EmptyOutput,
                PluginRunStatus::ErrorOutput => StepErrorKind::ErrorOutput,
                PluginRunStatus::Success => unreachable!("handled above"),
            };
            let message = result
                .error
                .or(result.output)
                .unwrap_or_else(|| "unknown plugin error".to_string());
            StepResult {
                status: StepStatus::Failed,
                outputs: BTreeMap::new(),
                error: Some(StepError { kind, message }),
                metadata,
            }
        }
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
            duration_ms: 0,
            cache_hit: false,
        }
    }

    #[test]
    fn success_maps_to_success_status_and_text_artifact() {
        let r = adapt(PluginRunResult::success("hello".to_string()), meta());
        assert!(r.is_success());
        assert_eq!(r.primary_output_text().as_deref(), Some("hello"));
        assert!(r.error.is_none());
    }

    #[test]
    fn validation_failed_maps_to_validation_failed_kind() {
        let r = adapt(PluginRunResult::validation_failed("bad input"), meta());
        assert!(!r.is_success());
        assert_eq!(
            r.error.as_ref().unwrap().kind,
            StepErrorKind::ValidationFailed
        );
        assert_eq!(r.display_error(), "bad input");
    }

    #[test]
    fn runtime_error_maps_to_runtime_error_kind() {
        let r = adapt(PluginRunResult::runtime_error("crashed"), meta());
        assert_eq!(r.error.as_ref().unwrap().kind, StepErrorKind::RuntimeError);
        assert_eq!(r.display_error(), "crashed");
    }

    #[test]
    fn empty_output_text_maps_to_empty_output_kind() {
        let r = adapt(PluginRunResult::from_plugin_text("   ".to_string()), meta());
        assert_eq!(r.error.as_ref().unwrap().kind, StepErrorKind::EmptyOutput);
    }

    #[test]
    fn error_prefixed_text_maps_to_error_output_kind() {
        let r = adapt(
            PluginRunResult::from_plugin_text("error: nope".to_string()),
            meta(),
        );
        assert_eq!(r.error.as_ref().unwrap().kind, StepErrorKind::ErrorOutput);
        assert_eq!(r.display_error(), "error: nope");
    }

    #[test]
    fn abi_v2_empty_success_status_maps_to_empty_output_kind() {
        let r = adapt(
            PluginRunResult::from_status_code(
                lao_plugin_api::LAO_STATUS_SUCCESS,
                None,
                "EchoPlugin",
            ),
            meta(),
        );
        assert_eq!(r.error.as_ref().unwrap().kind, StepErrorKind::EmptyOutput);
    }

    #[test]
    fn metadata_is_carried_through_unchanged() {
        let m = StepMetadata {
            plugin_name: "WhisperPlugin".to_string(),
            plugin_version: Some("1.2.3".to_string()),
            attempt: 3,
            duration_ms: 42,
            cache_hit: false,
        };
        let r = adapt(PluginRunResult::success("ok".to_string()), m.clone());
        assert_eq!(r.metadata, m);
    }
}
