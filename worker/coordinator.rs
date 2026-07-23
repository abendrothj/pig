//! Coordinator-side: talks to one or more configured workers over HTTP, runs the
//! scheduler to pick a placement, and implements `core::model::ModelInvoker` so the
//! workflow engine (synchronous, no async runtime) can invoke a model with a plain
//! function call. Uses `reqwest::blocking` specifically so this stays synchronous —
//! no nested tokio runtime, no need for the workflow engine to know anything async
//! exists.

use crate::defer_drop::DeferDrop;
use futures::{Stream, StreamExt};
use pig_core::model::{
    latest_matching_benchmark, schedule, BenchmarkFingerprint, BenchmarkSummary, FinishReason,
    ModelChunk, ModelExecutionError, ModelExecutionMetadata, ModelId, ModelInstance, ModelInvoker,
    ModelRegistry, ModelRequest, ModelResponse, ModelResponseStatus, ModelRole, ModelUsage,
    ReasoningMode, RoutingExplanation, SchedulingOverrides, WorkerId, WorkerLocality,
    WorkerSnapshot,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

pub type ModelChunkStream = Pin<Box<dyn Stream<Item = Result<ModelChunk, String>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorStreamError {
    NoEligibleWorker(String),
    WorkerUnavailable(String),
    ToolsUnsupported,
}

impl std::fmt::Display for CoordinatorStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEligibleWorker(reason) => write!(f, "no eligible worker: {}", reason),
            Self::WorkerUnavailable(reason) => write!(f, "worker unavailable: {}", reason),
            Self::ToolsUnsupported => write!(
                f,
                "the selected pig worker cannot honor requested tool calling"
            ),
        }
    }
}

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
    client: DeferDrop<reqwest::blocking::Client>,
    async_client: reqwest::Client,
}

