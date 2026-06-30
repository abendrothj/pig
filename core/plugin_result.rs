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
