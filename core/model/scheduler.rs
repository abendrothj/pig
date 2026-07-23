//! Deterministic, hardware-aware scheduling across registered workers.
//!
//! Two phases, always in this order: hard constraints filter out ineligible
//! (worker, model) placements with an explicit rejection reason each; then eligible
//! placements are scored with a fully itemized, non-hidden breakdown. Scheduling is a
//! pure function of its inputs — identical `(request, registry, workers, overrides)`
//! always produces the identical `RoutingExplanation`, including tie-breaks (by
//! worker id, then model id, both lexicographic).

use crate::model::registry::ModelRegistry;
use crate::model::types::{AcceleratorKind, ModelId, ModelRequest, PlacementPolicy, WorkerId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerLocality {
    Local,
    Remote,
}

/// Benchmark-derived throughput for one specific model on one specific worker, already
/// filtered to only fingerprint-matching (non-stale) history by whoever builds the
/// `WorkerSnapshot` — see `core::model::benchmark::latest_matching_benchmark`. The
/// scheduler itself does no fingerprint checking; by the time data reaches here it must
/// already be current, which is why this is keyed per-model rather than a single
/// worker-wide scalar (a stale-vs-current distinction is meaningless without knowing
/// *which* model a measurement was for).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    pub prompt_tokens_per_second: Option<f64>,
    pub generation_tokens_per_second: Option<f64>,
    /// Median time-to-first-token from the most recent valid benchmark record, in ms.
    pub p50_ttft_ms: Option<u32>,
}

