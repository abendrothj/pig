//! Versioned observability schema. Every field here is either read straight from
//! state something else already owns (the job map, the queue semaphore, the
//! backend's own "what's loaded" query) or, where nothing currently owns the data
//! (e.g. a cumulative routing-decision count on an ephemeral CLI process), left
//! explicitly `None` with the reason documented rather than fabricated or backed by
//! a new counter that could drift from reality - see `CoordinatorMetricsSnapshot`'s
//! `scheduler_decision` field.
//!
//! No exporters, no persistent time-series storage, no aggregation framework here by
//! design - this is the schema plus the plain structs that carry it; a worker HTTP
//! endpoint and a CLI command are the only consumers today, in `pig-worker` and
//! `pig` respectively.

use crate::model::types::{AcceleratorKind, ModelId, WorkerId};
use serde::{Deserialize, Serialize};

/// Bumped whenever a field is added, removed, or changes meaning. Additive changes
/// (new optional fields) don't strictly require a bump for JSON compatibility, but
/// bump it anyway when in doubt - cheap insurance for "suitable for future
/// exporters."
pub const METRICS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerLifecycleState {
    Idle,
    Loading,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelLoadState {
    NotLoaded,
    Loading,
    Loaded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerIdentityMetrics {
    pub worker_id: WorkerId,
    pub uptime_seconds: u64,
    pub lifecycle_state: WorkerLifecycleState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMetrics {
    pub capacity: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JobMetrics {
    pub active: usize,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub cumulative_tokens_processed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub loaded_model_id: Option<ModelId>,
    pub load_state: ModelLoadState,
    pub last_load_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub cpu_utilization_percent: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AcceleratorMetrics {
    pub kind: Option<AcceleratorKind>,
    pub name: Option<String>,
    pub utilization_percent: Option<f32>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ThroughputMetrics {
    pub last_prompt_tokens_per_second: Option<f64>,
    pub last_generation_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerMetricsSnapshot {
    pub schema_version: u32,
    pub timestamp_unix_ms: u64,
    pub worker: WorkerIdentityMetrics,
    pub queue: QueueMetrics,
    pub jobs: JobMetrics,
    pub model: ModelMetrics,
    pub system: SystemMetrics,
    pub accelerator: AcceleratorMetrics,
    pub throughput: ThroughputMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SchedulerDecisionMetrics {
    pub duration_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkFreshness {
    pub worker_id: WorkerId,
    pub model_id: ModelId,
    /// `None` when no benchmark record exists at all for this (worker, model) pair -
    /// distinct from a record existing but being stale (see `fingerprint_valid`).
    pub age_seconds: Option<u64>,
    /// Whether the *latest* record's fingerprint currently matches - i.e. whether it
    /// would actually be used by the scheduler right now. `false` with a `Some`
    /// `age_seconds` means history exists but is stale (model/backend/hardware
    /// changed since it was recorded).
    pub fingerprint_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorMetricsSnapshot {
    pub schema_version: u32,
    pub timestamp_unix_ms: u64,
    pub known_workers: usize,
    pub connected_workers: usize,
    pub active_jobs: usize,
    pub queued_jobs: usize,
    pub successful_jobs: u64,
    pub failed_jobs: u64,
    /// Always `None` today - see the module-level doc comment. Kept in the schema
    /// (typed, not omitted) so a future pass that gives the coordinator somewhere
    /// real to own this state doesn't need a schema version bump to add it.
    pub scheduler_decision: Option<SchedulerDecisionMetrics>,
    pub benchmarks: Vec<BenchmarkFreshness>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_worker_snapshot() -> WorkerMetricsSnapshot {
        WorkerMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 1_753_000_000_000,
            worker: WorkerIdentityMetrics {
                worker_id: WorkerId::from("fedora-worker"),
                uptime_seconds: 8_040,
                lifecycle_state: WorkerLifecycleState::Running,
            },
            queue: QueueMetrics {
                capacity: 8,
                depth: 1,
            },
            jobs: JobMetrics {
                active: 1,
                completed: 42,
                failed: 1,
                cancelled: 0,
                cumulative_tokens_processed: 12_345,
            },
            model: ModelMetrics {
                loaded_model_id: Some(ModelId::from("qwen3-8b-q4")),
                load_state: ModelLoadState::Loaded,
                last_load_duration_ms: Some(1_204),
            },
            system: SystemMetrics {
                memory_used_bytes: Some(4_000_000_000),
                memory_total_bytes: Some(16_000_000_000),
                cpu_utilization_percent: Some(12.5),
            },
            accelerator: AcceleratorMetrics {
                kind: Some(AcceleratorKind::Cuda),
                name: Some("NVIDIA GeForce RTX 2080 SUPER".to_string()),
                utilization_percent: Some(84.0),
                memory_used_bytes: Some(6_800_000_000),
                memory_total_bytes: Some(8_589_934_592),
            },
            throughput: ThroughputMetrics {
                last_prompt_tokens_per_second: Some(521.0),
                last_generation_tokens_per_second: Some(78.4),
            },
        }
    }

    #[test]
    fn worker_snapshot_round_trips_through_json() {
        let snapshot = sample_worker_snapshot();
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: WorkerMetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn worker_snapshot_with_every_optional_field_absent_round_trips_as_null_not_zero() {
        let snapshot = WorkerMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 0,
            worker: WorkerIdentityMetrics {
                worker_id: WorkerId::from("w"),
                uptime_seconds: 0,
                lifecycle_state: WorkerLifecycleState::Idle,
            },
            queue: QueueMetrics {
                capacity: 8,
                depth: 0,
            },
            jobs: JobMetrics::default(),
            model: ModelMetrics {
                loaded_model_id: None,
                load_state: ModelLoadState::NotLoaded,
                last_load_duration_ms: None,
            },
            system: SystemMetrics::default(),
            accelerator: AcceleratorMetrics::default(),
            throughput: ThroughputMetrics::default(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(
            json.contains("\"loaded_model_id\":null"),
            "unavailable optional fields must serialize as null, not be fabricated or omitted: {}",
            json
        );
        assert!(json.contains("\"cpu_utilization_percent\":null"));
        assert!(json.contains("\"utilization_percent\":null"));
        let back: WorkerMetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn coordinator_snapshot_round_trips_with_scheduler_decision_absent() {
        let snapshot = CoordinatorMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 1_753_000_000_000,
            known_workers: 2,
            connected_workers: 1,
            active_jobs: 1,
            queued_jobs: 0,
            successful_jobs: 42,
            failed_jobs: 1,
            scheduler_decision: None,
            benchmarks: vec![BenchmarkFreshness {
                worker_id: WorkerId::from("fedora-worker"),
                model_id: ModelId::from("qwen3-8b-q4"),
                age_seconds: Some(120),
                fingerprint_valid: true,
            }],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"scheduler_decision\":null"));
        let back: CoordinatorMetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn benchmark_freshness_distinguishes_no_history_from_stale_history() {
        let no_history = BenchmarkFreshness {
            worker_id: WorkerId::from("w"),
            model_id: ModelId::from("m"),
            age_seconds: None,
            fingerprint_valid: false,
        };
        let stale = BenchmarkFreshness {
            worker_id: WorkerId::from("w"),
            model_id: ModelId::from("m"),
            age_seconds: Some(9_999),
            fingerprint_valid: false,
        };
        assert_ne!(
            no_history, stale,
            "no history and stale history must not collapse to the same representation"
        );
    }
}
