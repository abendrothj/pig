//! Handlers for `worker`, `workers`, `models`, `route`, and `jobs` — the v0.5
//! local-inference CLI surface. Config (model registry + worker list) is read from
//! the same `lao.toml` resolution `TrustPolicy` already uses (`LAO_CONFIG`, else
//! `lao.toml`, else `config/lao.toml`), rather than inventing a parallel convention.

use lao_orchestrator_core::model::{
    record_benchmark, BenchmarkFingerprint, BenchmarkRecord, GenerationParameters, MessageRole,
    ModelId, ModelInvoker, ModelMessage, ModelRegistry, ModelRequest, ModelRequirements, ModelRole,
    ModelSelector, RequestId, SchedulingOverrides,
};
use lao_worker::coordinator::{Coordinator, WorkerEndpointConfig, WorkersConfig};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Instant;

fn config_path() -> Option<String> {
    if let Ok(p) = std::env::var("LAO_CONFIG") {
        return Some(p);
    }
    for candidate in ["lao.toml", "config/lao.toml"] {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn config_text() -> String {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

fn load_registry() -> ModelRegistry {
    ModelRegistry::from_toml_str(&config_text()).unwrap_or_default()
}

fn load_workers() -> Vec<WorkerEndpointConfig> {
    WorkersConfig::from_toml_str(&config_text())
        .map(|c| c.workers)
        .unwrap_or_default()
}

fn require_workers() -> Vec<WorkerEndpointConfig> {
    let workers = load_workers();
    if workers.is_empty() {
        eprintln!("[ERROR] no [[workers]] configured in lao.toml (or LAO_CONFIG)");
        std::process::exit(1);
    }
    workers
}

/// Builds a `ModelInvoker` from `[[workers]]` config when any are configured, so
/// `lao-cli run` can execute `local_llm` steps. Returns `None` (not an error) when no
/// workers are configured - workflows without any `local_llm` step are completely
/// unaffected, and one with a `local_llm` step will fail with a clear per-step error
/// from `StepExecutor` rather than this function guessing at a default.
pub fn build_model_invoker() -> Option<std::sync::Arc<dyn ModelInvoker>> {
    let workers = load_workers();
    if workers.is_empty() {
        return None;
    }
    Some(std::sync::Arc::new(Coordinator::new(
        workers,
        load_registry(),
    )))
}

fn require_model_inference_trust() {
    let trust = lao_orchestrator_core::trust::TrustPolicy::load_default();
    if !trust.allows_class(lao_orchestrator_core::trust::CapabilityClass::ModelInference) {
        eprintln!("[ERROR] requires trust.allow_model_inference = true in lao.toml");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// worker serve
// ---------------------------------------------------------------------------

pub fn worker_serve(config: Option<String>) {
    let path = config.unwrap_or_else(|| "lao.toml".to_string());
    let worker_config = match lao_worker::config::WorkerConfig::load(std::path::Path::new(&path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    };
    let registry_text = std::fs::read_to_string(&path).unwrap_or_default();
    let registry = ModelRegistry::from_toml_str(&registry_text).unwrap_or_default();
    let hardware = lao_worker::hardware::discover();
    let host_name = hardware.hostname.clone();

    let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
    rt.block_on(async move {
        let backend: std::sync::Arc<dyn lao_worker::backend::ModelBackend> =
            if worker_config.runtime.llama_cpp.enabled {
                std::sync::Arc::new(lao_worker::backend::llama_cpp::LlamaCppBackend::new(
                    lao_worker::backend::llama_cpp::LlamaCppConfig {
                        server_executable: PathBuf::from(
                            &worker_config.runtime.llama_cpp.server_executable,
                        ),
                        host: "127.0.0.1".to_string(),
                        startup_timeout: worker_config.llama_cpp_startup_timeout(),
                        request_timeout: worker_config.llama_cpp_request_timeout(),
                    },
                ))
            } else {
                std::sync::Arc::new(lao_worker::backend::fake::FakeBackend::new())
            };
        let backend_name = if worker_config.runtime.llama_cpp.enabled {
            "llama_cpp"
        } else {
            "fake"
        }
        .to_string();

        let auth_token = match worker_config.resolve_auth_token() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[ERROR] {}", e);
                std::process::exit(1);
            }
        };

        println!(
            "Starting worker '{}' on {} (backend: {})",
            worker_config.id, worker_config.bind, backend_name
        );

        let runtime = std::sync::Arc::new(lao_worker::job::WorkerRuntime::new(
            worker_config.id.clone(),
            host_name,
            backend,
            backend_name.clone(),
            worker_config.max_concurrent_jobs,
            worker_config.max_queued_jobs,
            worker_config.llama_cpp_request_timeout(),
        ));
        let state = std::sync::Arc::new(lao_worker::state::AppState {
            config: worker_config,
            runtime,
            registry,
            hardware,
            started_at: Instant::now(),
            auth_token,
            backend_name,
        });

        if let Err(e) = lao_worker::server::serve(state).await {
            eprintln!("[ERROR] worker server failed: {}", e);
            std::process::exit(1);
        }
    });
}

// ---------------------------------------------------------------------------
// workers
// ---------------------------------------------------------------------------

pub fn workers_list(json: bool) {
    let workers = load_workers();
    let coordinator = Coordinator::new(workers.clone(), load_registry());
    let snapshots = coordinator.snapshots();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot_json(&snapshots)).unwrap_or_default()
        );
        return;
    }
    if snapshots.is_empty() {
        println!("No workers configured.");
        return;
    }
    for s in &snapshots {
        println!(
            "- {} [{}] backend={} healthy={} queue={}/{} active={}",
            s.worker_id,
            if s.healthy { "up" } else { "down" },
            s.backend,
            s.healthy,
            s.queue_depth,
            s.max_queued_jobs,
            s.active_jobs
        );
    }
}

pub fn workers_health(json: bool) {
    workers_list(json);
    let workers = load_workers();
    let coordinator = Coordinator::new(workers, load_registry());
    if coordinator.snapshots().iter().any(|s| !s.healthy) {
        std::process::exit(1);
    }
}

pub fn workers_inspect(worker_id: String, json: bool) {
    let workers = load_workers();
    let coordinator = Coordinator::new(workers, load_registry());
    let Some(snapshot) = coordinator
        .snapshots()
        .into_iter()
        .find(|s| s.worker_id.0 == worker_id)
    else {
        eprintln!("[ERROR] unknown worker '{}'", worker_id);
        std::process::exit(1);
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot_json(&[snapshot])).unwrap_or_default()
        );
    } else {
        println!("{:#?}", snapshot);
    }
}