/// A point-in-time description of one worker's health and capacity, as reported by
/// `/v1/health` + `/v1/capabilities` (or synthesized for tests). The scheduler never
/// fetches this itself — it's pure data in, `RoutingExplanation` out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub worker_id: WorkerId,
    pub healthy: bool,
    pub backend: String,
    pub backend_version: Option<String>,
    pub backend_healthy: bool,
    /// Same coarse `{os}-{arch}-{hostname}` identity used in `BenchmarkFingerprint`
    /// (see `core::model::benchmark::worker_hardware_fingerprint`); `None` only when
    /// the worker's `/v1/capabilities` response is unreadable.
    pub worker_hardware_fingerprint: Option<String>,
    pub accelerators: Vec<AcceleratorKind>,
    pub loaded_models: Vec<ModelId>,
    pub known_models: Vec<ModelId>,
    pub queue_depth: usize,
    pub active_jobs: usize,
    pub max_queued_jobs: usize,
    pub available_memory_bytes: Option<u64>,
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_tools: bool,
    pub locality: WorkerLocality,
    /// Per-model, already fingerprint-filtered benchmark throughput. Absence of an
    /// entry for a model means "no current benchmark data," not "zero throughput."
    pub benchmarks: BTreeMap<ModelId, BenchmarkSummary>,
    pub priority: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchedulingOverrides {
    pub force_worker: Option<WorkerId>,
    pub force_model: Option<ModelId>,
    pub force_backend: Option<String>,
    pub force_cpu: bool,
    pub prefer_accelerator: Option<AcceleratorKind>,
    pub disable_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreComponent {
    pub label: String,
    pub value: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidatePlacement {
    pub worker_id: WorkerId,
    pub model_id: ModelId,
    pub backend: String,
    pub score: i64,
    pub score_breakdown: Vec<ScoreComponent>,
    pub used_cpu_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub worker_id: WorkerId,
    pub model_id: Option<ModelId>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingExplanation {
    pub selected: Option<CandidatePlacement>,
    pub rejected: Vec<RejectedCandidate>,
    pub all_candidates: Vec<CandidatePlacement>,
}

impl fmt::Display for RoutingExplanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.selected {
            Some(placement) => {
                writeln!(
                    f,
                    "Selected {} / {}:",
                    placement.worker_id, placement.model_id
                )?;
                for component in &placement.score_breakdown {
                    let sign = if component.value >= 0 { "+" } else { "-" };
                    writeln!(f, "{} {} {}", sign, component.value.abs(), component.label)?;
                }
                if placement.used_cpu_fallback {
                    writeln!(f, "(CPU fallback used)")?;
                }
            }
            None => writeln!(f, "No eligible worker")?,
        }
        for rejected in &self.rejected {
            match &rejected.model_id {
                Some(model) => writeln!(f, "\nRejected {} / {}:", rejected.worker_id, model)?,
                None => writeln!(f, "\nRejected {}:", rejected.worker_id)?,
            }
            for reason in &rejected.reasons {
                writeln!(f, "- {}", reason)?;
            }
        }
        Ok(())
    }
}

fn candidate_models(
    request: &ModelRequest,
    registry: &ModelRegistry,
    overrides: &SchedulingOverrides,
) -> Vec<ModelId> {
    if let Some(model) = &overrides.force_model {
        return vec![model.clone()];
    }
    match &request.model {
        Some(crate::model::types::ModelSelector::Id(id)) => vec![id.clone()],
        Some(crate::model::types::ModelSelector::Alias(alias)) => {
            vec![ModelId::from(alias.clone())]
        }
        None => registry
            .candidates_for_role(&request.role)
            .into_iter()
            .map(|e| e.id.clone())
            .collect(),
    }
}

/// Hard constraints. Returns `Ok(())` when the (worker, model) placement is eligible,
/// `Err(reasons)` (one entry per failed constraint, all evaluated rather than
/// short-circuiting on the first) otherwise.
fn check_hard_constraints(
    request: &ModelRequest,
    registry: &ModelRegistry,
    worker: &WorkerSnapshot,
    model_id: &ModelId,
    overrides: &SchedulingOverrides,
) -> Result<bool, Vec<String>> {
    let mut reasons = Vec::new();
    let mut used_cpu_fallback = false;

    if request.requirements.placement_policy == PlacementPolicy::LocalOnly
        && worker.locality != WorkerLocality::Local
    {
        reasons
            .push("placement policy requires a local worker but this worker is remote".to_string());
    }
    if !worker.healthy {
        reasons.push("worker is unhealthy".to_string());
    }
    if !worker.backend_healthy {
        reasons.push("backend is unhealthy".to_string());
    }
    if let Some(force_backend) = &overrides.force_backend {
        if &worker.backend != force_backend {
            reasons.push(format!(
                "backend '{}' does not match forced backend '{}'",
                worker.backend, force_backend
            ));
        }
    }
    if !worker.known_models.contains(model_id) {
        reasons.push(format!("model '{}' is not known to this worker", model_id));
    }

    let entry = registry.get(model_id);
    match entry {
        None => reasons.push(format!("model '{}' is not in the registry", model_id)),
        Some(entry) => {
            if let Some(required_ctx) = request.requirements.minimum_context_tokens {
                let supported = entry.context_tokens.unwrap_or(0);
                if supported < required_ctx {
                    reasons.push(format!(
                        "model context {} tokens is below the required {} tokens",
                        supported, required_ctx
                    ));
                }
            }
            if let (Some(estimated), Some(available)) =
                (entry.estimated_memory_bytes, worker.available_memory_bytes)
            {
                if estimated > available {
                    reasons.push(format!(
                        "estimated memory {} bytes exceeds available {} bytes",
                        estimated, available
                    ));
                }
            }
            if let Some(required_avail) = request.requirements.minimum_available_memory_bytes {
                if let Some(available) = worker.available_memory_bytes {
                    if available < required_avail {
                        reasons.push(format!(
                            "available memory {} bytes is below the required {} bytes",
                            available, required_avail
                        ));
                    }
                }
            }
        }
    }

    let real_accelerators: Vec<AcceleratorKind> = worker
        .accelerators
        .iter()
        .copied()
        .filter(|a| *a != AcceleratorKind::Cpu)
        .collect();
    let has_real_accelerator = !real_accelerators.is_empty() && !overrides.force_cpu;
    let require_accelerator = request.requirements.require_accelerator && !overrides.force_cpu;

    if require_accelerator && !has_real_accelerator {
        reasons.push("an accelerator is required but this worker has none available".to_string());
    } else if !has_real_accelerator {
        let fallback_allowed =
            request.requirements.allow_cpu_fallback && !overrides.disable_fallback;
        if !fallback_allowed {
            reasons.push("no accelerator is available and CPU fallback is not allowed".to_string());
        } else {
            used_cpu_fallback = true;
        }
    }

    if request.requirements.require_streaming && !worker.supports_streaming {
        reasons.push(
            "streaming is required but this worker's backend does not support it".to_string(),
        );
    }
    if worker.queue_depth >= worker.max_queued_jobs {
        reasons.push(format!(
            "worker queue is full ({}/{})",
            worker.queue_depth, worker.max_queued_jobs
        ));
    }

    if reasons.is_empty() {
        Ok(used_cpu_fallback)
    } else {
        Err(reasons)
    }
}

fn score_placement(
    request: &ModelRequest,
    registry: &ModelRegistry,
    worker: &WorkerSnapshot,
    model_id: &ModelId,
    used_cpu_fallback: bool,
    overrides: &SchedulingOverrides,
) -> Vec<ScoreComponent> {
    let mut components = Vec::new();

    if worker.loaded_models.contains(model_id) {
        components.push(ScoreComponent {
            label: "model already loaded".to_string(),
            value: 100,
        });
    } else {
        components.push(ScoreComponent {
            label: "model requires loading".to_string(),
            value: -10,
        });
    }

    let preferred = overrides
        .prefer_accelerator
        .or(request.requirements.preferred_accelerator);
    if let Some(preferred) = preferred {
        if !used_cpu_fallback && worker.accelerators.contains(&preferred) {
            components.push(ScoreComponent {
                label: format!("preferred {} accelerator", preferred),
                value: 40,
            });
        }
    }

    if let Some(entry) = registry.get(model_id) {
        if let (Some(estimated), Some(available)) =
            (entry.estimated_memory_bytes, worker.available_memory_bytes)
        {
            if available > estimated {
                components.push(ScoreComponent {
                    label: "memory headroom".to_string(),
                    value: 14,
                });
            }
        }
        if entry.roles.contains(&request.role) {
            components.push(ScoreComponent {
                label: format!("declared '{}' role suitability", request.role),
                value: 8,
            });
        }
    }

    // Only ever a fingerprint-matched (current) measurement for this exact model - see
    // `BenchmarkSummary`'s doc comment. A benchmark for a different model, or a stale
    // one for this model, is absent here, not present-but-wrong.
    if let Some(summary) = worker.benchmarks.get(model_id) {
        if let Some(tps) = summary.generation_tokens_per_second {
            components.push(ScoreComponent {
                label: "measured generation throughput".to_string(),
                value: (tps / 10.0).round() as i64,
            });
        }
        if let Some(tps) = summary.prompt_tokens_per_second {
            components.push(ScoreComponent {
                label: "measured prompt-processing throughput".to_string(),
                value: (tps / 20.0).round() as i64,
            });
        }
        // Lower TTFT is better; invert so higher score = faster first token.
        if let Some(ttft_ms) = summary.p50_ttft_ms {
            let score = 50i64.saturating_sub(ttft_ms as i64 / 100);
            if score > 0 {
                components.push(ScoreComponent {
                    label: format!("low time-to-first-token (p50 {}ms)", ttft_ms),
                    value: score,
                });
            }
        }
    }

    if worker.queue_depth > 0 {
        components.push(ScoreComponent {
            label: format!("{} active queued request(s)", worker.queue_depth),
            value: -(worker.queue_depth as i64) * 5,
        });
    }
    if worker.active_jobs > 0 {
        components.push(ScoreComponent {
            label: format!("{} active job(s)", worker.active_jobs),
            value: -(worker.active_jobs as i64) * 3,
        });
    }

    match worker.locality {
        WorkerLocality::Local => components.push(ScoreComponent {
            label: "local worker".to_string(),
            value: 5,
        }),
        WorkerLocality::Remote => components.push(ScoreComponent {
            label: "network transfer penalty".to_string(),
            value: -15,
        }),
    }

    if worker.priority != 0 {
        components.push(ScoreComponent {
            label: "configured priority".to_string(),
            value: worker.priority,
        });
    }

    components
}

pub fn schedule(
    request: &ModelRequest,
    registry: &ModelRegistry,
    workers: &[WorkerSnapshot],
    overrides: &SchedulingOverrides,
) -> RoutingExplanation {
    let models = candidate_models(request, registry, overrides);

    let mut sorted_workers: Vec<&WorkerSnapshot> = workers.iter().collect();
    sorted_workers.sort_by(|a, b| a.worker_id.0.cmp(&b.worker_id.0));

    if let Some(forced) = &overrides.force_worker {
        sorted_workers.retain(|w| &w.worker_id == forced);
        if sorted_workers.is_empty() {
            return RoutingExplanation {
                selected: None,
                rejected: vec![RejectedCandidate {
                    worker_id: forced.clone(),
                    model_id: None,
                    reasons: vec!["forced worker is not registered".to_string()],
                }],
                all_candidates: vec![],
            };
        }
    }

    let mut sorted_models = models.clone();
    sorted_models.sort_by(|a, b| a.0.cmp(&b.0));

    let mut rejected = Vec::new();
    let mut all_candidates = Vec::new();

    for worker in &sorted_workers {
        let mut worker_had_any_model_reason = false;
        for model_id in &sorted_models {
            match check_hard_constraints(request, registry, worker, model_id, overrides) {
                Ok(used_cpu_fallback) => {
                    let breakdown = score_placement(
                        request,
                        registry,
                        worker,
                        model_id,
                        used_cpu_fallback,
                        overrides,
                    );
                    let score = breakdown.iter().map(|c| c.value).sum();
                    all_candidates.push(CandidatePlacement {
                        worker_id: worker.worker_id.clone(),
                        model_id: model_id.clone(),
                        backend: worker.backend.clone(),
                        score,
                        score_breakdown: breakdown,
                        used_cpu_fallback,
                    });
                    worker_had_any_model_reason = true;
                }
                Err(reasons) => {
                    rejected.push(RejectedCandidate {
                        worker_id: worker.worker_id.clone(),
                        model_id: Some(model_id.clone()),
                        reasons,
                    });
                }
            }
        }
        if sorted_models.is_empty() && !worker_had_any_model_reason {
            rejected.push(RejectedCandidate {
                worker_id: worker.worker_id.clone(),
                model_id: None,
                reasons: vec!["no candidate model for the requested role/selector".to_string()],
            });
        }
    }

    all_candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.worker_id.0.cmp(&b.worker_id.0))
            .then_with(|| a.model_id.0.cmp(&b.model_id.0))
    });

    let selected = all_candidates.first().cloned();

    RoutingExplanation {
        selected,
        rejected,
        all_candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::registry::ModelEntry;
    use crate::model::types::{
        GenerationParameters, ModelMessage, ModelRequirements, ModelRole, RequestId,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn entry(id: &str, ctx: u32, mem: u64, roles: &[ModelRole]) -> ModelEntry {
        ModelEntry {
            id: ModelId::from(id),
            format: "gguf".to_string(),
            path: PathBuf::from(format!("/models/{}.gguf", id)),
            backend: "llama_cpp".to_string(),
            context_tokens: Some(ctx),
            estimated_memory_bytes: Some(mem),
            roles: roles.to_vec(),
            execution_config: serde_json::Value::Null,
            tool_calling: None,
        }
    }

    fn registry() -> ModelRegistry {
        let entries = vec![
            entry(
                "big",
                32768,
                11_000_000_000,
                &[ModelRole::Reasoning, ModelRole::Coding],
            ),
            entry(
                "small",
                32768,
                4_000_000_000,
                &[ModelRole::Reasoning, ModelRole::Coding],
            ),
        ];
        let mut roles = BTreeMap::new();
        roles.insert(
            ModelRole::Reasoning,
            vec![ModelId::from("big"), ModelId::from("small")],
        );
        ModelRegistry::new(entries, roles).unwrap()
    }

    fn worker(id: &str) -> WorkerSnapshot {
        WorkerSnapshot {
            worker_id: WorkerId::from(id),
            healthy: true,
            backend: "llama_cpp".to_string(),
            backend_version: Some("test-version".to_string()),
            backend_healthy: true,
            worker_hardware_fingerprint: Some("test-os-test-arch-test-host".to_string()),
            accelerators: vec![AcceleratorKind::Cpu],
            loaded_models: vec![],
            known_models: vec![ModelId::from("big"), ModelId::from("small")],
            queue_depth: 0,
            active_jobs: 0,
            max_queued_jobs: 16,
            available_memory_bytes: Some(20_000_000_000),
            supports_streaming: true,
            supports_tools: true,
            locality: WorkerLocality::Local,
            benchmarks: BTreeMap::new(),
            priority: 0,
        }
    }

    fn request() -> ModelRequest {
        ModelRequest {
            request_id: RequestId::generate(),
            role: ModelRole::Reasoning,
            model: None,
            messages: vec![ModelMessage::user("hi")],
            parameters: GenerationParameters::default(),
            requirements: ModelRequirements::default(),
            inputs: vec![],
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn selects_the_highest_scoring_eligible_placement() {
        let reg = registry();
        let mut w1 = worker("w1");
        w1.loaded_models = vec![ModelId::from("small")];
        let workers = vec![w1, worker("w2")];
        let explanation = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());
        let selected = explanation.selected.unwrap();
        assert_eq!(selected.worker_id, WorkerId::from("w1"));
        assert_eq!(selected.model_id, ModelId::from("small"));
    }

    #[test]
    fn scheduling_is_deterministic_for_identical_input() {
        let reg = registry();
        let workers = vec![worker("w2"), worker("w1")]; // insertion order shouldn't matter
        let r1 = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());
        let r2 = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());
        assert_eq!(r1, r2);
    }

    #[test]
    fn unhealthy_worker_is_hard_rejected() {
        let reg = registry();
        let mut w = worker("w1");
        w.healthy = false;
        let explanation = schedule(&request(), &reg, &[w], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation
            .rejected
            .iter()
            .any(|r| r.reasons.iter().any(|m| m.contains("unhealthy"))));
    }

    #[test]
    fn insufficient_context_is_hard_rejected() {
        let reg = registry();
        let mut req = request();
        req.requirements.minimum_context_tokens = Some(999_999);
        let explanation = schedule(&req, &reg, &[worker("w1")], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation
            .rejected
            .iter()
            .any(|r| r.reasons.iter().any(|m| m.contains("below the required"))));
    }

    #[test]
    fn required_accelerator_unavailable_is_hard_rejected() {
        let reg = registry();
        let mut req = request();
        req.requirements.require_accelerator = true;
        let explanation = schedule(&req, &reg, &[worker("w1")], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation.rejected.iter().any(|r| r
            .reasons
            .iter()
            .any(|m| m.contains("accelerator is required"))));
    }

    #[test]
    fn cpu_fallback_is_used_when_allowed_and_no_accelerator_present() {
        let reg = registry();
        let req = request(); // allow_cpu_fallback defaults true
        let explanation = schedule(&req, &reg, &[worker("w1")], &SchedulingOverrides::default());
        assert!(explanation.selected.unwrap().used_cpu_fallback);
    }

    #[test]
    fn cpu_fallback_disabled_without_accelerator_is_rejected() {
        let reg = registry();
        let mut req = request();
        req.requirements.allow_cpu_fallback = false;
        let explanation = schedule(&req, &reg, &[worker("w1")], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation.rejected.iter().any(|r| r
            .reasons
            .iter()
            .any(|m| m.contains("CPU fallback is not allowed"))));
    }

    #[test]
    fn queue_full_worker_is_hard_rejected() {
        let reg = registry();
        let mut w = worker("w1");
        w.queue_depth = 16;
        w.max_queued_jobs = 16;
        let explanation = schedule(&request(), &reg, &[w], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation
            .rejected
            .iter()
            .any(|r| r.reasons.iter().any(|m| m.contains("queue is full"))));
    }

    #[test]
    fn streaming_required_but_unsupported_is_hard_rejected() {
        let reg = registry();
        let mut req = request();
        req.requirements.require_streaming = true;
        let mut w = worker("w1");
        w.supports_streaming = false;
        let explanation = schedule(&req, &reg, &[w], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
    }

    #[test]
    fn force_worker_restricts_selection_to_that_worker() {
        let reg = registry();
        let mut w1 = worker("w1");
        w1.loaded_models = vec![ModelId::from("small")]; // would normally win
        let workers = vec![w1, worker("w2")];
        let overrides = SchedulingOverrides {
            force_worker: Some(WorkerId::from("w2")),
            ..Default::default()
        };
        let explanation = schedule(&request(), &reg, &workers, &overrides);
        assert_eq!(
            explanation.selected.unwrap().worker_id,
            WorkerId::from("w2")
        );
    }

    #[test]
    fn impossible_forced_worker_fails_clearly_rather_than_routing_elsewhere() {
        let reg = registry();
        let workers = vec![worker("w1"), worker("w2")];
        let overrides = SchedulingOverrides {
            force_worker: Some(WorkerId::from("ghost")),
            ..Default::default()
        };
        let explanation = schedule(&request(), &reg, &workers, &overrides);
        assert!(explanation.selected.is_none());
        assert_eq!(explanation.rejected.len(), 1);
        assert_eq!(explanation.rejected[0].worker_id, WorkerId::from("ghost"));
    }

    #[test]
    fn force_model_restricts_candidate_models() {
        let reg = registry();
        let overrides = SchedulingOverrides {
            force_model: Some(ModelId::from("big")),
            ..Default::default()
        };
        let explanation = schedule(&request(), &reg, &[worker("w1")], &overrides);
        assert_eq!(explanation.selected.unwrap().model_id, ModelId::from("big"));
    }

    #[test]
    fn explanation_display_lists_selection_and_rejections() {
        let reg = registry();
        let mut unhealthy = worker("w2");
        unhealthy.healthy = false;
        let workers = vec![worker("w1"), unhealthy];
        let explanation = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());
        let text = explanation.to_string();
        assert!(text.starts_with("Selected w1"));
        assert!(text.contains("Rejected w2"));
    }

    #[test]
    fn local_only_policy_rejects_remote_worker() {
        let reg = registry();
        let mut remote = worker("w1");
        remote.locality = WorkerLocality::Remote;
        let mut req = request();
        req.requirements.placement_policy = PlacementPolicy::LocalOnly;
        let explanation = schedule(&req, &reg, &[remote], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation
            .rejected
            .iter()
            .any(|r| r.reasons.iter().any(|m| m.contains("local"))));
    }

    #[test]
    fn local_only_policy_selects_local_worker() {
        let reg = registry();
        let mut req = request();
        req.requirements.placement_policy = PlacementPolicy::LocalOnly;
        let explanation = schedule(&req, &reg, &[worker("w1")], &SchedulingOverrides::default());
        assert!(explanation.selected.is_some());
    }

    #[test]
    fn no_workers_yields_no_selection_and_no_panic() {
        let reg = registry();
        let explanation = schedule(&request(), &reg, &[], &SchedulingOverrides::default());
        assert!(explanation.selected.is_none());
        assert!(explanation.all_candidates.is_empty());
    }

    #[test]
    fn benchmark_data_for_the_candidate_model_raises_its_score() {
        let reg = registry();
        let mut w1 = worker("w1");
        w1.benchmarks.insert(
            ModelId::from("big"),
            BenchmarkSummary {
                prompt_tokens_per_second: Some(400.0),
                generation_tokens_per_second: Some(200.0),
                p50_ttft_ms: None,
            },
        );
        let workers = vec![w1, worker("w2")]; // w2 has identical eligibility, no benchmark data
        let explanation = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());

        let w1_big = explanation
            .all_candidates
            .iter()
            .find(|c| c.worker_id == WorkerId::from("w1") && c.model_id == ModelId::from("big"))
            .unwrap();
        let w2_big = explanation
            .all_candidates
            .iter()
            .find(|c| c.worker_id == WorkerId::from("w2") && c.model_id == ModelId::from("big"))
            .unwrap();
        assert!(
            w1_big.score > w2_big.score,
            "benchmarked worker should outscore an identical worker with no benchmark data"
        );
        assert!(w1_big
            .score_breakdown
            .iter()
            .any(|c| c.label == "measured generation throughput"));
    }

    #[test]
    fn benchmark_data_does_not_leak_into_a_different_models_score() {
        let reg = registry();
        let mut w1 = worker("w1");
        // Benchmark data recorded for "small" must not affect scoring "big" on the
        // same worker - this is exactly the per-model scoping the fingerprint/summary
        // map exists to guarantee.
        w1.benchmarks.insert(
            ModelId::from("small"),
            BenchmarkSummary {
                prompt_tokens_per_second: Some(9000.0),
                generation_tokens_per_second: Some(9000.0),
                p50_ttft_ms: None,
            },
        );
        let workers = vec![w1];
        let explanation = schedule(&request(), &reg, &workers, &SchedulingOverrides::default());
        let big_candidate = explanation
            .all_candidates
            .iter()
            .find(|c| c.model_id == ModelId::from("big"))
            .unwrap();
        assert!(!big_candidate
            .score_breakdown
            .iter()
            .any(|c| c.label.contains("throughput")));
    }
}
