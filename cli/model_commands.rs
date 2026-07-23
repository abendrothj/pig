//! Handlers for `worker`, `workers`, `models`, `route`, and `jobs` — the v0.5
//! local-inference CLI surface. Config (model registry + worker list) is read from
//! the same `pig.toml` resolution `TrustPolicy` already uses (`PIG_CONFIG`, else
//! `pig.toml`, else `config/pig.toml`), rather than inventing a parallel convention.

use crate::profiles::Profile;
use pig_core::model::{
    load_benchmark_records, record_benchmark, BenchmarkFingerprint, BenchmarkFreshness,
    BenchmarkRecord, CoordinatorMetricsSnapshot, GenerationParameters, MessageContent, MessageRole,
    ModelChunk, ModelId, ModelInvoker, ModelLoadState, ModelMessage, ModelRegistry, ModelRequest,
    ModelRequirements, ModelRole, ModelSelector, RequestId, SchedulingOverrides,
    WorkerLifecycleState, WorkerMetricsSnapshot, METRICS_SCHEMA_VERSION,
};
use pig_worker::coordinator::{Coordinator, WorkerEndpointConfig, WorkersConfig};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Instant;

fn config_path() -> Option<String> {
    if let Ok(p) = std::env::var("PIG_CONFIG") {
        return Some(p);
    }
    for candidate in ["pig.toml", "config/pig.toml"] {
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

pub fn resolve_profile(name: Option<&str>) -> Profile {
    match crate::profiles::selected(name, &config_text()) {
        Ok(profile) => profile,
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(2);
        }
    }
}

fn remote_request(
    profile: &Profile,
    method: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    let Some((_url, token_env)) = profile.remote() else {
        return Err("selected profile is not remote".to_string());
    };
    Ok(match token_env {
        Some(var) => method.bearer_auth(std::env::var(var).map_err(|_| {
            format!(
                "coordinator token environment variable '{}' is not set",
                var
            )
        })?),
        None => method,
    })
}

fn remote_url(profile: &Profile, path: &str) -> Result<String, String> {
    let Some((base, _)) = profile.remote() else {
        return Err("selected profile is not remote".to_string());
    };
    Ok(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
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
        eprintln!("[ERROR] no [[workers]] configured in pig.toml (or PIG_CONFIG)");
        std::process::exit(1);
    }
    workers
}

// ---------------------------------------------------------------------------
// worker serve
// ---------------------------------------------------------------------------

pub fn worker_serve(config: Option<String>) {
    let path = config.unwrap_or_else(|| "pig.toml".to_string());
    let worker_config = match pig_worker::config::WorkerConfig::load(std::path::Path::new(&path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ERROR] {e}");
            if std::path::Path::new("pig.toml.example").exists() {
                eprintln!("hint: copy pig.toml.example to pig.toml and fill in your worker and model paths");
            }
            std::process::exit(1);
        }
    };
    let registry_text = std::fs::read_to_string(&path).unwrap_or_default();
    let registry = ModelRegistry::from_toml_str(&registry_text).unwrap_or_default();
    let hardware = pig_worker::hardware::discover();
    let host_name = hardware.hostname.clone();

    let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
    rt.block_on(async move {
        let (backend, backend_name): (std::sync::Arc<dyn pig_worker::backend::ModelBackend>, &str) =
            if worker_config.runtime.mlx.enabled {
                (
                    std::sync::Arc::new(pig_worker::backend::mlx::MlxBackend::new(
                        pig_worker::backend::mlx::MlxConfig {
                            server_executable: PathBuf::from(
                                &worker_config.runtime.mlx.server_executable,
                            ),
                            startup_timeout: std::time::Duration::from_secs(
                                worker_config.runtime.mlx.startup_timeout_seconds,
                            ),
                            request_timeout: std::time::Duration::from_secs(
                                worker_config.runtime.mlx.request_timeout_seconds,
                            ),
                        },
                    )),
                    "mlx",
                )
            } else if worker_config.runtime.llama_cpp.enabled {
                (
                    std::sync::Arc::new(pig_worker::backend::llama_cpp::LlamaCppBackend::new(
                        pig_worker::backend::llama_cpp::LlamaCppConfig {
                            server_executable: PathBuf::from(
                                &worker_config.runtime.llama_cpp.server_executable,
                            ),
                            host: "127.0.0.1".to_string(),
                            startup_timeout: worker_config.llama_cpp_startup_timeout(),
                            request_timeout: worker_config.llama_cpp_request_timeout(),
                        },
                    )),
                    "llama_cpp",
                )
            } else {
                (
                    std::sync::Arc::new(pig_worker::backend::fake::FakeBackend::new()),
                    "fake",
                )
            };
        let backend_name = backend_name.to_string();

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

        let runtime = std::sync::Arc::new(pig_worker::job::WorkerRuntime::new(
            worker_config.id.clone(),
            host_name,
            backend,
            backend_name.clone(),
            worker_config.max_concurrent_jobs,
            worker_config.max_queued_jobs,
            worker_config.llama_cpp_request_timeout(),
        ));
        let state = std::sync::Arc::new(pig_worker::state::AppState {
            config: worker_config,
            runtime,
            registry,
            hardware,
            started_at: Instant::now(),
            auth_token,
            backend_name,
            hardware_cache: std::sync::Mutex::new(None),
        });

        if let Err(e) = pig_worker::server::serve(state).await {
            eprintln!("[ERROR] worker server failed: {}", e);
            std::process::exit(1);
        }
    });
}

