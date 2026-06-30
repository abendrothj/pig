/// Structured plugin execution result (host-side; ABI v1 still uses text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRunStatus {
    Success,
    ValidationFailed,
    RuntimeError,
    EmptyOutput,
    ErrorOutput,
}

#[derive(Debug, Clone)]
pub struct PluginRunResult {
    pub status: PluginRunStatus,
    pub output: Option<String>,
    pub error: Option<String>,
}

impl PluginRunResult {
    pub fn success(output: String) -> Self {
        Self {
            status: PluginRunStatus::Success,
            output: Some(output),
            error: None,
        }
    }

    pub fn validation_failed(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            status: PluginRunStatus::ValidationFailed,
            output: None,
            error: Some(msg),
        }
    }

    pub fn runtime_error(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            status: PluginRunStatus::RuntimeError,
            output: None,
            error: Some(msg),
        }
    }

    /// Map an ABI v2 structured status code + optional text into a host result.
    pub fn from_status_code(status: u32, text: Option<String>, plugin_name: &str) -> Self {
        use lao_plugin_api::{
            LAO_STATUS_RUNTIME_ERROR, LAO_STATUS_SUCCESS, LAO_STATUS_VALIDATION_FAILED,
        };
        match status {
            LAO_STATUS_SUCCESS => match text {
                Some(t) if !t.trim().is_empty() => Self::success(t),
                _ => Self {
                    status: PluginRunStatus::EmptyOutput,
                    output: None,
                    error: Some(format!("plugin '{}' returned empty output", plugin_name)),
                },
            },
            LAO_STATUS_VALIDATION_FAILED => Self::validation_failed(
                text.unwrap_or_else(|| format!("plugin '{}' rejected input", plugin_name)),
            ),
            LAO_STATUS_RUNTIME_ERROR => {
                Self::runtime_error(text.unwrap_or_else(|| {
                    format!("plugin '{}' reported a runtime error", plugin_name)
                }))
            }
            other => Self::runtime_error(format!(
                "plugin '{}' returned unknown status code {}",
                plugin_name, other
            )),
        }
    }

    pub fn from_plugin_text(output: String) -> Self {
        if output.trim().is_empty() {
            return Self {
                status: PluginRunStatus::EmptyOutput,
                output: None,
                error: Some("plugin returned empty output".to_string()),
            };
        }
        if output.trim().to_lowercase().starts_with("error:") {
            return Self {
                status: PluginRunStatus::ErrorOutput,
                output: None,
                error: Some(output),
            };
        }
        Self::success(output)
    }

    pub fn is_success(&self) -> bool {
        self.status == PluginRunStatus::Success
    }

    pub fn display_error(&self) -> String {
        self.error
            .clone()
            .or_else(|| self.output.clone())
            .unwrap_or_else(|| "unknown plugin error".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_carries_output() {
        let r = PluginRunResult::success("hello".to_string());
        assert!(r.is_success());
        assert_eq!(r.output.as_deref(), Some("hello"));
        assert!(r.error.is_none());
    }

    #[test]
    fn from_text_empty_is_empty_output() {
        let r = PluginRunResult::from_plugin_text("   ".to_string());
        assert_eq!(r.status, PluginRunStatus::EmptyOutput);
        assert!(!r.is_success());
    }

    #[test]
    fn from_text_error_prefix_is_error_output() {
        let r = PluginRunResult::from_plugin_text("error: boom".to_string());
        assert_eq!(r.status, PluginRunStatus::ErrorOutput);
        assert!(!r.is_success());
        assert_eq!(r.display_error(), "error: boom");
    }

    #[test]
    fn from_text_error_prefix_is_case_insensitive() {
        let r = PluginRunResult::from_plugin_text("ERROR: nope".to_string());
        assert_eq!(r.status, PluginRunStatus::ErrorOutput);
    }

    #[test]
    fn from_text_normal_is_success() {
        let r = PluginRunResult::from_plugin_text("real output".to_string());
        assert!(r.is_success());
        assert_eq!(r.output.as_deref(), Some("real output"));
    }

    #[test]
    fn validation_and_runtime_errors_surface_messages() {
        let v = PluginRunResult::validation_failed("bad input");
        assert_eq!(v.status, PluginRunStatus::ValidationFailed);
        assert_eq!(v.display_error(), "bad input");

        let rt = PluginRunResult::runtime_error("crashed");
        assert_eq!(rt.status, PluginRunStatus::RuntimeError);
        assert_eq!(rt.display_error(), "crashed");
    }

    #[test]
    fn from_status_code_maps_abi_v2_statuses() {
        use lao_plugin_api::{
            LAO_STATUS_RUNTIME_ERROR, LAO_STATUS_SUCCESS, LAO_STATUS_VALIDATION_FAILED,
        };

        let ok = PluginRunResult::from_status_code(
            LAO_STATUS_SUCCESS,
            Some("done".to_string()),
            "EchoPlugin",
        );
        assert!(ok.is_success());
        assert_eq!(ok.output.as_deref(), Some("done"));

        let empty = PluginRunResult::from_status_code(LAO_STATUS_SUCCESS, None, "EchoPlugin");
        assert_eq!(empty.status, PluginRunStatus::EmptyOutput);

        let bad = PluginRunResult::from_status_code(
            LAO_STATUS_VALIDATION_FAILED,
            Some("nope".to_string()),
            "EchoPlugin",
        );
        assert_eq!(bad.status, PluginRunStatus::ValidationFailed);
        assert_eq!(bad.display_error(), "nope");

        let rt = PluginRunResult::from_status_code(LAO_STATUS_RUNTIME_ERROR, None, "EchoPlugin");
        assert_eq!(rt.status, PluginRunStatus::RuntimeError);
        assert!(rt.display_error().contains("EchoPlugin"));

        let unknown = PluginRunResult::from_status_code(999, None, "EchoPlugin");
        assert_eq!(unknown.status, PluginRunStatus::RuntimeError);
        assert!(unknown.display_error().contains("999"));
    }

    #[test]
    fn display_error_falls_back_to_output_then_default() {
        let only_output = PluginRunResult {
            status: PluginRunStatus::ErrorOutput,
            output: Some("partial".to_string()),
            error: None,
        };
        assert_eq!(only_output.display_error(), "partial");

        let nothing = PluginRunResult {
            status: PluginRunStatus::RuntimeError,
            output: None,
            error: None,
        };
        assert_eq!(nothing.display_error(), "unknown plugin error");
    }
}
