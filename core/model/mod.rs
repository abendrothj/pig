//! Local model execution: structured request/response contracts (`types`), the model
//! registry mapping roles to physical models (`registry`), hardware-aware routing
//! (`scheduler`), and the coordinator-side HTTP client for talking to a worker
//! (`worker_client`). Worker-side process supervision (the llama.cpp backend, the HTTP
//! server, job queueing) lives in the separate `lao-worker` crate — LAO's core process
//! never links a model runtime directly.

pub mod invoker;
pub mod registry;
pub mod types;

pub use invoker::ModelInvoker;
pub use registry::{
    discover_gguf_files, ModelEntry, ModelRegistry, RegistryError, ResolvedModelEntry,
};
pub use types::*;