// ---------------------------------------------------------------------------
// workers
// ---------------------------------------------------------------------------

pub fn workers_list(json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        let response = remote_request(
            profile,
            client.get(remote_url(profile, "/v1/workers").unwrap()),
        )
        .and_then(|request| request.send().map_err(|e| e.to_string()))
        .and_then(|response| response.error_for_status().map_err(|e| e.to_string()));
        let snapshots: Vec<pig_core::model::WorkerSnapshot> = match response {
            Ok(response) => match response.json() {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("[ERROR] malformed coordinator response: {}", e);
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                std::process::exit(1);
            }
        };
        print_snapshots(&snapshots, json);
        return;
    }
    let workers = load_workers();
    let coordinator = Coordinator::new(workers.clone(), load_registry());
    let snapshots = coordinator.snapshots();
    print_snapshots(&snapshots, json);
}

fn print_snapshots(snapshots: &[pig_core::model::WorkerSnapshot], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot_json(snapshots)).unwrap_or_default()
        );
        return;
    }
    if snapshots.is_empty() {
        println!("No workers configured.");
        return;
    }
    for s in snapshots {
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

pub fn workers_health(json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        workers_list(json, profile);
        return;
    }
    let coordinator = Coordinator::new(load_workers(), load_registry());
    let snapshots = coordinator.snapshots();
    print_snapshots(&snapshots, json);
    if snapshots.iter().any(|s| !s.healthy) {
        std::process::exit(1);
    }
}

pub fn workers_inspect(worker_id: String, json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        let response = remote_request(
            profile,
            client.get(remote_url(profile, "/v1/workers").unwrap()),
        )
        .and_then(|request| request.send().map_err(|e| e.to_string()))
        .and_then(|response| response.error_for_status().map_err(|e| e.to_string()));
        let snapshots: Vec<pig_core::model::WorkerSnapshot> =
            match response.and_then(|r| r.json().map_err(|e| e.to_string())) {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                    std::process::exit(1);
                }
            };
        let Some(snapshot) = snapshots.into_iter().find(|s| s.worker_id.0 == worker_id) else {
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
        return;
    }
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

fn snapshot_json(snapshots: &[pig_core::model::WorkerSnapshot]) -> serde_json::Value {
    serde_json::to_value(snapshots).unwrap_or(serde_json::Value::Array(vec![]))
}

