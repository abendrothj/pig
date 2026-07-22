//! Injection point for invoking a model from inside the workflow execution engine.
//!
//! `core` never talks to a network or spawns a model runtime itself — it only defines
//! this trait. The concrete implementation (routing through the scheduler to an HTTP
//! worker) lives in the separate `lao-worker` crate and is constructed by the CLI,
//! which passes it in when building a `StepExecutor`. This keeps `core` free of any
//! async runtime or HTTP client dependency, and keeps model runtimes out of LAO's core
//! process (they run in a worker, a separate OS process, supervised over HTTP).

use crate::model::types::{ModelRequest, ModelResponse};

/// Execute a model request to completion. Streaming is a worker/CLI-level concern —
/// the workflow engine only needs the final resolved artifact and metadata.
pub trait ModelInvoker: Send + Sync {
    fn invoke(&self, request: ModelRequest) -> ModelResponse;
}
