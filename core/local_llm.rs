//! Prompt assembly and `with:` parsing for `run: local_llm` workflow steps.
//!
//! Deterministic ordering: system message (if any), then the literal `prompt:` text,
//! then any upstream artifact under an explicit label. Attachments are size-capped and
//! truncation is always reported in-band (via the returned `bool`, surfaced by the
//! caller) — never silently dropped.

use crate::execution::Artifact;
use crate::model::{GenerationParameters, ModelMessage, ModelRequirements};
use serde::Deserialize;

const MAX_ATTACHMENT_CHARS: usize = 20_000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocalLlmWith {
    pub role: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub generation: GenerationParameters,
    #[serde(default)]
    pub requirements: ModelRequirements,
}

pub fn parse_with(params: &serde_yaml::Value) -> LocalLlmWith {
    params
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("with".to_string())))
        .cloned()
        .and_then(|v| serde_yaml::from_value(v).ok())
        .unwrap_or_default()
}

/// Returns the assembled messages plus whether the attached upstream artifact had to
/// be truncated to fit `MAX_ATTACHMENT_CHARS`.
pub fn assemble_prompt(
    with: &LocalLlmWith,
    injected_input: Option<&str>,
) -> (Vec<ModelMessage>, bool) {
    let mut messages = Vec::new();
    if let Some(system) = &with.system {
        messages.push(ModelMessage::system(system.clone()));
    }

    let mut user_content = String::new();
    if let Some(prompt) = &with.prompt {
        user_content.push_str(prompt);
    }

    let mut truncated = false;
    if let Some(input) = injected_input {
        if !user_content.is_empty() {
            user_content.push_str("\n\n");
        }
        user_content.push_str("--- Attached artifact (upstream step output) ---\n");
        let char_count = input.chars().count();
        if char_count > MAX_ATTACHMENT_CHARS {
            let clipped: String = input.chars().take(MAX_ATTACHMENT_CHARS).collect();
            user_content.push_str(&clipped);
            user_content.push_str(&format!(
                "\n--- [truncated: {} of {} characters shown] ---",
                MAX_ATTACHMENT_CHARS, char_count
            ));
            truncated = true;
        } else {
            user_content.push_str(input);
        }
    }

    messages.push(ModelMessage::user(user_content));
    (messages, truncated)
}

/// Bridges a typed `Artifact` response back into the legacy plain-string
/// `outputs`/`StepLog.output` shape every other step type already produces: text and
/// JSON are preserved faithfully, everything else gets a labeled debug fallback rather
/// than silently losing information.
pub fn artifact_to_text(artifact: &Artifact) -> String {
    match artifact {
        Artifact::Text(s) => s.clone(),
        Artifact::Json(v) => serde_json::to_string(v).unwrap_or_default(),
        Artifact::Null => String::new(),
        other => format!("{:?}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessageRole;

    #[test]
    fn assembles_system_then_prompt_then_labeled_attachment_in_order() {
        let with = LocalLlmWith {
            system: Some("be terse".to_string()),
            prompt: Some("Analyze this:".to_string()),
            ..Default::default()
        };
        let (messages, truncated) = assemble_prompt(&with, Some("upstream text"));
        assert!(!truncated);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::System);
        assert!(messages[1].content.starts_with("Analyze this:"));
        assert!(messages[1].content.contains("upstream text"));
    }

    #[test]
    fn oversized_attachment_is_truncated_with_an_explicit_marker() {
        let with = LocalLlmWith::default();
        let huge = "x".repeat(MAX_ATTACHMENT_CHARS + 500);
        let (messages, truncated) = assemble_prompt(&with, Some(&huge));
        assert!(truncated);
        assert!(messages[0].content.contains("truncated"));
    }

    #[test]
    fn no_attachment_and_no_prompt_yields_empty_user_message_not_a_panic() {
        let with = LocalLlmWith::default();
        let (messages, truncated) = assemble_prompt(&with, None);
        assert!(!truncated);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "");
    }

    #[test]
    fn artifact_to_text_preserves_json_faithfully() {
        let a = Artifact::Json(serde_json::json!({"a": 1}));
        assert_eq!(artifact_to_text(&a), "{\"a\":1}");
    }

    #[test]
    fn artifact_to_text_preserves_text_verbatim() {
        let a = Artifact::Text("hello world".to_string());
        assert_eq!(artifact_to_text(&a), "hello world");
    }

    #[test]
    fn parse_with_defaults_when_absent() {
        let with = parse_with(&serde_yaml::Value::Null);
        assert!(with.role.is_none());
    }

    #[test]
    fn parse_with_reads_role_and_generation() {
        let yaml = "with:\n  role: reasoning\n  prompt: hi\n  generation:\n    max_tokens: 100\n";
        let params: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let with = parse_with(&params);
        assert_eq!(with.role.as_deref(), Some("reasoning"));
        assert_eq!(with.generation.max_tokens, Some(100));
    }
}
