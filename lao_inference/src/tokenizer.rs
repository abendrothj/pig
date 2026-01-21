//! Tokenizer utilities

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Chat message format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Chat role
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// Chat template format
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatTemplate {
    /// Llama 2 style: [INST] <<SYS>> system <</SYS>> user [/INST]
    Llama2,
    /// Llama 3 style: <|begin_of_text|><|start_header_id|>system<|end_header_id|>
    Llama3,
    /// Mistral style: [INST] user [/INST]
    Mistral,
    /// ChatML style: <|im_start|>system\ncontent<|im_end|>
    ChatML,
    /// Phi style: <|user|>\ncontent<|end|>
    Phi,
    /// Alpaca style: ### Instruction:\n### Response:
    Alpaca,
    /// Vicuna style: USER: content ASSISTANT:
    Vicuna,
    /// Simple: Just concatenate messages
    Simple,
}

impl ChatTemplate {
    /// Format messages using this template
    pub fn format(&self, messages: &[ChatMessage]) -> String {
        match self {
            Self::Llama2 => format_llama2(messages),
            Self::Llama3 => format_llama3(messages),
            Self::Mistral => format_mistral(messages),
            Self::ChatML => format_chatml(messages),
            Self::Phi => format_phi(messages),
            Self::Alpaca => format_alpaca(messages),
            Self::Vicuna => format_vicuna(messages),
            Self::Simple => format_simple(messages),
        }
    }
}

fn format_llama2(messages: &[ChatMessage]) -> String {
    let mut result = String::new();
    let mut system_msg = None;

    for msg in messages {
        match msg.role {
            ChatRole::System => {
                system_msg = Some(&msg.content);
            }
            ChatRole::User => {
                result.push_str("<s>[INST] ");
                if let Some(sys) = system_msg.take() {
                    result.push_str("<<SYS>>\n");
                    result.push_str(sys);
                    result.push_str("\n<</SYS>>\n\n");
                }
                result.push_str(&msg.content);
                result.push_str(" [/INST]");
            }
            ChatRole::Assistant => {
                result.push(' ');
                result.push_str(&msg.content);
                result.push_str(" </s>");
            }
        }
    }

    result
}

fn format_llama3(messages: &[ChatMessage]) -> String {
    let mut result = String::from("<|begin_of_text|>");

    for msg in messages {
        let role = match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };

        result.push_str(&format!(
            "<|start_header_id|>{}<|end_header_id|>\n\n{}<|eot_id|>",
            role, msg.content
        ));
    }

    // Add assistant header for generation
    result.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");

    result
}

fn format_mistral(messages: &[ChatMessage]) -> String {
    let mut result = String::new();

    for msg in messages {
        match msg.role {
            ChatRole::System | ChatRole::User => {
                result.push_str("<s>[INST] ");
                result.push_str(&msg.content);
                result.push_str(" [/INST]");
            }
            ChatRole::Assistant => {
                result.push_str(&msg.content);
                result.push_str("</s>");
            }
        }
    }

    result
}

fn format_chatml(messages: &[ChatMessage]) -> String {
    let mut result = String::new();

    for msg in messages {
        let role = match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };

        result.push_str(&format!(
            "<|im_start|>{}\n{}<|im_end|>\n",
            role, msg.content
        ));
    }

    result.push_str("<|im_start|>assistant\n");

    result
}

fn format_phi(messages: &[ChatMessage]) -> String {
    let mut result = String::new();

    for msg in messages {
        match msg.role {
            ChatRole::System => {
                result.push_str("<|system|>\n");
                result.push_str(&msg.content);
                result.push_str("<|end|>\n");
            }
            ChatRole::User => {
                result.push_str("<|user|>\n");
                result.push_str(&msg.content);
                result.push_str("<|end|>\n");
            }
            ChatRole::Assistant => {
                result.push_str("<|assistant|>\n");
                result.push_str(&msg.content);
                result.push_str("<|end|>\n");
            }
        }
    }

    result.push_str("<|assistant|>\n");

    result
}

fn format_alpaca(messages: &[ChatMessage]) -> String {
    let mut result = String::new();

    for msg in messages {
        match msg.role {
            ChatRole::System => {
                result.push_str(&msg.content);
                result.push_str("\n\n");
            }
            ChatRole::User => {
                result.push_str("### Instruction:\n");
                result.push_str(&msg.content);
                result.push_str("\n\n");
            }
            ChatRole::Assistant => {
                result.push_str("### Response:\n");
                result.push_str(&msg.content);
                result.push_str("\n\n");
            }
        }
    }

    result.push_str("### Response:\n");

    result
}

fn format_vicuna(messages: &[ChatMessage]) -> String {
    let mut result = String::new();

    for msg in messages {
        match msg.role {
            ChatRole::System => {
                result.push_str(&msg.content);
                result.push_str("\n\n");
            }
            ChatRole::User => {
                result.push_str("USER: ");
                result.push_str(&msg.content);
                result.push('\n');
            }
            ChatRole::Assistant => {
                result.push_str("ASSISTANT: ");
                result.push_str(&msg.content);
                result.push_str("</s>\n");
            }
        }
    }

    result.push_str("ASSISTANT:");

    result
}

fn format_simple(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse a raw prompt into chat messages
pub fn parse_prompt(prompt: &str, template: ChatTemplate) -> Vec<ChatMessage> {
    // Simple heuristic parsing based on common patterns
    let mut messages = Vec::new();

    if prompt.contains("[INST]") {
        // Llama/Mistral style
        let parts: Vec<&str> = prompt.split("[INST]").collect();
        for part in parts {
            if part.contains("<</SYS>>") {
                // Extract system message
                if let Some(start) = part.find("<<SYS>>") {
                    if let Some(end) = part.find("<</SYS>>") {
                        let system = part[start + 7..end].trim();
                        messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: system.to_string(),
                        });
                    }
                }
            }

            if let Some(end) = part.find("[/INST]") {
                let user = part[..end].trim();
                if !user.is_empty() && !user.contains("<<SYS>>") {
                    messages.push(ChatMessage {
                        role: ChatRole::User,
                        content: user.to_string(),
                    });
                }
            }
        }
    } else {
        // Treat as single user message
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
        });
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chatml_format() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: "You are helpful.".to_string(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: "Hello!".to_string(),
            },
        ];

        let formatted = ChatTemplate::ChatML.format(&messages);
        assert!(formatted.contains("<|im_start|>system"));
        assert!(formatted.contains("You are helpful."));
        assert!(formatted.contains("<|im_start|>user"));
    }

    #[test]
    fn test_llama3_format() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::User,
                content: "Hello!".to_string(),
            },
        ];

        let formatted = ChatTemplate::Llama3.format(&messages);
        assert!(formatted.contains("<|begin_of_text|>"));
        assert!(formatted.contains("<|start_header_id|>user"));
    }
}