/// `worker_id` given -> that one worker's live telemetry. Omitted -> a coordinator-
/// wide aggregate built entirely from functions that already exist elsewhere
/// (`coordinator.snapshots()`, the same `worker_metrics()` client used for the
/// single-worker case, and the existing benchmark-history functions) - not a new
/// aggregation framework, just summing numbers that are already computed somewhere.
pub fn workers_metrics(worker_id: Option<String>, json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        eprintln!("[ERROR] remote coordinator metrics aggregation is not implemented; query the coordinator's /v1/metrics when it is added");
        std::process::exit(1);
    }
    let workers = load_workers();
    let coordinator = Coordinator::new(workers, load_registry());

    match worker_id {
        Some(id) => {
            let snapshot = match coordinator.worker_metrics(&id) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] {}", e);
                    std::process::exit(1);
                }
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot).unwrap_or_default()
                );
            } else {
                println!("{}", format_worker_metrics(&snapshot));
            }
        }
        None => {
            let snapshot = coordinator_metrics_snapshot(&coordinator);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot).unwrap_or_default()
                );
            } else {
                println!("{}", format_coordinator_metrics(&snapshot));
            }
        }
    }
}

fn coordinator_metrics_snapshot(coordinator: &Coordinator) -> CoordinatorMetricsSnapshot {
    let snapshots = coordinator.snapshots();
    let known_workers = snapshots.len();
    let connected: Vec<_> = snapshots.iter().filter(|s| s.healthy).collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut successful_jobs = 0u64;
    let mut failed_jobs = 0u64;
    let mut benchmarks = Vec::new();
    for snap in &connected {
        if let Ok(m) = coordinator.worker_metrics(&snap.worker_id.0) {
            successful_jobs += m.jobs.completed;
            failed_jobs += m.jobs.failed;
        }
        for model_id in &snap.known_models {
            let latest = load_benchmark_records(model_id)
                .into_iter()
                .filter(|r| r.worker_id == snap.worker_id)
                .max_by_key(|r| r.timestamp_unix_ms);
            let age_seconds = latest.map(|r| now.saturating_sub(r.timestamp_unix_ms) / 1000);
            benchmarks.push(BenchmarkFreshness {
                worker_id: snap.worker_id.clone(),
                model_id: model_id.clone(),
                age_seconds,
                fingerprint_valid: snap.benchmarks.contains_key(model_id),
            });
        }
    }

    CoordinatorMetricsSnapshot {
        schema_version: METRICS_SCHEMA_VERSION,
        timestamp_unix_ms: now,
        known_workers,
        connected_workers: connected.len(),
        active_jobs: snapshots.iter().map(|s| s.active_jobs).sum(),
        queued_jobs: snapshots.iter().map(|s| s.queue_depth).sum(),
        successful_jobs,
        failed_jobs,
        // See core::model::metrics's module doc: no honest cumulative owner exists
        // for this on an ephemeral CLI-process coordinator.
        scheduler_decision: None,
        benchmarks,
    }
}

fn opt_pct(v: Option<f32>) -> String {
    match v {
        Some(x) => format!("{:.0}%", x),
        None => "unavailable".to_string(),
    }
}

fn opt_tps(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.1} tok/s", x),
        None => "unavailable".to_string(),
    }
}