impl Coordinator {
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    pub fn new(workers: Vec<WorkerEndpointConfig>, registry: ModelRegistry) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(600))
            .build()
            .expect("reqwest client builds with static config");
        Self {
            workers,
            registry,
            client: DeferDrop::new(client),
            async_client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(600))
                .build()
                .expect("reqwest async client builds with static config"),
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
        let mut health_req = self.client.get(format!("{}/v1/health", worker.url));
        if let Some(token) = self.auth_header(worker) {
            health_req = health_req.bearer_auth(token);
        }
        let health: serde_json::Value = match health_req.send().and_then(|r| r.error_for_status()) {
            Ok(resp) => resp.json().unwrap_or(serde_json::Value::Null),
            Err(_) => return self.unhealthy_snapshot(worker),
        };
        let mut caps_req = self.client.get(format!("{}/v1/capabilities", worker.url));
        if let Some(token) = self.auth_header(worker) {
            caps_req = caps_req.bearer_auth(token);
        }
        let caps: serde_json::Value = match caps_req.send().and_then(|r| r.error_for_status()) {
            Ok(resp) => resp.json().unwrap_or(serde_json::Value::Null),
            Err(_) => return self.unhealthy_snapshot(worker),
        };
        self.parse_snapshot(worker, &health, &caps)
    }

    /// Health/capability snapshot of every configured worker, independent of any
    /// specific request - used by `workers list`/`workers health`.
    pub fn snapshots(&self) -> Vec<WorkerSnapshot> {
        self.workers.iter().map(|w| self.snapshot(w)).collect()
    }

    /// All model instances across all configured workers, from most to least desirable
    /// (loaded first, then by benchmark score). This is the coordinator-level view of
    /// available inference capacity — two workers running the same model are distinct
    /// instances with potentially very different performance characteristics.
    pub async fn model_instances(&self) -> Vec<ModelInstance> {
        let snapshots = self.async_snapshots().await;
        let mut instances: Vec<ModelInstance> = snapshots
            .iter()
            .flat_map(|snapshot| {
                snapshot.known_models.iter().map(|model_id| {
                    let loaded = snapshot.loaded_models.contains(model_id);
                    let context_tokens = self.registry.get(model_id).and_then(|e| e.context_tokens);
                    ModelInstance {
                        instance_id: format!("{}/{}", snapshot.worker_id.0, model_id.0),
                        worker_id: snapshot.worker_id.clone(),
                        model_id: model_id.clone(),
                        backend: snapshot.backend.clone(),
                        loaded,
                        context_tokens,
                        accelerators: snapshot.accelerators.clone(),
                        benchmark: snapshot.benchmarks.get(model_id).cloned(),
                    }
                })
            })
            .collect();
        // Loaded instances first, then by generation throughput descending.
        instances.sort_by(|a, b| {
            b.loaded.cmp(&a.loaded).then_with(|| {
                let tps_b = b
                    .benchmark
                    .as_ref()
                    .and_then(|bm| bm.generation_tokens_per_second)
                    .unwrap_or(0.0);
                let tps_a = a
                    .benchmark
                    .as_ref()
                    .and_then(|bm| bm.generation_tokens_per_second)
                    .unwrap_or(0.0);
                tps_b
                    .partial_cmp(&tps_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        instances
    }

    /// Async-native discovery for the long-running server. The embedded workflow
    /// path deliberately remains synchronous because it implements ModelInvoker.
    /// Scheduling itself remains the pure `schedule(&[WorkerSnapshot])` function.
    pub async fn async_snapshots(&self) -> Vec<WorkerSnapshot> {
        let mut snapshots = Vec::with_capacity(self.workers.len());
        for worker in &self.workers {
            snapshots.push(self.async_snapshot(worker).await);
        }
        snapshots
    }

    async fn async_snapshot(&self, worker: &WorkerEndpointConfig) -> WorkerSnapshot {
        let mut health_req = self.async_client.get(format!("{}/v1/health", worker.url));
        if let Some(token) = self.auth_header(worker) {
            health_req = health_req.bearer_auth(token);
        }
        let health: serde_json::Value =
            match health_req.send().await.and_then(|r| r.error_for_status()) {
                Ok(resp) => match resp.json().await {
                    Ok(v) => v,
                    Err(_) => return self.unhealthy_snapshot(worker),
                },
                Err(_) => return self.unhealthy_snapshot(worker),
            };
        let mut caps_req = self
            .async_client
            .get(format!("{}/v1/capabilities", worker.url));
        if let Some(token) = self.auth_header(worker) {
            caps_req = caps_req.bearer_auth(token);
        }
        let caps: serde_json::Value = match caps_req.send().await.and_then(|r| r.error_for_status())
        {
            Ok(resp) => match resp.json().await {
                Ok(v) => v,
                Err(_) => return self.unhealthy_snapshot(worker),
            },
            Err(_) => return self.unhealthy_snapshot(worker),
        };
        self.parse_snapshot(worker, &health, &caps)
    }

    fn unhealthy_snapshot(&self, worker: &WorkerEndpointConfig) -> WorkerSnapshot {
        WorkerSnapshot {
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
            supports_tools: false,
            locality: WorkerLocality::Remote,
            benchmarks: BTreeMap::new(),
            priority: worker.priority,
        }
    }

    /// Shared JSON → WorkerSnapshot parsing used by both the sync and async probe paths.
    fn parse_snapshot(
        &self,
        worker: &WorkerEndpointConfig,
        health: &serde_json::Value,
        caps: &serde_json::Value,
    ) -> WorkerSnapshot {
        let backend_obj = caps
            .get("backend")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let backend = backend_obj
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let backend_version = backend_obj
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let accelerators: Vec<pig_core::model::AcceleratorKind> = backend_obj
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
        let supports_tools = backend_obj
            .get("supports_tools")
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
        let loaded_models: Vec<ModelId> = caps
            .get("loaded_models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(ModelId::from)
                    .collect()
            })
            .unwrap_or_default();
        let available_memory_bytes = caps.get("available_memory_bytes").and_then(|v| v.as_u64());

        // Coarse but real: built the same way at benchmark-record time (see
        // cli::model_commands::models_benchmark), from data the worker itself reports
        // rather than a new hardware-probing mechanism.
        let hardware_fp = caps.get("hardware").map(|hw| {
            pig_core::model::worker_hardware_fingerprint(
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

        // Only ever fingerprint-matched (current) history reaches the scheduler — a
        // benchmark recorded under different conditions is absent here, not present-but-wrong.
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
                        p50_ttft_ms: record.p50_ttft_ms,
                    },
                ))
            })
            .collect();

        WorkerSnapshot {
            worker_id,
            healthy: health.get("status").and_then(|v| v.as_str()) == Some("ok"),
            backend,
            backend_version,
            backend_healthy: !backend_obj.is_null(),
            worker_hardware_fingerprint: hardware_fp,
            accelerators,
            loaded_models,
            known_models,
            queue_depth: health
                .get("queued_jobs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            active_jobs: health
                .get("active_jobs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            max_queued_jobs,
            available_memory_bytes,
            supports_streaming,
            supports_tools,
            locality,
            benchmarks,
            priority: worker.priority,
        }
    }

    pub fn route(
        &self,
        request: &ModelRequest,
        overrides: &SchedulingOverrides,
    ) -> RoutingExplanation {
        let snapshots = self.snapshots();
        schedule(request, &self.registry, &snapshots, overrides)
    }

    /// Whether the placement selected for `request` can honor tool definitions.
    /// Model-level `tool_calling` in the registry takes precedence over the backend's
    /// blanket capability — a model knows its own tool support better than its backend.
    pub fn selected_worker_supports_tools(&self, request: &ModelRequest) -> bool {
        let snapshots = self.snapshots();
        let route = schedule(
            request,
            &self.registry,
            &snapshots,
            &SchedulingOverrides::default(),
        );
        let Some(placement) = route.selected else {
            return false;
        };
        // Model-level override wins.
        if let Some(model_override) = self
            .registry
            .get(&placement.model_id)
            .and_then(|e| e.tool_calling)
        {
            return model_override;
        }
        // Fall back to backend capability.
        snapshots
            .iter()
            .find(|s| s.worker_id == placement.worker_id)
            .map(|s| s.supports_tools)
            .unwrap_or(false)
    }

    /// Route and relay a worker's canonical stream without translating it into a
    /// client protocol. Dropping the returned stream drops the upstream HTTP body,
    /// which lets reqwest close the worker connection instead of buffering output.
    pub async fn stream(
        &self,
        mut request: ModelRequest,
    ) -> Result<ModelChunkStream, CoordinatorStreamError> {
        let snapshots = self.async_snapshots().await;
        let explanation = schedule(
            &request,
            &self.registry,
            &snapshots,
            &SchedulingOverrides::default(),
        );
        let placement = explanation
            .selected
            .clone()
            .ok_or_else(|| CoordinatorStreamError::NoEligibleWorker(explanation.to_string()))?;
        let worker = self
            .workers
            .iter()
            .find(|worker| worker.id == placement.worker_id.0)
            .ok_or_else(|| {
                CoordinatorStreamError::WorkerUnavailable(placement.worker_id.0.clone())
            })?;
        let snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.worker_id == placement.worker_id)
            .ok_or_else(|| {
                CoordinatorStreamError::WorkerUnavailable(placement.worker_id.0.clone())
            })?;
        // Model-level override takes precedence over the backend's blanket capability.
        let model_supports_tools = self
            .registry
            .get(&placement.model_id)
            .and_then(|e| e.tool_calling)
            .unwrap_or(snapshot.supports_tools);
        if !request.parameters.tools.is_empty() && !model_supports_tools {
            return Err(CoordinatorStreamError::ToolsUnsupported);
        }

        request.parameters.reasoning_mode = resolve_reasoning_mode(&request);

        let mut body = serde_json::to_value(&request)
            .map_err(|error| CoordinatorStreamError::WorkerUnavailable(error.to_string()))?;
        if let Some(map) = body.as_object_mut() {
            map.insert("stream".to_string(), serde_json::Value::Bool(true));
        }
        let mut http_request = self
            .async_client
            .post(format!("{}/v1/generate", worker.url.trim_end_matches('/')))
            .json(&body);
        if let Some(token) = self.auth_header(worker) {
            http_request = http_request.bearer_auth(token);
        }
        let response = http_request
            .send()
            .await
            .map_err(|error| CoordinatorStreamError::WorkerUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(CoordinatorStreamError::WorkerUnavailable(format!(
                "worker returned {}",
                response.status()
            )));
        }
        Ok(worker_sse_chunks(response))
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

    /// Fetches one worker's current metrics snapshot on demand. Purely an
    /// observability read, not part of routing - `pig workers metrics` is the
    /// only caller today.
    pub fn worker_metrics(
        &self,
        worker_id: &str,
    ) -> Result<pig_core::model::WorkerMetricsSnapshot, String> {
        let worker = self
            .workers
            .iter()
            .find(|w| w.id == worker_id)
            .ok_or_else(|| format!("unknown worker '{}'", worker_id))?;
        let mut http_request = self.client.get(format!("{}/v1/metrics", worker.url));
        if let Some(token) = self.auth_header(worker) {
            http_request = http_request.bearer_auth(token);
        }
        let response = http_request.send().map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("worker returned {}", response.status()));
        }
        response
            .json::<pig_core::model::WorkerMetricsSnapshot>()
            .map_err(|e| e.to_string())
    }

    /// Coordinator-owned proxy for worker job operations. It keeps remote CLI
    /// profiles from needing worker URLs or worker bearer tokens locally.
    pub fn job_request(
        &self,
        worker_id: &str,
        method: reqwest::Method,
        path: &str,
    ) -> Result<serde_json::Value, String> {
        let worker = self
            .workers
            .iter()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| format!("unknown worker '{}'", worker_id))?;
        let mut request = self.client.request(
            method,
            format!("{}/{}", worker.url.trim_end_matches('/'), path),
        );
        if let Some(token) = self.auth_header(worker) {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("worker returned {}", response.status()));
        }
        response.json().map_err(|e| e.to_string())
    }
}

