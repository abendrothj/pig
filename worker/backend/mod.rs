//! `ModelBackend`: the worker-side abstraction over an actual inference runtime.
//!
//! A backend supervises loading/unloading models and running generations. It never
//! talks HTTP itself — the worker's axum server sits on top of it. Two
//! implementations exist: `fake` (deterministic, no installed runtime required — used
//! for CI) and `llama_cpp` (supervises a real `llama-server` subprocess).

pub mod fake;
pub mod llama_cpp;
pub mod mlx;

use async_trait::async_trait;
use pig_core::model::{
    AcceleratorKind, FinishReason, GenerationParameters, ModelChunk, ModelId, ModelMessage,
    ModelToolCall, RequestId, ReasoningMode,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub enum BackendError {
    Unavailable(String),
    ModelNotFound(String),
    LoadFailed(String),
    GenerationFailed(String),
    Timeout,
    Cancelled,
    Unsupported(String),
    Internal(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Unavailable(m) => write!(f, "backend unavailable: {}", m),
            BackendError::ModelNotFound(m) => write!(f, "model not found: {}", m),
            BackendError::LoadFailed(m) => write!(f, "model load failed: {}", m),
            BackendError::GenerationFailed(m) => write!(f, "generation failed: {}", m),
            BackendError::Timeout => write!(f, "backend operation timed out"),
            BackendError::Cancelled => write!(f, "backend operation cancelled"),
            BackendError::Unsupported(m) => write!(f, "unsupported: {}", m),
            BackendError::Internal(m) => write!(f, "internal backend error: {}", m),
        }
    }
}

impl std::error::Error for BackendError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendHealth {
    pub available: bool,
    pub detail: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub backend: String,
    pub version: Option<String>,
    pub accelerators: Vec<AcceleratorKind>,
    pub supports_streaming: bool,
    /// Whether this backend can honor OpenAI-style function/tool definitions and
    /// return structured tool calls. Clients must not silently lose tool semantics.
    pub supports_tools: bool,
    pub supports_embedding: bool,
    pub supports_reranking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelAvailability {
    pub model_id: ModelId,
    pub path: Option<PathBuf>,
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadModelRequest {
    pub model_id: ModelId,
    pub path: PathBuf,
    pub context_size: Option<u32>,
    /// Backend-specific execution options (e.g. `LlamaCppExecutionConfig`), opaque to
    /// the trait itself — the concrete backend decides how to interpret it.
    #[serde(default)]
    pub execution_config: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadedModel {
    pub model_id: ModelId,
    pub context_tokens: Option<u32>,
    pub already_loaded: bool,
    pub load_ms: u64,
    pub accelerator: Option<AcceleratorKind>,
    pub cpu_threads: Option<u32>,
    pub gpu_layers: Option<i32>,
    pub batch_size: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct BackendGenerationRequest {
    pub request_id: RequestId,
    pub model_id: ModelId,
    pub messages: Vec<ModelMessage>,
    pub parameters: GenerationParameters,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendGenerationResponse {
    pub finish_reason: FinishReason,
    pub content: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub prompt_ms: u64,
    pub generation_ms: u64,
    pub prompt_tokens_per_second: Option<f64>,
    pub generation_tokens_per_second: Option<f64>,
    pub tool_calls: Vec<ModelToolCall>,
}

pub type ModelEventSender = mpsc::Sender<ModelChunk>;

/// Append Qwen3-style `/think` or `/no_think` control tokens so the model
/// enters or skips the thinking phase. Called by every backend that forwards
/// to an OpenAI-compatible server (llama.cpp, mlx_lm).
///
/// * Last message is user → append inline token to its content.
/// * Last message is tool/assistant (tool continuation) → inject via system
///   message so we don't retroactively alter a historical user turn.
pub(super) fn apply_reasoning_mode(messages: &mut Vec<serde_json::Value>, mode: ReasoningMode) {
    let (inline_token, system_token) = match mode {
        ReasoningMode::Enabled => (" /think", "/think"),
        ReasoningMode::Disabled => (" /no_think", "/no_think"),
        ReasoningMode::Auto => return,
    };
    let last_is_user = messages
        .last()
        .and_then(|m| m.get("role").and_then(|r| r.as_str()))
        == Some("user");
    if last_is_user {
        let m = messages.last_mut().unwrap();
        if let Some(c) = m.get("content").and_then(|c| c.as_str()) {
            let new = format!("{}{}", c, inline_token);
            m["content"] = serde_json::json!(new);
        }
    } else {
        if let Some(sys) = messages
            .iter_mut()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        {
            if let Some(c) = sys.get("content").and_then(|c| c.as_str()) {
                let new = format!("{}\n{}", c, system_token);
                sys["content"] = serde_json::json!(new);
            }
        } else {
            messages.insert(
                0,
                serde_json::json!({"role": "system", "content": system_token}),
            );
        }
    }
}

#[async_trait]
pub trait ModelBackend: Send + Sync {
    async fn health(&self) -> Result<BackendHealth, BackendError>;
    async fn capabilities(&self) -> Result<BackendCapabilities, BackendError>;
    async fn list_models(&self) -> Result<Vec<ModelAvailability>, BackendError>;
    async fn load_model(&self, request: LoadModelRequest) -> Result<LoadedModel, BackendError>;
    async fn unload_model(&self, model: &ModelId) -> Result<(), BackendError>;
    async fn generate(
        &self,
        request: BackendGenerationRequest,
        events: ModelEventSender,
        cancellation: CancellationToken,
    ) -> Result<BackendGenerationResponse, BackendError>;
}