fn format_gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn format_worker_metrics(m: &WorkerMetricsSnapshot) -> String {
    let state = match m.worker.lifecycle_state {
        WorkerLifecycleState::Idle => "idle",
        WorkerLifecycleState::Loading => "loading",
        WorkerLifecycleState::Running => "running",
    };
    let hours = m.worker.uptime_seconds / 3600;
    let minutes = (m.worker.uptime_seconds % 3600) / 60;

    let mut out = String::new();
    out.push_str(&format!("Worker: {}\n", m.worker.worker_id));
    out.push_str(&format!("State: {}\n", state));
    out.push_str(&format!("Uptime: {}h {}m\n\n", hours, minutes));

    out.push_str(&format!(
        "Queue: {} / {}\n",
        m.queue.depth, m.queue.capacity
    ));
    out.push_str(&format!(
        "Jobs: {} active, {} completed, {} failed, {} cancelled\n\n",
        m.jobs.active, m.jobs.completed, m.jobs.failed, m.jobs.cancelled
    ));

    match &m.model.loaded_model_id {
        Some(id) => out.push_str(&format!("Model: {}\n", id)),
        None => {
            let state = match m.model.load_state {
                ModelLoadState::NotLoaded => "not loaded",
                ModelLoadState::Loading => "loading",
                ModelLoadState::Loaded => "loaded",
            };
            out.push_str(&format!("Model: none ({})\n", state));
        }
    }

    if let Some(name) = &m.accelerator.name {
        out.push_str(&format!("GPU: {}\n", name));
        out.push_str(&format!(
            "GPU utilization: {}\n",
            opt_pct(m.accelerator.utilization_percent)
        ));
        match (
            m.accelerator.memory_used_bytes,
            m.accelerator.memory_total_bytes,
        ) {
            (Some(used), Some(total)) => out.push_str(&format!(
                "VRAM: {} / {}\n",
                format_gib(used),
                format_gib(total)
            )),
            _ => out.push_str("VRAM: unavailable\n"),
        }
    } else {
        out.push_str("Accelerator: none\n");
    }
    out.push('\n');

    out.push_str(&format!(
        "Prompt throughput: {}\n",
        opt_tps(m.throughput.last_prompt_tokens_per_second)
    ));
    out.push_str(&format!(
        "Generation throughput: {}\n",
        opt_tps(m.throughput.last_generation_tokens_per_second)
    ));

    out
}

fn format_coordinator_metrics(m: &CoordinatorMetricsSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!("Known workers: {}\n", m.known_workers));
    out.push_str(&format!("Connected workers: {}\n\n", m.connected_workers));
    out.push_str(&format!("Active jobs: {}\n", m.active_jobs));
    out.push_str(&format!("Queued jobs: {}\n", m.queued_jobs));
    out.push_str(&format!(
        "Jobs: {} successful, {} failed\n\n",
        m.successful_jobs, m.failed_jobs
    ));
    if m.benchmarks.is_empty() {
        out.push_str("Benchmarks: none recorded\n");
    } else {
        out.push_str("Benchmarks:\n");
        for b in &m.benchmarks {
            let age = match b.age_seconds {
                Some(s) => format!("{}s ago", s),
                None => "no history".to_string(),
            };
            out.push_str(&format!(
                "- {} / {}: {} ({})\n",
                b.worker_id,
                b.model_id,
                if b.fingerprint_valid {
                    "valid"
                } else {
                    "stale"
                },
                age
            ));
        }
    }
    out
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
    let found = pig_core::model::discover_gguf_files(std::path::Path::new(&directory));
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

/// Attach the worker's bearer token (if configured and its env var is set) to a
/// request. The `Coordinator`-based paths (`workers *`, non-streaming `models
/// generate`, `models benchmark`) get this for free; every direct HTTP call the CLI
/// makes against a worker's own endpoints has to do it explicitly.
fn with_worker_auth(
    mut req: reqwest::blocking::RequestBuilder,
    target: &WorkerEndpointConfig,
) -> reqwest::blocking::RequestBuilder {
    if let Some(var) = &target.auth_token_env {
        if let Ok(token) = std::env::var(var) {
            req = req.bearer_auth(token);
        }
    }
    req
}

