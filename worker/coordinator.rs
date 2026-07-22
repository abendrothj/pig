//! Coordinator-side: talks to one or more configured workers over HTTP, runs the
//! scheduler to pick a placement, and implements `core::model::ModelInvoker` so the
//! workflow engine (synchronous, no async runtime) can invoke a model with a plain
//! function call. Uses `reqwest::blocking` specifically so this stays synchronous —
//! no nested tokio runtime, no need for the workflow engine to know anything async
//! exists.

use lao_orchestrator_core::model::{
    latest_matching_benchmark, schedule, BenchmarkFingerprint, BenchmarkSummary, FinishReason,
    ModelExecutionError, ModelExecutionMetadata, ModelId, ModelInvoker, ModelRegistry,
    ModelRequest, ModelResponse, ModelResponseStatus, ModelUsage, RoutingExplanation,
    SchedulingOverrides, WorkerId, WorkerLocality, WorkerSnapshot,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerEndpointConfig {
    pub id: String,
    pub url: String,
    pub auth_token_env: Option<String>,
    #[serde(default)]
    pub priority: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkersConfig {
    #[serde(default)]
    pub workers: Vec<WorkerEndpointConfig>,
}

impl WorkersConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
        Self::from_toml_str(&text)
    }

    pub fn from_toml_str(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("invalid TOML: {}", e))
    }
}

fn is_loopback_or_private(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.host_str() {
        Some(host) => {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .map(|ip| match ip {
                        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
                        std::net::IpAddr::V6(v6) => v6.is_loopback(),
                    })
                    .unwrap_or(false)
        }
        None => false,
    }
}

pub struct Coordinator {
    workers: Vec<WorkerEndpointConfig>,
    registry: ModelRegistry,
    client: reqwest::blocking::Client,
}

