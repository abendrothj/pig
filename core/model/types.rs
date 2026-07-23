//! Model execution contracts: the structured request/response types that flow between
//! a workflow step, the scheduler, a worker, and a backend. Nothing in this module
//! talks to a network or a process — it is pure data plus validation.

use crate::artifact::Artifact;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

string_id!(RequestId);
string_id!(ModelId);
string_id!(WorkerId);
string_id!(JobId);

impl RequestId {
    /// Generate a fresh request id. Not derived from any external randomness source
    /// beyond `uuid::Uuid::new_v4` — callers needing determinism (e.g. cache keys)
    /// should not rely on request ids.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl JobId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Reasoning,
    Coding,
    Summarization,
    Extraction,
    Embedding,
    Reranking,
    Verification,
    Custom(String),
}

impl ModelRole {
    /// Parse a role name from configuration/YAML text. Unrecognized names become
    /// `Custom` rather than an error, so role vocabularies can grow without a schema
    /// change — the trade-off is that a typo silently becomes a new custom role
    /// instead of a validation error; callers matching against a known role set (the
    /// registry's `[models.roles.*]` tables) still catch that via "no candidates".
    pub fn parse(s: &str) -> Self {
        match s {
            "reasoning" => Self::Reasoning,
            "coding" => Self::Coding,
            "summarization" => Self::Summarization,
            "extraction" => Self::Extraction,
            "embedding" => Self::Embedding,
            "reranking" => Self::Reranking,
            "verification" => Self::Verification,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for ModelRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelRole::Reasoning => write!(f, "reasoning"),
            ModelRole::Coding => write!(f, "coding"),
            ModelRole::Summarization => write!(f, "summarization"),
            ModelRole::Extraction => write!(f, "extraction"),
            ModelRole::Embedding => write!(f, "embedding"),
            ModelRole::Reranking => write!(f, "reranking"),
            ModelRole::Verification => write!(f, "verification"),
            ModelRole::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// Selects a specific model, bypassing role-based candidate resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ModelSelector {
    Id(ModelId),
    Alias(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A model-requested function invocation. This is intentionally protocol-neutral:
/// OpenAI compatibility maps its `function` tool-call shape here, while the
/// scheduler only carries it as model context and never interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub id: String,
    pub function: ModelToolFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelToolFunction {
    pub name: String,
    /// JSON text supplied by the model. It remains text because models can emit an
    /// incomplete or invalid object; the agent runtime owns schema validation.
    pub arguments: String,
}

/// A single part within a multipart message content array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlContent },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrlContent {
    /// `data:image/<mime>;base64,<data>` or an `https://` URL.
    pub url: String,
}

/// Message content: either a plain string (ordinary chat) or a parts array
/// (multipart — text interleaved with images).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}

impl MessageContent {
    pub fn has_images(&self) -> bool {
        matches!(self, MessageContent::Parts(parts) if parts.iter().any(|p| matches!(p, ContentPart::ImageUrl { .. })))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: MessageRole,
    pub content: MessageContent,
    /// Present only on assistant messages which requested tool execution.
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
    /// Present only on a tool-result message and links it to its assistant call.
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

impl ModelMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: MessageContent::Text(content.into()),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: MessageContent::Text(content.into()),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    Json,
}

/// Controls whether the model reasons before responding.
/// `Auto` leaves the decision to the model or scheduler policy.
/// `Enabled` / `Disabled` are explicit overrides — translated to backend-specific
/// control tokens (e.g. `/think` / `/no_think` for Qwen3) by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParameters {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub seed: Option<u64>,
    #[serde(default)]
    pub stop: Vec<String>,
    pub response_format: Option<ResponseFormat>,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_mode: ReasoningMode,
}

impl GenerationParameters {
    pub fn validate(&self) -> Result<(), ModelRequestError> {
        if let Some(mt) = self.max_tokens {
            if mt == 0 {
                return Err(ModelRequestError::new(
                    "max_tokens",
                    "must be greater than 0",
                ));
            }
        }
        if let Some(t) = self.temperature {
            if !(0.0..=2.0).contains(&t) || !t.is_finite() {
                return Err(ModelRequestError::new(
                    "temperature",
                    "must be a finite value in [0.0, 2.0]",
                ));
            }
        }
        if let Some(p) = self.top_p {
            if !(0.0..=1.0).contains(&p) || !p.is_finite() {
                return Err(ModelRequestError::new(
                    "top_p",
                    "must be a finite value in [0.0, 1.0]",
                ));
            }
        }
        if let Some(p) = self.min_p {
            if !(0.0..=1.0).contains(&p) || !p.is_finite() {
                return Err(ModelRequestError::new(
                    "min_p",
                    "must be a finite value in [0.0, 1.0]",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceleratorKind {
    Cuda,
    Metal,
    Vulkan,
    Rocm,
    Cpu,
}

/// Where the request may be sent for execution.
/// `Any` allows routing to any registered worker, including remote ones.
/// `LocalOnly` restricts the scheduler to workers with `WorkerLocality::Local`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementPolicy {
    #[default]
    Any,
    LocalOnly,
}

impl fmt::Display for AcceleratorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AcceleratorKind::Cuda => "cuda",
            AcceleratorKind::Metal => "metal",
            AcceleratorKind::Vulkan => "vulkan",
            AcceleratorKind::Rocm => "rocm",
            AcceleratorKind::Cpu => "cpu",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequirements {
    pub minimum_context_tokens: Option<u32>,
    pub preferred_accelerator: Option<AcceleratorKind>,
    #[serde(default)]
    pub require_accelerator: bool,
    #[serde(default = "default_true")]
    pub allow_cpu_fallback: bool,
    pub maximum_queue_wait_ms: Option<u64>,
    pub maximum_execution_ms: Option<u64>,
    pub minimum_available_memory_bytes: Option<u64>,
    #[serde(default)]
    pub require_streaming: bool,
    #[serde(default)]
    pub placement_policy: PlacementPolicy,
    /// If `Some(true)`, only route to models tagged `reasoning = true` in their
    /// registry entry. Requests that require reasoning will be rejected rather than
    /// silently sent to a model that lacks the capability.
    #[serde(default)]
    pub reasoning: Option<bool>,
    /// If `Some(true)`, only route to models tagged `vision = true` in their
    /// registry entry. Auto-set when any user message contains image content parts.
    #[serde(default)]
    pub vision: Option<bool>,
}

fn default_true() -> bool {
    true
}

// Implemented by hand (rather than `#[derive(Default)]`) so `ModelRequirements::default()`
// agrees with what an empty JSON object deserializes to - `#[serde(default = "default_true")]`
// only affects deserialization, not the `Default` trait, and the two silently diverging
// (derive gives `false`, JSON gives `true`) would be a real footgun for any Rust call site
// using `..Default::default()`.
impl Default for ModelRequirements {
    fn default() -> Self {
        Self {
            minimum_context_tokens: None,
            preferred_accelerator: None,
            require_accelerator: false,
            allow_cpu_fallback: true,
            maximum_queue_wait_ms: None,
            maximum_execution_ms: None,
            minimum_available_memory_bytes: None,
            require_streaming: false,
            placement_policy: PlacementPolicy::Any,
            reasoning: None,
            vision: None,
        }
    }
}

impl ModelRequirements {
    pub fn validate(&self) -> Result<(), ModelRequestError> {
        if let Some(c) = self.minimum_context_tokens {
            if c == 0 {
                return Err(ModelRequestError::new(
                    "minimum_context_tokens",
                    "must be greater than 0",
                ));
            }
        }
        if self.require_accelerator && self.preferred_accelerator == Some(AcceleratorKind::Cpu) {
            return Err(ModelRequestError::new(
                "require_accelerator",
                "cannot require an accelerator while preferring CPU",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub step: String,
    pub output: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub request_id: RequestId,
    pub role: ModelRole,
    pub model: Option<ModelSelector>,
    pub messages: Vec<ModelMessage>,
    #[serde(default)]
    pub parameters: GenerationParameters,
    #[serde(default)]
    pub requirements: ModelRequirements,
    #[serde(default)]
    pub inputs: Vec<ArtifactReference>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl ModelRequest {
    pub fn validate(&self) -> Result<(), ModelRequestError> {
        if self.messages.is_empty() {
            return Err(ModelRequestError::new(
                "messages",
                "must contain at least one message",
            ));
        }
        self.parameters.validate()?;
        self.requirements.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequestError {
    pub field: String,
    pub message: String,
}

impl ModelRequestError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {}", self.field, self.message)
    }
}

impl std::error::Error for ModelRequestError {}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResponseStatus {
    Success,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    Cancelled,
    TimedOut,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedModel {
    pub model_id: ModelId,
    pub role: Option<ModelRole>,
    pub backend: String,
    pub identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationReason {
    UserRequested,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationInfo {
    pub reason: CancellationReason,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelExecutionMetadata {
    pub worker_id: WorkerId,
    pub host_name: String,
    pub backend: String,
    pub backend_version: Option<String>,
    pub model_id: ModelId,
    pub model_identity: String,
    pub accelerator: Option<AcceleratorKind>,
    pub cpu_threads: Option<u32>,
    pub gpu_layers: Option<i32>,
    pub context_tokens: Option<u32>,
    pub batch_size: Option<u32>,
    pub queue_wait_ms: u64,
    pub model_load_ms: u64,
    pub prompt_eval_ms: u64,
    pub generation_ms: u64,
    pub total_ms: u64,
    pub prompt_tokens: u32,
    pub generated_tokens: u32,
    pub prompt_tokens_per_second: Option<f64>,
    pub generation_tokens_per_second: Option<f64>,
    pub model_already_loaded: bool,
    pub cancellation: Option<CancellationInfo>,
    pub peak_memory_bytes: Option<u64>,
    pub peak_vram_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// One ordered piece of an in-progress model response. This is pig's canonical
/// streaming contract: it describes model output, not an HTTP or SSE wire format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelChunk {
    TextDelta {
        text: String,
    },
    /// A partial function call. `id` and `function_name` normally arrive in the
    /// first chunk; later chunks may contain only argument text.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        function_name: Option<String>,
        arguments_delta: Option<String>,
    },
    Finished {
        finish_reason: FinishReason,
        usage: Option<ModelUsage>,
    },
}

/// Structured failure detail. Failures are never communicated via `output` text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelExecutionError {
    ModelNotFound { model: String },
    WorkerUnavailable { worker: String },
    NoEligibleWorker { reason: String },
    BackendError { message: String },
    Timeout { after_ms: u64 },
    Cancelled,
    InvalidRequest { message: String },
}

impl fmt::Display for ModelExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelExecutionError::ModelNotFound { model } => {
                write!(f, "model '{}' not found", model)
            }
            ModelExecutionError::WorkerUnavailable { worker } => {
                write!(f, "worker '{}' is unavailable", worker)
            }
            ModelExecutionError::NoEligibleWorker { reason } => {
                write!(f, "no eligible worker: {}", reason)
            }
            ModelExecutionError::BackendError { message } => {
                write!(f, "backend error: {}", message)
            }
            ModelExecutionError::Timeout { after_ms } => {
                write!(f, "timed out after {}ms", after_ms)
            }
            ModelExecutionError::Cancelled => write!(f, "cancelled"),
            ModelExecutionError::InvalidRequest { message } => {
                write!(f, "invalid request: {}", message)
            }
        }
    }
}

impl std::error::Error for ModelExecutionError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub request_id: RequestId,
    pub status: ModelResponseStatus,
    pub output: Artifact,
    pub finish_reason: FinishReason,
    pub model: ResolvedModel,
    pub execution: ModelExecutionMetadata,
    pub usage: ModelUsage,
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
    pub error: Option<ModelExecutionError>,
}

impl ModelResponse {
    pub fn is_success(&self) -> bool {
        self.status == ModelResponseStatus::Success
    }
}

/// A single available inference resource: one model on one worker, with its current
/// load state and benchmark data. The scheduler chooses capacity (a ModelInstance),
/// not just a machine — two workers running the same model are not equivalent if one
/// is loaded, cold, or has different measured throughput.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInstance {
    /// Stable unique key: `"{worker_id}/{model_id}"`.
    pub instance_id: String,
    pub worker_id: WorkerId,
    pub model_id: ModelId,
    pub backend: String,
    /// Whether the model is currently loaded and ready (no cold-start penalty).
    pub loaded: bool,
    pub context_tokens: Option<u32>,
    pub accelerators: Vec<AcceleratorKind>,
    /// Fresh benchmark data for this (worker, model) pair; `None` when no matching
    /// record exists yet (e.g. model was never benchmarked on this worker).
    pub benchmark: Option<crate::model::BenchmarkSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> ModelRequest {
        ModelRequest {
            request_id: RequestId::generate(),
            role: ModelRole::Reasoning,
            model: Some(ModelSelector::Id(ModelId::from("qwen3-14b-q4"))),
            messages: vec![ModelMessage::system("be terse"), ModelMessage::user("hi")],
            parameters: GenerationParameters {
                max_tokens: Some(128),
                temperature: Some(0.2),
                ..Default::default()
            },
            requirements: ModelRequirements {
                minimum_context_tokens: Some(16384),
                preferred_accelerator: Some(AcceleratorKind::Metal),
                allow_cpu_fallback: true,
                ..Default::default()
            },
            inputs: vec![ArtifactReference {
                step: "step1".to_string(),
                output: "output".to_string(),
                label: Some("upstream".to_string()),
            }],
            metadata: BTreeMap::new(),
        }
    }

    fn sample_response() -> ModelResponse {
        ModelResponse {
            request_id: RequestId::generate(),
            status: ModelResponseStatus::Success,
            output: Artifact::Text("hello".to_string()),
            finish_reason: FinishReason::Stop,
            model: ResolvedModel {
                model_id: ModelId::from("qwen3-14b-q4"),
                role: Some(ModelRole::Reasoning),
                backend: "llama_cpp".to_string(),
                identity: "/models/qwen3.gguf".to_string(),
            },
            execution: ModelExecutionMetadata {
                worker_id: WorkerId::from("macbook-worker"),
                host_name: "macbook".to_string(),
                backend: "llama_cpp".to_string(),
                backend_version: Some("b9960".to_string()),
                model_id: ModelId::from("qwen3-14b-q4"),
                model_identity: "/models/qwen3.gguf".to_string(),
                accelerator: Some(AcceleratorKind::Metal),
                cpu_threads: Some(10),
                gpu_layers: Some(99),
                context_tokens: Some(16384),
                batch_size: Some(2048),
                queue_wait_ms: 5,
                model_load_ms: 0,
                prompt_eval_ms: 30,
                generation_ms: 200,
                total_ms: 235,
                prompt_tokens: 12,
                generated_tokens: 40,
                prompt_tokens_per_second: Some(400.0),
                generation_tokens_per_second: Some(200.0),
                model_already_loaded: true,
                cancellation: None,
                peak_memory_bytes: Some(4_000_000_000),
                peak_vram_bytes: Some(4_000_000_000),
            },
            usage: ModelUsage {
                prompt_tokens: 12,
                completion_tokens: 40,
                total_tokens: 52,
            },
            tool_calls: vec![],
            error: None,
        }
    }

    #[test]
    fn request_round_trips_through_json() {
        let req = sample_request();
        let json = serde_json::to_string(&req).unwrap();
        let back: ModelRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn tool_call_history_round_trips_through_json() {
        let mut request = sample_request();
        request.messages = vec![
            ModelMessage {
                role: MessageRole::Assistant,
                content: MessageContent::default(),
                tool_calls: vec![ModelToolCall {
                    id: "call_weather".to_string(),
                    function: ModelToolFunction {
                        name: "weather".to_string(),
                        arguments: r#"{"city":"San Francisco"}"#.to_string(),
                    },
                }],
                tool_call_id: None,
            },
            ModelMessage {
                role: MessageRole::Tool,
                content: MessageContent::Text(r#"{"temperature_f":62}"#.to_string()),
                tool_calls: vec![],
                tool_call_id: Some("call_weather".to_string()),
            },
        ];

        let json = serde_json::to_string(&request).unwrap();
        let back: ModelRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request, back);
        assert_eq!(
            back.messages[1].tool_call_id.as_deref(),
            Some("call_weather")
        );
    }

    #[test]
    fn model_chunks_preserve_partial_tool_arguments() {
        let chunks = vec![
            ModelChunk::TextDelta {
                text: "Looking that up".to_string(),
            },
            ModelChunk::ToolCallDelta {
                index: 1,
                id: Some("call_lookup".to_string()),
                function_name: Some("lookup".to_string()),
                arguments_delta: Some(r#"{"path":"src/lib"#.to_string()),
            },
            ModelChunk::ToolCallDelta {
                index: 1,
                id: None,
                function_name: None,
                arguments_delta: Some(r#".rs"}"#.to_string()),
            },
            ModelChunk::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: Some(ModelUsage {
                    prompt_tokens: 4,
                    completion_tokens: 2,
                    total_tokens: 6,
                }),
            },
        ];

        let json = serde_json::to_string(&chunks).unwrap();
        let back: Vec<ModelChunk> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, chunks);
    }

    #[test]
    fn response_round_trips_through_json() {
        let resp = sample_response();
        let json = serde_json::to_string(&resp).unwrap();
        let back: ModelResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn custom_role_round_trips() {
        let role = ModelRole::Custom("triage".to_string());
        let json = serde_json::to_string(&role).unwrap();
        let back: ModelRole = serde_json::from_str(&json).unwrap();
        assert_eq!(role, back);
    }

    #[test]
    fn error_is_structured_not_stringified_into_output() {
        let mut resp = sample_response();
        resp.status = ModelResponseStatus::Failed;
        resp.error = Some(ModelExecutionError::Timeout { after_ms: 5000 });
        assert!(!resp.is_success());
        assert_eq!(
            resp.error.as_ref().unwrap().to_string(),
            "timed out after 5000ms"
        );
        // output is untouched by the error - callers must read `.error`, not sniff `.output`.
        assert_eq!(resp.output, Artifact::Text("hello".to_string()));
    }

    #[test]
    fn zero_max_tokens_is_rejected() {
        let params = GenerationParameters {
            max_tokens: Some(0),
            ..Default::default()
        };
        assert!(params.validate().is_err());
    }

    #[test]
    fn out_of_range_temperature_is_rejected() {
        let params = GenerationParameters {
            temperature: Some(3.0),
            ..Default::default()
        };
        assert!(params.validate().is_err());
    }

    #[test]
    fn nan_temperature_is_rejected() {
        let params = GenerationParameters {
            temperature: Some(f32::NAN),
            ..Default::default()
        };
        assert!(params.validate().is_err());
    }

    #[test]
    fn zero_minimum_context_is_rejected() {
        let reqs = ModelRequirements {
            minimum_context_tokens: Some(0),
            ..Default::default()
        };
        assert!(reqs.validate().is_err());
    }

    #[test]
    fn contradictory_accelerator_requirement_is_rejected() {
        let reqs = ModelRequirements {
            require_accelerator: true,
            preferred_accelerator: Some(AcceleratorKind::Cpu),
            ..Default::default()
        };
        assert!(reqs.validate().is_err());
    }

    #[test]
    fn valid_request_passes_validation() {
        assert!(sample_request().validate().is_ok());
    }

    #[test]
    fn empty_messages_are_rejected() {
        let mut req = sample_request();
        req.messages.clear();
        assert!(req.validate().is_err());
    }

    #[test]
    fn allow_cpu_fallback_defaults_true_when_absent_from_json() {
        let reqs: ModelRequirements = serde_json::from_str("{}").unwrap();
        assert!(reqs.allow_cpu_fallback);
        assert!(!reqs.require_accelerator);
    }

    #[test]
    fn rust_default_agrees_with_empty_json_default() {
        let from_json: ModelRequirements = serde_json::from_str("{}").unwrap();
        assert_eq!(ModelRequirements::default(), from_json);
    }
}