pub fn models_load(model_id: String, worker: Option<String>) {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, worker.clone());
    let client = reqwest::blocking::Client::new();
    let resp = with_worker_auth(
        client
            .post(format!("{}/v1/models/load", target.url))
            .json(&serde_json::json!({"model_id": model_id.clone()})),
        &target,
    )
    .send();
    match resp {
        Ok(r) if r.status().is_success() => {
            let body = r.text().unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                let accel = v["accelerator"].as_str().unwrap_or("cpu");
                let load_ms = v["load_ms"].as_u64().unwrap_or(0);
                let already = v["already_loaded"].as_bool().unwrap_or(false);
                if already {
                    println!("Already loaded: {} ({})", model_id, accel);
                } else {
                    println!("Loaded: {} ({}, {}ms)", model_id, accel, load_ms);
                }
            } else {
                println!("{}", body);
            }
            let bench_path = format!(
                ".pig_benchmarks/{}.jsonl",
                model_id.replace(['/', ' '], "_")
            );
            let model_id_bg = model_id.clone();
            let worker_bg = worker.clone();
            std::thread::spawn(move || {
                models_benchmark_silent(model_id_bg, worker_bg);
            });
            println!("Auto-benchmark running in background → {}", bench_path);
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
    let resp = with_worker_auth(
        client
            .post(format!("{}/v1/models/unload", target.url))
            .json(&serde_json::json!({"model_id": model_id})),
        &target,
    )
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
    let workers = require_workers();
    let registry = load_registry();

    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(ModelMessage {
            role: MessageRole::System,
            content: MessageContent::Text(s),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }
    messages.push(ModelMessage {
        role: MessageRole::User,
        content: MessageContent::Text(prompt),
        tool_calls: vec![],
        tool_call_id: None,
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
            pig_core::artifact::Artifact::Text(t) => println!("{}", t),
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
    let req = with_worker_auth(
        client
            .post(format!("{}/v1/generate", target.url))
            .json(&body),
        target,
    );
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
    // "chunk" payloads are canonical JSON ModelChunk values. A blank line ends an
    // event block; the final worker response is deliberately ignored here.
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
            if current_event.as_deref() == Some("chunk") {
                if let Ok(ModelChunk::TextDelta { text }) = serde_json::from_str(payload) {
                    let _ = write!(out, "{}", text);
                    let _ = out.flush();
                }
            }
            continue;
        }
        if line.is_empty() {
            current_event = None;
        }
    }
    println!();
}

/// Recursively sum file sizes under a path (directory → walk; file → its own size).
fn model_disk_size(path: &std::path::Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return Some(meta.len());
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if let Ok(m) = std::fs::metadata(&p) {
                if m.is_file() {
                    total += m.len();
                } else if m.is_dir() {
                    stack.push(p);
                }
            }
        }
    }
    Some(total)
}

fn median_f64(mut vals: Vec<f64>) -> f64 {
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = vals.len();
    if n % 2 == 0 {
        (vals[n / 2 - 1] + vals[n / 2]) / 2.0
    } else {
        vals[n / 2]
    }
}

fn median_u64(mut vals: Vec<u64>) -> u64 {
    vals.sort_unstable();
    let n = vals.len();
    if n % 2 == 0 {
        (vals[n / 2 - 1] + vals[n / 2]) / 2
    } else {
        vals[n / 2]
    }
}

const BENCHMARK_RUNS: usize = 3;