fn worker_sse_chunks(response: reqwest::Response) -> ModelChunkStream {
    Box::pin(futures::stream::unfold(
        (response.bytes_stream(), String::new(), false),
        |(mut bytes, mut buffer, done)| async move {
            if done {
                return None;
            }
            loop {
                if let Some(end) = buffer.find("\n\n") {
                    let frame = buffer[..end].to_string();
                    buffer.drain(..end + 2);
                    if let Some(item) = parse_worker_sse_frame(&frame) {
                        return Some((item, (bytes, buffer, false)));
                    }
                    continue;
                }

                match bytes.next().await {
                    Some(Ok(chunk)) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
                    Some(Err(error)) => {
                        return Some((
                            Err(format!("worker stream failed: {}", error)),
                            (bytes, buffer, true),
                        ));
                    }
                    None => return None,
                }
            }
        },
    ))
}

fn parse_worker_sse_frame(frame: &str) -> Option<Result<ModelChunk, String>> {
    let mut event = None;
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(name) = line.strip_prefix("event: ") {
            event = Some(name);
        } else if let Some(payload) = line.strip_prefix("data: ") {
            data.push_str(payload);
        }
    }
    (event == Some("chunk")).then(|| {
        serde_json::from_str::<ModelChunk>(&data)
            .map_err(|error| format!("invalid worker stream chunk: {}", error))
    })
}