fn snapshot_json(snapshots: &[lao_orchestrator_core::model::WorkerSnapshot]) -> serde_json::Value {
    serde_json::json!(snapshots
        .iter()
        .map(|s| serde_json::json!({
            "worker_id": s.worker_id.0,
            "healthy": s.healthy,
            "backend": s.backend,
            "backend_healthy": s.backend_healthy,
            "queue_depth": s.queue_depth,
            "max_queued_jobs": s.max_queued_jobs,
            "active_jobs": s.active_jobs,
            "known_models": s.known_models.iter().map(|m| m.0.clone()).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}

// ---------------------------------------------------------------------------
// models
// ---------------------------------------------------------------------------

pub fn models_list(json: bool) {
    let registry = load_registry();
    let resolved = registry.all_resolved();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&resolved).unwrap_or_default()
        );
        return;
    }
    if resolved.is_empty() {
        println!("No models configured in [models.entries].");
        return;
    }
    for r in resolved {
        println!(
            "- {} [{}] backend={} context={:?} roles={:?}",
            r.entry.id,
            if r.available {
                "available"
            } else {
                "unavailable"
            },
            r.entry.backend,
            r.entry.context_tokens,
            r.entry
                .roles
                .iter()
                .map(|role| role.to_string())
                .collect::<Vec<_>>()
        );
        if let Some(reason) = &r.unavailable_reason {
            println!("    {}", reason);
        }
    }
}

pub fn models_inspect(model_id: String, json: bool) {
    let registry = load_registry();
    let mid = ModelId::from(model_id.clone());
    let Some(resolved) = registry.resolve(&mid) else {
        eprintln!("[ERROR] unknown model id '{}'", model_id);
        std::process::exit(1);
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&resolved).unwrap_or_default()
        );
    } else {
        println!("{:#?}", resolved);
    }
}