/// Runs the benchmark prompt BENCHMARK_RUNS times and returns a record with
/// median metrics. Returns `(record, last_response, target_worker)`.
fn run_benchmark(
    model_id: &str,
    worker: Option<String>,
) -> Option<(
    BenchmarkRecord,
    pig_core::model::ModelResponse,
    WorkerEndpointConfig,
)> {
    let workers = require_workers();
    let target = resolve_target_worker(&workers, worker);
    let registry = load_registry();
    let model_id_typed = ModelId::from(model_id);
    let file_size_bytes = registry
        .get(&model_id_typed)
        .and_then(|entry| model_disk_size(&entry.path));
    let coordinator = Coordinator::new(vec![target.clone()], registry);

    let make_request = || ModelRequest {
        request_id: RequestId::generate(),
        role: ModelRole::Reasoning,
        model: Some(ModelSelector::Id(model_id_typed.clone())),
        messages: vec![ModelMessage {
            role: MessageRole::User,
            content: MessageContent::Text(
                "Count from 1 to 50. Write each number on its own line. Do not add any other text."
                    .to_string(),
            ),
            tool_calls: vec![],
            tool_call_id: None,
        }],
        parameters: GenerationParameters {
            max_tokens: Some(200),
            temperature: Some(0.0),
            ..Default::default()
        },
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    };

    let mut last_response = None;
    let mut gen_tps_samples: Vec<f64> = Vec::new();
    let mut prompt_tps_samples: Vec<f64> = Vec::new();
    let mut ttft_samples: Vec<u64> = Vec::new();
    let mut total_ms_samples: Vec<u64> = Vec::new();
    let mut prompt_tokens_last = 0u32;
    let mut generated_tokens_last = 0u32;

    for _ in 0..BENCHMARK_RUNS {
        let resp = coordinator.invoke(make_request());
        if resp.is_success() {
            if let Some(v) = resp.execution.generation_tokens_per_second {
                gen_tps_samples.push(v);
            }
            if let Some(v) = resp.execution.prompt_tokens_per_second {
                prompt_tps_samples.push(v);
            }
            ttft_samples.push(resp.execution.prompt_eval_ms);
            total_ms_samples.push(resp.execution.total_ms);
            prompt_tokens_last = resp.execution.prompt_tokens;
            generated_tokens_last = resp.execution.generated_tokens;
        }
        last_response = Some(resp);
    }

    let response = last_response?;

    let worker_hardware_fingerprint = coordinator
        .snapshots()
        .into_iter()
        .find(|snap| snap.worker_id == response.execution.worker_id)
        .and_then(|snap| snap.worker_hardware_fingerprint)
        .unwrap_or_default();

    let gen_tps = if gen_tps_samples.is_empty() {
        None
    } else {
        Some(median_f64(gen_tps_samples))
    };
    let prompt_tps = if prompt_tps_samples.is_empty() {
        None
    } else {
        Some(median_f64(prompt_tps_samples))
    };
    let p50_ttft = if ttft_samples.is_empty() {
        None
    } else {
        Some(median_u64(ttft_samples) as u32)
    };
    let total_ms = if total_ms_samples.is_empty() {
        response.execution.total_ms
    } else {
        median_u64(total_ms_samples)
    };

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
        prompt_tokens_per_second: prompt_tps,
        generation_tokens_per_second: gen_tps,
        total_ms,
        prompt_tokens: prompt_tokens_last,
        generated_tokens: generated_tokens_last,
        p50_ttft_ms: p50_ttft,
        p95_ttft_ms: None,
        pipeline_acceptance_rate: None,
    };

    if let Err(e) = record_benchmark(&model_id_typed, &record) {
        eprintln!("[WARN] failed to persist benchmark record: {}", e);
    }

    Some((record, response, target))
}

/// Background-safe variant: runs the benchmark but produces no stdout output.
/// Called after `pig models load` so the scheduler gets fresh data immediately.
pub fn models_benchmark_silent(model_id: String, worker: Option<String>) {
    run_benchmark(&model_id, worker);
}

pub fn models_benchmark(model_id: String, worker: Option<String>, json: bool) {
    let Some((record, response, target)) = run_benchmark(&model_id, worker) else {
        eprintln!("[ERROR] benchmark failed");
        std::process::exit(1);
    };
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

pub fn route_explain(role: Option<String>, model: Option<String>, json: bool, profile: &Profile) {
    let request = ModelRequest {
        request_id: RequestId::generate(),
        role: role
            .as_deref()
            .map(ModelRole::parse)
            .unwrap_or(ModelRole::Reasoning),
        model: model.map(ModelSelector::Alias),
        messages: vec![ModelMessage {
            role: MessageRole::User,
            content: MessageContent::Text("explain".to_string()),
            tool_calls: vec![],
            tool_call_id: None,
        }],
        parameters: GenerationParameters::default(),
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    };
    let explanation = if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        match remote_request(
            profile,
            client
                .post(remote_url(profile, "/v1/route").unwrap())
                .json(&request),
        )
        .and_then(|request| request.send().map_err(|e| e.to_string()))
        .and_then(|response| response.error_for_status().map_err(|e| e.to_string()))
        .and_then(|response| response.json().map_err(|e| e.to_string()))
        {
            Ok(explanation) => explanation,
            Err(e) => {
                eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        let coordinator = Coordinator::new(require_workers(), load_registry());
        coordinator.route(&request, &SchedulingOverrides::default())
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&explanation_json(&explanation)).unwrap_or_default()
        );
    } else {
        print!("{}", explanation);
    }
}