impl Coordinator {
    pub fn new(workers: Vec<WorkerEndpointConfig>, registry: ModelRegistry) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(600))
            .build()
            .expect("reqwest client builds with static config");
        Self {
            workers,
            registry,
            client,
        }
    }

    fn auth_header(&self, worker: &WorkerEndpointConfig) -> Option<String> {
        let var = worker.auth_token_env.as_ref()?;
        std::env::var(var).ok()
    }

    /// Health + capability probe. A worker that can't be reached still gets a
    /// snapshot (marked unhealthy) rather than being silently dropped, so the
    /// coordinator keeps operating and the routing explanation shows *why* that
    /// worker was rejected instead of just omitting it.
    fn snapshot(&self, worker: &WorkerEndpointConfig) -> WorkerSnapshot {
        let unhealthy = |_reason: &str| WorkerSnapshot {
            worker_id: WorkerId::from(worker.id.clone()),
            healthy: false,
            backend: "unknown".to_string(),
            backend_version: None,
            backend_healthy: false,
            worker_hardware_fingerprint: None,
            accelerators: vec![],
            loaded_models: vec![],
            known_models: vec![],
            queue_depth: 0,
            active_jobs: 0,
            max_queued_jobs: 0,
            available_memory_bytes: None,
            supports_streaming: false,
            locality: WorkerLocality::Remote,
            benchmarks: BTreeMap::new(),
            priority: worker.priority,
        };

        let mut request = self.client.get(format!("{}/v1/health", worker.url));
        if let Some(token) = self.auth_header(worker) {
            request = request.bearer_auth(token);
        }
        let health: serde_json::Value = match request.send().and_then(|r| r.error_for_status()) {
            Ok(resp) => resp.json().unwrap_or(serde_json::Value::Null),
            Err(_) => return unhealthy("health check failed"),
        };
        let healthy = health.get("status").and_then(|s| s.as_str()) == Some("ok");
        let active_jobs = health
            .get("active_jobs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let queue_depth = health
            .get("queued_jobs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let mut caps_request = self.client.get(format!("{}/v1/capabilities", worker.url));
        if let Some(token) = self.auth_header(worker) {
            caps_request = caps_request.bearer_auth(token);
        }
        let caps: serde_json::Value = match caps_request.send().and_then(|r| r.error_for_status()) {
            Ok(resp) => resp.json().unwrap_or(serde_json::Value::Null),
            Err(_) => return unhealthy("capabilities check failed"),
        };

        let backend_obj = caps
            .get("backend")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let backend_healthy = !backend_obj.is_null();
        let backend = backend_obj
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let backend_version = backend_obj
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let accelerators: Vec<lao_orchestrator_core::model::AcceleratorKind> = backend_obj
            .get("accelerators")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(parse_accelerator)
                    .collect()
            })
            .unwrap_or_default();
        let supports_streaming = backend_obj
            .get("supports_streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_queued_jobs = caps
            .get("max_queued_jobs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let known_models: Vec<ModelId> = caps
            .get("known_models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(ModelId::from)
                    .collect()
            })
            .unwrap_or_default();

        // Coarse but real: built the same way at benchmark-record time (see
        // cli::model_commands::models_benchmark), from data the worker itself reports
        // rather than a new hardware-probing mechanism.
        let hardware_fp = caps.get("hardware").map(|hw| {
            lao_orchestrator_core::model::worker_hardware_fingerprint(
                hw.get("os").and_then(|v| v.as_str()).unwrap_or("unknown"),
                hw.get("arch").and_then(|v| v.as_str()).unwrap_or("unknown"),
                hw.get("hostname")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown"),
            )
        });

        let locality = if is_loopback_or_private(&worker.url) {
            WorkerLocality::Local
        } else {
            WorkerLocality::Remote
        };

        // Only ever fingerprint-matched (current) history reaches the scheduler - a
        // benchmark recorded under different conditions (model file changed, backend
        // upgraded, different worker hardware, different context/accelerator) is
        // simply absent here, not present-but-wrong. See BenchmarkFingerprint::matches.
        let worker_id = WorkerId::from(worker.id.clone());
        let benchmarks: BTreeMap<ModelId, BenchmarkSummary> = known_models
            .iter()
            .filter_map(|model_id| {
                let entry = self.registry.get(model_id)?;
                let current_fp = BenchmarkFingerprint {
                    model_id: model_id.clone(),
                    model_file_size_bytes: std::fs::metadata(&entry.path).ok().map(|m| m.len()),
                    model_file_hash: None,
                    backend: backend.clone(),
                    backend_version: backend_version.clone(),
                    worker_hardware_fingerprint: hardware_fp.clone()?,
                    accelerator: accelerators.first().copied(),
                    context_tokens: entry.context_tokens,
                    gpu_layers: None,
                    cpu_threads: None,
                    batch_size: None,
                };
                let record = latest_matching_benchmark(model_id, &worker_id, &current_fp)?;
                Some((
                    model_id.clone(),
                    BenchmarkSummary {
                        prompt_tokens_per_second: record.prompt_tokens_per_second,
                        generation_tokens_per_second: record.generation_tokens_per_second,
                    },
                ))
            })
            .collect();

        WorkerSnapshot {
            worker_id,
            healthy,
            backend,
            backend_version,
            backend_healthy,
            worker_hardware_fingerprint: hardware_fp,
            accelerators,
            loaded_models: vec![], // not currently exposed distinctly from known_models over HTTP
            known_models,
            queue_depth,
            active_jobs,
            max_queued_jobs,
            available_memory_bytes: None,
            supports_streaming,
            locality,
            benchmarks,
            priority: worker.priority,
        }
    }

    /// Health/capability snapshot of every configured worker, independent of any
    /// specific request - used by `workers list`/`workers health`.
    pub fn snapshots(&self) -> Vec<WorkerSnapshot> {
        self.workers.iter().map(|w| self.snapshot(w)).collect()
    }

    pub fn route(
        &self,
        request: &ModelRequest,
        overrides: &SchedulingOverrides,
    ) -> RoutingExplanation {
        let snapshots = self.snapshots();
        schedule(request, &self.registry, &snapshots, overrides)
    }

    fn generate_on(
        &self,
        worker: &WorkerEndpointConfig,
        request: &ModelRequest,
    ) -> Result<ModelResponse, String> {
        let mut body = serde_json::to_value(request).map_err(|e| e.to_string())?;
        if let Some(map) = body.as_object_mut() {
            map.insert("stream".to_string(), serde_json::Value::Bool(false));
        }
        let mut http_request = self
            .client
            .post(format!("{}/v1/generate", worker.url))
            .json(&body);
        if let Some(token) = self.auth_header(worker) {
            http_request = http_request.bearer_auth(token);
        }
        let response = http_request.send().map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("worker returned {}", response.status()));
        }
        response.json::<ModelResponse>().map_err(|e| e.to_string())
    }
}