pub fn models_discover(directory: String) {
    let found = lao_orchestrator_core::model::discover_gguf_files(std::path::Path::new(&directory));
    if found.is_empty() {
        println!("No .gguf files found under {}.", directory);
        return;
    }
    println!(
        "Found {} GGUF file(s) under {} (not added to config automatically):",
        found.len(),
        directory
    );
    for path in found {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!("- {} ({} bytes)", path.display(), size);
    }
}

pub fn models_load(model_id: String, worker: Option<String>) {
    require_model_inference_trust();
    let workers = require_workers();
    let target = resolve_target_worker(&workers, worker);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{}/v1/models/load", target.url))
        .json(&serde_json::json!({"model_id": model_id}))
        .send();
    match resp {
        Ok(r) if r.status().is_success() => {
            println!("{}", r.text().unwrap_or_default());
        }
        Ok(r) => {
            eprintln!(
                "[ERROR] worker returned {}: {}",
                r.status(),
                r.text().unwrap_or_default()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

pub fn models_unload(model_id: String, worker: Option<String>) {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, worker);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{}/v1/models/unload", target.url))
        .json(&serde_json::json!({"model_id": model_id}))
        .send();
    match resp {
        Ok(r) if r.status().is_success() => println!("Unloaded."),
        Ok(r) => {
            eprintln!("[ERROR] worker returned {}", r.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

fn resolve_target_worker(
    workers: &[WorkerEndpointConfig],
    requested: Option<String>,
) -> WorkerEndpointConfig {
    match requested {
        Some(id) => workers
            .iter()
            .find(|w| w.id == id)
            .cloned()
            .unwrap_or_else(|| {
                eprintln!("[ERROR] unknown worker '{}'", id);
                std::process::exit(1);
            }),
        None => workers[0].clone(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn models_generate(
    role: Option<String>,
    model: Option<String>,
    prompt: String,
    system: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    json: bool,
    stream: bool,
    force_worker: Option<String>,
    force_cpu: bool,
) {
    require_model_inference_trust();
    let workers = require_workers();
    let registry = load_registry();

    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(ModelMessage {
            role: MessageRole::System,
            content: s,
        });
    }
    messages.push(ModelMessage {
        role: MessageRole::User,
        content: prompt,
    });

    let request = ModelRequest {
        request_id: RequestId::generate(),
        role: role
            .as_deref()
            .map(ModelRole::parse)
            .unwrap_or(ModelRole::Reasoning),
        model: model.map(ModelSelector::Alias),
        messages,
        parameters: GenerationParameters {
            max_tokens,
            temperature,
            ..Default::default()
        },
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    };

    if let Err(e) = request.validate() {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }

    let overrides = SchedulingOverrides {
        force_worker: force_worker.map(Into::into),
        force_cpu,
        ..Default::default()
    };

    let coordinator = Coordinator::new(workers.clone(), registry);
    let explanation = coordinator.route(&request, &overrides);
    let Some(placement) = explanation.selected.clone() else {
        eprintln!("[ERROR] no eligible worker:\n{}", explanation);
        std::process::exit(1);
    };
    let target = workers
        .iter()
        .find(|w| w.id == placement.worker_id.0)
        .cloned()
        .unwrap();

    if stream && !json {
        stream_generate(&target, &request);
        return;
    }

    let response = coordinator.invoke(request);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        match &response.output {
            lao_orchestrator_core::execution::Artifact::Text(t) => println!("{}", t),
            other => println!("{:?}", other),
        }
        eprintln!(
            "\n[{}] worker={} model={} tokens={}/{} {:.1} tok/s",
            if response.is_success() {
                "ok"
            } else {
                "failed"
            },
            response.execution.worker_id,
            response.execution.model_id,
            response.execution.prompt_tokens,
            response.execution.generated_tokens,
            response
                .execution
                .generation_tokens_per_second
                .unwrap_or(0.0)
        );
    }
    if !response.is_success() {
        std::process::exit(1);
    }
}

fn stream_generate(target: &WorkerEndpointConfig, request: &ModelRequest) {
    let mut body = serde_json::to_value(request).unwrap_or_default();
    if let Some(map) = body.as_object_mut() {
        map.insert("stream".to_string(), serde_json::Value::Bool(true));
    }
    let client = reqwest::blocking::Client::new();
    let mut req = client
        .post(format!("{}/v1/generate", target.url))
        .json(&body);
    if let Some(var) = &target.auth_token_env {
        if let Ok(token) = std::env::var(var) {
            req = req.bearer_auth(token);
        }
    }
    let response = match req.send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    };
    if !response.status().is_success() {
        eprintln!("[ERROR] worker returned {}", response.status());
        std::process::exit(1);
    }
    // Real SSE wire format (axum's Event::default().event(name).data(payload)): the
    // event name and payload arrive on separate lines within one event block, and the
    // "token" payload is the raw generated text, not a JSON envelope. A blank line
    // ends the block. Only the "response" event's payload is JSON.
    let reader = BufReader::new(response);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut current_event: Option<String> = None;
    for line in reader.lines().map_while(Result::ok) {
        if let Some(name) = line.strip_prefix("event: ") {
            current_event = Some(name.to_string());
            continue;
        }
        if let Some(payload) = line.strip_prefix("data: ") {
            if current_event.as_deref() == Some("token") {
                let _ = write!(out, "{}", payload);
                let _ = out.flush();
            }
            continue;
        }
        if line.is_empty() {
            current_event = None;
        }
    }
    println!();
}

pub fn models_benchmark(model_id: String, worker: Option<String>, json: bool) {
    require_model_inference_trust();
    let workers = require_workers();
    let target = resolve_target_worker(&workers, worker.clone());
    let registry = load_registry();
    let model_id_typed = ModelId::from(model_id.clone());
    let file_size_bytes = registry
        .get(&model_id_typed)
        .and_then(|entry| std::fs::metadata(&entry.path).ok())
        .map(|meta| meta.len());
    let coordinator = Coordinator::new(vec![target.clone()], registry);

    let request = ModelRequest {
        request_id: RequestId::generate(),
        role: ModelRole::Reasoning,
        model: Some(ModelSelector::Id(model_id.clone().into())),
        messages: vec![ModelMessage {
            role: MessageRole::User,
            content: "Reply with a short, one-sentence greeting.".to_string(),
        }],
        parameters: GenerationParameters {
            max_tokens: Some(64),
            temperature: Some(0.0),
            ..Default::default()
        },
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    };

    let response = coordinator.invoke(request);

    let worker_hardware_fingerprint = coordinator
        .snapshots()
        .into_iter()
        .find(|snap| snap.worker_id == response.execution.worker_id)
        .and_then(|snap| snap.worker_hardware_fingerprint)
        .unwrap_or_default();

    let fingerprint = BenchmarkFingerprint {
        model_id: model_id_typed.clone(),
        model_file_size_bytes: file_size_bytes,
        model_file_hash: None,
        backend: response.execution.backend.clone(),
        backend_version: response.execution.backend_version.clone(),
        worker_hardware_fingerprint,
        accelerator: response.execution.accelerator,
        context_tokens: response.execution.context_tokens,
        gpu_layers: response.execution.gpu_layers,
        cpu_threads: response.execution.cpu_threads,
        batch_size: response.execution.batch_size,
    };
    let record = BenchmarkRecord {
        fingerprint,
        worker_id: response.execution.worker_id.clone(),
        timestamp_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        success: response.is_success(),
        model_load_ms: response.execution.model_load_ms,
        prompt_tokens_per_second: response.execution.prompt_tokens_per_second,
        generation_tokens_per_second: response.execution.generation_tokens_per_second,
        total_ms: response.execution.total_ms,
        prompt_tokens: response.execution.prompt_tokens,
        generated_tokens: response.execution.generated_tokens,
    };

    if let Err(e) = record_benchmark(&model_id_typed, &record) {
        eprintln!("[WARN] failed to persist benchmark record: {}", e);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&record).unwrap_or_default()
        );
    } else {
        println!(
            "Benchmark {} on {}: load={}ms prompt={:.1} tok/s gen={:.1} tok/s total={}ms",
            model_id,
            target.id,
            response.execution.model_load_ms,
            response.execution.prompt_tokens_per_second.unwrap_or(0.0),
            response
                .execution
                .generation_tokens_per_second
                .unwrap_or(0.0),
            response.execution.total_ms
        );
    }
    if !response.is_success() {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// route
// ---------------------------------------------------------------------------

pub fn route_explain(role: Option<String>, model: Option<String>, json: bool) {
    let workers = require_workers();
    let registry = load_registry();
    let coordinator = Coordinator::new(workers, registry.clone());

    let request = ModelRequest {
        request_id: RequestId::generate(),
        role: role
            .as_deref()
            .map(ModelRole::parse)
            .unwrap_or(ModelRole::Reasoning),
        model: model.map(ModelSelector::Alias),
        messages: vec![ModelMessage {
            role: MessageRole::User,
            content: "explain".to_string(),
        }],
        parameters: GenerationParameters::default(),
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    };
    let explanation = coordinator.route(&request, &SchedulingOverrides::default());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&explanation_json(&explanation)).unwrap_or_default()
        );
    } else {
        print!("{}", explanation);
    }
}

fn explanation_json(
    explanation: &lao_orchestrator_core::model::RoutingExplanation,
) -> serde_json::Value {
    serde_json::json!({
        "selected": explanation.selected.as_ref().map(|p| serde_json::json!({
            "worker_id": p.worker_id.0,
            "model_id": p.model_id.0,
            "backend": p.backend,
            "score": p.score,
            "used_cpu_fallback": p.used_cpu_fallback,
            "score_breakdown": p.score_breakdown.iter().map(|c| serde_json::json!({"label": c.label, "value": c.value})).collect::<Vec<_>>(),
        })),
        "rejected": explanation.rejected.iter().map(|r| serde_json::json!({
            "worker_id": r.worker_id.0,
            "model_id": r.model_id.as_ref().map(|m| m.0.clone()),
            "reasons": r.reasons,
        })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// jobs
// ---------------------------------------------------------------------------

pub fn jobs_list(worker: String, json: bool) {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match client.get(format!("{}/v1/jobs", target.url)).send() {
        Ok(r) => {
            let text = r.text().unwrap_or_default();
            if json {
                println!("{}", text);
            } else {
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                for job in parsed.as_array().cloned().unwrap_or_default() {
                    println!(
                        "- {} status={} request={}",
                        job.get("job_id").and_then(|v| v.as_str()).unwrap_or("?"),
                        job.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
                        job.get("request_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

pub fn jobs_inspect(job_id: String, worker: String, json: bool) {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match client
        .get(format!("{}/v1/jobs/{}", target.url, job_id))
        .send()
    {
        Ok(r) if r.status().is_success() => {
            let text = r.text().unwrap_or_default();
            if json {
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                println!("{}", serde_json::to_string_pretty(&parsed).unwrap_or(text));
            } else {
                println!("{}", text);
            }
        }
        Ok(r) => {
            eprintln!("[ERROR] worker returned {}", r.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

pub fn jobs_cancel(job_id: String, worker: String) {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match client
        .post(format!("{}/v1/jobs/{}/cancel", target.url, job_id))
        .send()
    {
        Ok(r) if r.status().is_success() => println!("Cancellation requested."),
        Ok(r) => {
            eprintln!("[ERROR] worker returned {}", r.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}
