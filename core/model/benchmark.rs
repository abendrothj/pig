//! Benchmark records and the compatibility fingerprint that determines whether a past
//! measurement may influence scheduler scoring.
//!
//! A record is only ever "current" when its fingerprint matches the placement being
//! scored. Stale records remain stored for history (nothing here deletes them) but the
//! scheduler must never treat them as live data — see `latest_matching_benchmark`.

use crate::model::types::{AcceleratorKind, ModelId, WorkerId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Identity of the exact conditions a benchmark was measured under. `model_id`,
/// `backend`, and `worker_hardware_fingerprint` are always known and compared
/// strictly; every other field is compared only when *both* the stored record and the
/// current placement have a known value for it — an unknown value never causes a
/// mismatch by itself (e.g. v0.5 doesn't yet configure per-model `gpu_layers` up front,
/// so that field can't be evaluated before a model loads; leaving it as `None` on the
/// "current" side means it simply isn't checked, not that anything not matches).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkFingerprint {
    pub model_id: ModelId,
    pub model_file_size_bytes: Option<u64>,
    /// Preferred over size alone when available; v0.5 does not compute this by default
    /// (hashing a multi-GB GGUF file on every routing decision would be its own
    /// performance problem), so it is `None` on the "current" side today and this
    /// field is effectively dormant until something populates it.
    pub model_file_hash: Option<String>,
    pub backend: String,
    pub backend_version: Option<String>,
    pub worker_hardware_fingerprint: String,
    pub accelerator: Option<AcceleratorKind>,
    pub context_tokens: Option<u32>,
    pub gpu_layers: Option<i32>,
    pub cpu_threads: Option<u32>,
    pub batch_size: Option<u32>,
}

impl BenchmarkFingerprint {
    pub fn matches(&self, current: &BenchmarkFingerprint) -> bool {
        self.model_id == current.model_id
            && self.backend == current.backend
            && self.worker_hardware_fingerprint == current.worker_hardware_fingerprint
            && opt_eq_when_both_known(&self.model_file_size_bytes, &current.model_file_size_bytes)
            && opt_eq_when_both_known(&self.model_file_hash, &current.model_file_hash)
            && opt_eq_when_both_known(&self.backend_version, &current.backend_version)
            && opt_eq_when_both_known(&self.accelerator, &current.accelerator)
            && opt_eq_when_both_known(&self.context_tokens, &current.context_tokens)
            && opt_eq_when_both_known(&self.gpu_layers, &current.gpu_layers)
            && opt_eq_when_both_known(&self.cpu_threads, &current.cpu_threads)
            && opt_eq_when_both_known(&self.batch_size, &current.batch_size)
    }
}