fn explanation_json(explanation: &pig_core::model::RoutingExplanation) -> serde_json::Value {
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

pub fn jobs_list(worker: String, json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        let url = format!(
            "{}?worker={}",
            remote_url(profile, "/v1/jobs").unwrap(),
            worker
        );
        let result = remote_request(profile, client.get(url))
            .and_then(|request| request.send().map_err(|e| e.to_string()))
            .and_then(|response| response.error_for_status().map_err(|e| e.to_string()))
            .and_then(|response| response.text().map_err(|e| e.to_string()));
        match result {
            Ok(text) => {
                if json {
                    println!("{}", text);
                } else {
                    print_jobs(&text);
                }
            }
            Err(e) => {
                eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match with_worker_auth(client.get(format!("{}/v1/jobs", target.url)), &target).send() {
        Ok(r) => {
            let text = r.text().unwrap_or_default();
            if json {
                println!("{}", text);
            } else {
                print_jobs(&text);
            }
        }
        Err(e) => {
            eprintln!("[ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

fn print_jobs(text: &str) {
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    for job in parsed.as_array().cloned().unwrap_or_default() {
        println!(
            "- {} status={} request={}",
            job.get("job_id").and_then(|v| v.as_str()).unwrap_or("?"),
            job.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
            job.get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
        );
    }
}

pub fn jobs_inspect(job_id: String, worker: String, json: bool, profile: &Profile) {
    if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        let url = format!(
            "{}?worker={}",
            remote_url(profile, &format!("/v1/jobs/{}", job_id)).unwrap(),
            worker
        );
        let result = remote_request(profile, client.get(url))
            .and_then(|request| request.send().map_err(|e| e.to_string()))
            .and_then(|response| response.error_for_status().map_err(|e| e.to_string()))
            .and_then(|response| response.text().map_err(|e| e.to_string()));
        match result {
            Ok(text) => {
                if json {
                    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    println!("{}", serde_json::to_string_pretty(&parsed).unwrap_or(text));
                } else {
                    println!("{}", text);
                }
            }
            Err(e) => {
                eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match with_worker_auth(
        client.get(format!("{}/v1/jobs/{}", target.url, job_id)),
        &target,
    )
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

pub fn jobs_cancel(job_id: String, worker: String, profile: &Profile) {
    if profile.remote().is_some() {
        let client = reqwest::blocking::Client::new();
        let url = format!(
            "{}?worker={}",
            remote_url(profile, &format!("/v1/jobs/{}/cancel", job_id)).unwrap(),
            worker
        );
        match remote_request(profile, client.post(url))
            .and_then(|request| request.send().map_err(|e| e.to_string()))
            .and_then(|response| response.error_for_status().map_err(|e| e.to_string()))
        {
            Ok(_) => println!("Cancellation requested."),
            Err(e) => {
                eprintln!("[ERROR] remote coordinator unavailable: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let workers = require_workers();
    let target = resolve_target_worker(&workers, Some(worker));
    let client = reqwest::blocking::Client::new();
    match with_worker_auth(
        client.post(format!("{}/v1/jobs/{}/cancel", target.url, job_id)),
        &target,
    )
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

#[cfg(test)]
mod metrics_formatting_tests {
    use super::*;
    use pig_core::model::{
        AcceleratorKind, AcceleratorMetrics, JobMetrics, ModelMetrics, QueueMetrics, SystemMetrics,
        ThroughputMetrics, WorkerId, WorkerIdentityMetrics,
    };

    fn fully_populated() -> WorkerMetricsSnapshot {
        WorkerMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 0,
            worker: WorkerIdentityMetrics {
                worker_id: WorkerId::from("fedora-worker"),
                uptime_seconds: 8_040, // 2h 14m
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
                cumulative_tokens_processed: 999,
            },
            model: ModelMetrics {
                loaded_model_id: Some(ModelId::from("qwen2.5-8b-q4")),
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
                memory_used_bytes: Some(7_086_696_038), // ~6.6 GiB
                memory_total_bytes: Some(8_589_934_592), // 8.0 GiB
            },
            throughput: ThroughputMetrics {
                last_prompt_tokens_per_second: Some(521.0),
                last_generation_tokens_per_second: Some(78.4),
            },
        }
    }

    fn all_optionals_absent() -> WorkerMetricsSnapshot {
        WorkerMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 0,
            worker: WorkerIdentityMetrics {
                worker_id: WorkerId::from("idle-worker"),
                uptime_seconds: 30,
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
        }
    }

    #[test]
    fn formats_a_fully_populated_snapshot_matching_the_expected_shape() {
        let out = format_worker_metrics(&fully_populated());
        assert!(out.contains("Worker: fedora-worker"));
        assert!(out.contains("State: running"));
        assert!(out.contains("Uptime: 2h 14m"));
        assert!(out.contains("Queue: 1 / 8"));
        assert!(out.contains("Jobs: 1 active, 42 completed, 1 failed, 0 cancelled"));
        assert!(out.contains("Model: qwen2.5-8b-q4"));
        assert!(out.contains("GPU: NVIDIA GeForce RTX 2080 SUPER"));
        assert!(out.contains("GPU utilization: 84%"));
        assert!(out.contains("VRAM: 6.6 GiB / 8.0 GiB"));
        assert!(out.contains("Prompt throughput: 521.0 tok/s"));
        assert!(out.contains("Generation throughput: 78.4 tok/s"));
    }

    #[test]
    fn missing_measurements_render_as_unavailable_not_zero() {
        let out = format_worker_metrics(&all_optionals_absent());
        assert!(out.contains("Model: none (not loaded)"));
        assert!(out.contains("Accelerator: none"));
        assert!(out.contains("Prompt throughput: unavailable"));
        assert!(out.contains("Generation throughput: unavailable"));
        assert!(
            !out.contains("0%") && !out.contains("0.0 tok/s"),
            "an unavailable measurement must never be rendered as a fabricated zero: {}",
            out
        );
    }

    #[test]
    fn coordinator_metrics_distinguishes_no_history_from_stale_benchmarks() {
        let snapshot = CoordinatorMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 0,
            known_workers: 2,
            connected_workers: 1,
            active_jobs: 1,
            queued_jobs: 0,
            successful_jobs: 42,
            failed_jobs: 1,
            scheduler_decision: None,
            benchmarks: vec![
                BenchmarkFreshness {
                    worker_id: WorkerId::from("fedora-worker"),
                    model_id: ModelId::from("qwen2.5-8b-q4"),
                    age_seconds: Some(120),
                    fingerprint_valid: true,
                },
                BenchmarkFreshness {
                    worker_id: WorkerId::from("fedora-worker"),
                    model_id: ModelId::from("old-model"),
                    age_seconds: Some(999_999),
                    fingerprint_valid: false,
                },
                BenchmarkFreshness {
                    worker_id: WorkerId::from("fedora-worker"),
                    model_id: ModelId::from("never-benchmarked"),
                    age_seconds: None,
                    fingerprint_valid: false,
                },
            ],
        };
        let out = format_coordinator_metrics(&snapshot);
        assert!(out.contains("Known workers: 2"));
        assert!(out.contains("Connected workers: 1"));
        assert!(out.contains("Jobs: 42 successful, 1 failed"));
        assert!(out.contains("qwen2.5-8b-q4: valid (120s ago)"));
        assert!(out.contains("old-model: stale (999999s ago)"));
        assert!(
            out.contains("never-benchmarked: stale (no history)"),
            "a model with no benchmark history at all must read distinctly from a stale one: {}",
            out
        );
    }

    #[test]
    fn coordinator_metrics_with_no_benchmarks_says_so_explicitly() {
        let snapshot = CoordinatorMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            timestamp_unix_ms: 0,
            known_workers: 1,
            connected_workers: 1,
            active_jobs: 0,
            queued_jobs: 0,
            successful_jobs: 0,
            failed_jobs: 0,
            scheduler_decision: None,
            benchmarks: vec![],
        };
        assert!(format_coordinator_metrics(&snapshot).contains("Benchmarks: none recorded"));
    }
}