impl Coordinator {
    /// Same as `invoke` but accepts explicit scheduling overrides — used by the
    /// pipeline handler to implement session affinity (pin subsequent steps to
    /// the same worker that served the first step).
    pub fn invoke_with_overrides(
        &self,
        mut request: ModelRequest,
        overrides: SchedulingOverrides,
    ) -> ModelResponse {
        let explanation = self.route(&request, &overrides);
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
        request.parameters.reasoning_mode = resolve_reasoning_mode(&request);
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

impl ModelInvoker for Coordinator {
    fn invoke(&self, mut request: ModelRequest) -> ModelResponse {
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
        request.parameters.reasoning_mode = resolve_reasoning_mode(&request);
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

fn parse_accelerator(s: &str) -> Option<pig_core::model::AcceleratorKind> {
    use pig_core::model::AcceleratorKind::*;
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
        output: pig_core::artifact::Artifact::Null,
        finish_reason: FinishReason::Error,
        model: pig_core::model::ResolvedModel {
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
        tool_calls: vec![],
        error: Some(error),
    }
}

/// Resolves `ReasoningMode::Auto` to an explicit directive before the request
/// reaches the backend. The backend translates the resolved mode to a control
/// token; it never makes policy decisions.
///
/// Policy: when the caller explicitly sets a mode, honour it. Otherwise leave
/// reasoning off. The caller (CLI `--reasoning-mode`, pipeline step config, or
/// an OpenAI-compatible client that sends `reasoning_mode`) is in the best
/// position to decide — auto-enabling by role surprised callers that manage
/// their own reasoning loops and don't expect the gateway to inject /think tokens.
fn resolve_reasoning_mode(request: &ModelRequest) -> ReasoningMode {
    if request.parameters.reasoning_mode != ReasoningMode::Auto {
        return request.parameters.reasoning_mode;
    }
    ReasoningMode::Disabled
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
auth_token_env = "PIG_LINUX_WORKER_TOKEN"
"#;
        let cfg = WorkersConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.workers.len(), 2);
        assert_eq!(cfg.workers[0].id, "macbook-worker");
        assert_eq!(
            cfg.workers[1].auth_token_env.as_deref(),
            Some("PIG_LINUX_WORKER_TOKEN")
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
            request_id: pig_core::model::RequestId::generate(),
            role: pig_core::model::ModelRole::Reasoning,
            model: None,
            messages: vec![pig_core::model::ModelMessage::user("hi")],
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

    fn reasoning_request(
        role: ModelRole,
        tools: Vec<serde_json::Value>,
        mode: ReasoningMode,
    ) -> ModelRequest {
        use pig_core::model::{GenerationParameters, ModelMessage, ModelRequirements, RequestId};
        ModelRequest {
            request_id: RequestId::generate(),
            role,
            model: None,
            messages: vec![ModelMessage::user("hi")],
            parameters: GenerationParameters {
                reasoning_mode: mode,
                tools,
                ..Default::default()
            },
            requirements: ModelRequirements::default(),
            inputs: vec![],
            metadata: Default::default(),
        }
    }

    #[test]
    fn resolve_auto_reasoning_defaults_to_disabled_for_all_roles() {
        // Auto no longer enables reasoning by role — the caller decides.
        let r = reasoning_request(ModelRole::Reasoning, vec![], ReasoningMode::Auto);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Disabled);
        let r = reasoning_request(ModelRole::Coding, vec![], ReasoningMode::Auto);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Disabled);
        let tool = serde_json::json!({"type": "function", "function": {"name": "search", "parameters": {}}});
        let r = reasoning_request(ModelRole::Summarization, vec![tool], ReasoningMode::Auto);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Disabled);
        let r = reasoning_request(ModelRole::Summarization, vec![], ReasoningMode::Auto);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Disabled);
    }

    #[test]
    fn resolve_reasoning_pass_through_explicit_modes() {
        let r = reasoning_request(ModelRole::Reasoning, vec![], ReasoningMode::Disabled);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Disabled);
        let r = reasoning_request(ModelRole::Summarization, vec![], ReasoningMode::Enabled);
        assert_eq!(resolve_reasoning_mode(&r), ReasoningMode::Enabled);
    }

    #[test]
    fn worker_sse_frames_decode_to_canonical_chunks() {
        let frame = concat!(
            "event: chunk\n",
            "data: {\"type\":\"tool_call_delta\",\"index\":1,\"id\":\"call_1\",\"function_name\":null,\"arguments_delta\":\"{\\\"path\\\":\"}\n"
        );
        assert_eq!(
            parse_worker_sse_frame(frame),
            Some(Ok(ModelChunk::ToolCallDelta {
                index: 1,
                id: Some("call_1".to_string()),
                function_name: None,
                arguments_delta: Some(r#"{"path":"#.to_string()),
            }))
        );
        assert!(parse_worker_sse_frame("event: response\ndata: {}\n").is_none());
    }
}