fn opt_eq_when_both_known<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// A coarse but real hardware identity ("what machine was this measured on"), built
/// from data every worker already reports rather than a new probing mechanism.
pub fn worker_hardware_fingerprint(os: &str, arch: &str, hostname: &str) -> String {
    format!("{}-{}-{}", os, arch, hostname)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRecord {
    pub fingerprint: BenchmarkFingerprint,
    pub worker_id: WorkerId,
    pub timestamp_unix_ms: u64,
    pub success: bool,
    pub model_load_ms: u64,
    pub prompt_tokens_per_second: Option<f64>,
    pub generation_tokens_per_second: Option<f64>,
    pub total_ms: u64,
    pub prompt_tokens: u32,
    pub generated_tokens: u32,
}

pub fn benchmark_store_dir() -> PathBuf {
    PathBuf::from(".pig_benchmarks")
}

fn benchmark_file_path(model_id: &ModelId) -> PathBuf {
    benchmark_store_dir().join(format!("{}.jsonl", model_id.0.replace(['/', ' '], "_")))
}

/// Append one record. Never overwrites or prunes history — invalidation is handled at
/// read time by `latest_matching_benchmark`, not by deleting old data.
pub fn record_benchmark(model_id: &ModelId, record: &BenchmarkRecord) -> std::io::Result<()> {
    let dir = benchmark_store_dir();
    std::fs::create_dir_all(&dir)?;
    let path = benchmark_file_path(model_id);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

/// All records for a model, oldest first. Malformed lines (partial writes, format
/// changes) are skipped rather than failing the whole read, matching the project's
/// existing convention for tolerant local-state loading (see `state_manager`).
pub fn load_benchmark_records(model_id: &ModelId) -> Vec<BenchmarkRecord> {
    let path = benchmark_file_path(model_id);
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<BenchmarkRecord>(line).ok())
        .collect()
}

/// The most recent record for `(model_id, worker_id)` whose fingerprint matches
/// `current` — i.e. the most recent measurement that is still valid evidence for this
/// exact placement. Returns `None` when there is no history or every record on file is
/// stale relative to `current`.
pub fn latest_matching_benchmark(
    model_id: &ModelId,
    worker_id: &WorkerId,
    current: &BenchmarkFingerprint,
) -> Option<BenchmarkRecord> {
    load_benchmark_records(model_id)
        .into_iter()
        .filter(|r| &r.worker_id == worker_id && r.success && r.fingerprint.matches(current))
        .max_by_key(|r| r.timestamp_unix_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_fingerprint() -> BenchmarkFingerprint {
        BenchmarkFingerprint {
            model_id: ModelId::from("m1"),
            model_file_size_bytes: Some(1_000_000),
            model_file_hash: None,
            backend: "llama_cpp".to_string(),
            backend_version: Some("b9960".to_string()),
            worker_hardware_fingerprint: "macos-aarch64-host1".to_string(),
            accelerator: Some(AcceleratorKind::Metal),
            context_tokens: Some(2048),
            gpu_layers: Some(99),
            cpu_threads: Some(10),
            batch_size: Some(2048),
        }
    }

    #[test]
    fn identical_fingerprints_match() {
        let f = base_fingerprint();
        assert!(f.matches(&f.clone()));
    }

    #[test]
    fn different_model_id_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.model_id = ModelId::from("m2");
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_file_size_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.model_file_size_bytes = Some(2_000_000);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_hash_does_not_match_even_if_size_agrees() {
        let mut stored = base_fingerprint();
        stored.model_file_hash = Some("aaa".to_string());
        let mut current = base_fingerprint();
        current.model_file_hash = Some("bbb".to_string());
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_backend_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.backend = "fake".to_string();
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_backend_version_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.backend_version = Some("b9999".to_string());
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_worker_hardware_fingerprint_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.worker_hardware_fingerprint = "linux-x86_64-host2".to_string();
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_accelerator_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.accelerator = Some(AcceleratorKind::Cuda);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_context_tokens_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.context_tokens = Some(4096);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_gpu_layers_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.gpu_layers = Some(0);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_cpu_threads_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.cpu_threads = Some(4);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn different_batch_size_does_not_match() {
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.batch_size = Some(512);
        assert!(!stored.matches(&current));
    }

    #[test]
    fn unknown_current_side_field_does_not_block_a_match() {
        // v0.5 cannot determine gpu_layers/cpu_threads/batch_size before a model is
        // loaded, so "current" legitimately has None there; that must not make an
        // otherwise-valid historical record look stale.
        let stored = base_fingerprint();
        let mut current = base_fingerprint();
        current.gpu_layers = None;
        current.cpu_threads = None;
        current.batch_size = None;
        current.model_file_hash = None;
        assert!(stored.matches(&current));
    }

    #[test]
    fn record_round_trips_through_load_and_filters_by_worker_and_freshness() {
        let dir = benchmark_store_dir();
        let unique = format!("bench_test_model_{}", std::process::id());
        let model_id = ModelId::from(unique.clone());
        let _ = std::fs::remove_file(dir.join(format!("{}.jsonl", unique)));

        let fp = base_fingerprint();
        let mut fp_for_model = fp.clone();
        fp_for_model.model_id = model_id.clone();

        let older = BenchmarkRecord {
            fingerprint: fp_for_model.clone(),
            worker_id: WorkerId::from("w1"),
            timestamp_unix_ms: 1000,
            success: true,
            model_load_ms: 10,
            prompt_tokens_per_second: Some(100.0),
            generation_tokens_per_second: Some(50.0),
            total_ms: 20,
            prompt_tokens: 5,
            generated_tokens: 5,
        };
        let newer = BenchmarkRecord {
            timestamp_unix_ms: 2000,
            generation_tokens_per_second: Some(60.0),
            ..older.clone()
        };
        let other_worker = BenchmarkRecord {
            worker_id: WorkerId::from("w2"),
            timestamp_unix_ms: 3000,
            ..older.clone()
        };
        let mut stale_fp = fp_for_model.clone();
        stale_fp.context_tokens = Some(999);
        let stale = BenchmarkRecord {
            fingerprint: stale_fp,
            timestamp_unix_ms: 4000, // newest by time, but fingerprint won't match
            ..older.clone()
        };

        record_benchmark(&model_id, &older).unwrap();
        record_benchmark(&model_id, &newer).unwrap();
        record_benchmark(&model_id, &other_worker).unwrap();
        record_benchmark(&model_id, &stale).unwrap();

        let all = load_benchmark_records(&model_id);
        assert_eq!(all.len(), 4);

        let best =
            latest_matching_benchmark(&model_id, &WorkerId::from("w1"), &fp_for_model).unwrap();
        assert_eq!(
            best.timestamp_unix_ms, 2000,
            "should pick the newest matching record, not the newest overall"
        );

        let _ = std::fs::remove_file(dir.join(format!("{}.jsonl", unique)));
    }

    #[test]
    fn no_history_yields_none_not_a_panic() {
        let model_id = ModelId::from(format!("no_history_{}", std::process::id()));
        assert!(
            latest_matching_benchmark(&model_id, &WorkerId::from("w1"), &base_fingerprint())
                .is_none()
        );
    }
}