impl ModelInvoker for Coordinator {
    fn invoke(&self, request: ModelRequest) -> ModelResponse {
        let explanation = self.route(&request, &SchedulingOverrides::default());
        let Some(placement) = explanation.selected else {
            return failure_response(
                &request,
                ModelExecutionError::NoEligibleWorker {
                    reason: explanation.to_string(),
                },
            );
        };
        let Some(worker_cfg) = self.workers.iter().find(|w| w.id == placement.worker_id.0) else {
            return failure_response(
                &request,
                ModelExecutionError::WorkerUnavailable {
                    worker: placement.worker_id.0.clone(),
                },
            );
        };
        match self.generate_on(worker_cfg, &request) {
            Ok(response) => response,
            Err(message) => failure_response(
                &request,
                ModelExecutionError::WorkerUnavailable {
                    worker: format!("{}: {}", worker_cfg.id, message),
                },
            ),
        }
    }
}

fn parse_accelerator(s: &str) -> Option<lao_orchestrator_core::model::AcceleratorKind> {
    use lao_orchestrator_core::model::AcceleratorKind::*;
    match s {
        "cuda" => Some(Cuda),
        "metal" => Some(Metal),
        "vulkan" => Some(Vulkan),
        "rocm" => Some(Rocm),
        "cpu" => Some(Cpu),
        _ => None,
    }
}

fn failure_response(request: &ModelRequest, error: ModelExecutionError) -> ModelResponse {
    ModelResponse {
        request_id: request.request_id.clone(),
        status: ModelResponseStatus::Failed,
        output: lao_orchestrator_core::execution::Artifact::Null,
        finish_reason: FinishReason::Error,
        model: lao_orchestrator_core::model::ResolvedModel {
            model_id: ModelId::from("unresolved"),
            role: Some(request.role.clone()),
            backend: "none".to_string(),
            identity: "none".to_string(),
        },
        execution: ModelExecutionMetadata {
            worker_id: WorkerId::from("none"),
            host_name: "coordinator".to_string(),
            backend: "none".to_string(),
            backend_version: None,
            model_id: ModelId::from("unresolved"),
            model_identity: "none".to_string(),
            accelerator: None,
            cpu_threads: None,
            gpu_layers: None,
            context_tokens: None,
            batch_size: None,
            queue_wait_ms: 0,
            model_load_ms: 0,
            prompt_eval_ms: 0,
            generation_ms: 0,
            total_ms: 0,
            prompt_tokens: 0,
            generated_tokens: 0,
            prompt_tokens_per_second: None,
            generation_tokens_per_second: None,
            model_already_loaded: false,
            cancellation: None,
            peak_memory_bytes: None,
            peak_vram_bytes: None,
        },
        usage: ModelUsage::default(),
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_config_parses_the_example() {
        let toml = r#"
[[workers]]
id = "macbook-worker"
url = "http://127.0.0.1:9847"

[[workers]]
id = "linux-worker"
url = "http://100.64.0.5:9847"
auth_token_env = "LAO_LINUX_WORKER_TOKEN"
"#;
        let cfg = WorkersConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.workers.len(), 2);
        assert_eq!(cfg.workers[0].id, "macbook-worker");
        assert_eq!(
            cfg.workers[1].auth_token_env.as_deref(),
            Some("LAO_LINUX_WORKER_TOKEN")
        );
    }

    #[test]
    fn loopback_and_private_urls_are_local() {
        assert!(is_loopback_or_private("http://127.0.0.1:9847"));
        assert!(is_loopback_or_private("http://localhost:9847"));
        assert!(is_loopback_or_private("http://192.168.1.5:9847"));
    }

    #[test]
    fn public_urls_are_not_local() {
        assert!(!is_loopback_or_private("http://1.2.3.4:9847"));
    }

    #[test]
    fn unreachable_worker_yields_a_failed_but_structured_response() {
        let registry = ModelRegistry::default();
        let coordinator = Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "ghost".to_string(),
                url: "http://127.0.0.1:1".to_string(), // nothing listens here
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        );
        let request = ModelRequest {
            request_id: lao_orchestrator_core::model::RequestId::generate(),
            role: lao_orchestrator_core::model::ModelRole::Reasoning,
            model: None,
            messages: vec![lao_orchestrator_core::model::ModelMessage::user("hi")],
            parameters: Default::default(),
            requirements: Default::default(),
            inputs: vec![],
            metadata: Default::default(),
        };
        let response = coordinator.invoke(request);
        assert_eq!(response.status, ModelResponseStatus::Failed);
        assert!(matches!(
            response.error,
            Some(ModelExecutionError::NoEligibleWorker { .. })
        ));
    }
}
