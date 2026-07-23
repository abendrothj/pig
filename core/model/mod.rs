//! Local model execution: structured request/response contracts (`types`), the model
//! registry mapping roles to physical models (`registry`), hardware-aware routing
//! (`scheduler`), and the coordinator-side HTTP client for talking to a worker
//! (`worker_client`). Worker-side process supervision (the llama.cpp backend, the HTTP
//! server, job queueing) lives in the separate `pig-worker` crate — pig's core process
//! never links a model runtime directly.

pub mod benchmark;
pub mod invoker;
pub mod metrics;
pub mod registry;
pub mod scheduler;
pub mod types;

pub use benchmark::{
    benchmark_store_dir, latest_matching_benchmark, load_benchmark_records, record_benchmark,
    worker_hardware_fingerprint, BenchmarkFingerprint, BenchmarkRecord,
};
pub use invoker::ModelInvoker;
pub use metrics::{
    AcceleratorMetrics, BenchmarkFreshness, CoordinatorMetricsSnapshot, JobMetrics, ModelLoadState,
    ModelMetrics, QueueMetrics, SchedulerDecisionMetrics, SystemMetrics, ThroughputMetrics,
    WorkerIdentityMetrics, WorkerLifecycleState, WorkerMetricsSnapshot, METRICS_SCHEMA_VERSION,
};
pub use registry::{
    discover_gguf_files, ModelEntry, ModelRegistry, RegistryError, ResolvedModelEntry,
};
pub use scheduler::{
    schedule, BenchmarkSummary, CandidatePlacement, RejectedCandidate, RoutingExplanation,
    SchedulingOverrides, ScoreComponent, WorkerLocality, WorkerSnapshot,
};
pub use types::*;
