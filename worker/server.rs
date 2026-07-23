//! The worker's HTTP surface. Every endpoint here is genuinely implemented — nothing
//! is advertised in `/v1/capabilities` that isn't real (embeddings/reranking return an
//! explicit unsupported-capability response rather than pretending to work).
//!
//! Streaming uses Server-Sent Events: `POST /v1/generate` with `"stream": true`
//! returns `text/event-stream`, canonical `event: chunk` records during inference,
//! followed by one `event: response` carrying the final structured `ModelResponse`.

use crate::backend::{BackendError, LoadModelRequest};
use crate::job::{JobRecord, QueueError, ResolvedGeneration};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use pig_core::model::{
    AcceleratorMetrics, JobId, ModelId, ModelLoadState, ModelMetrics, ModelRequest, ModelSelector,
    QueueMetrics, SystemMetrics, ThroughputMetrics, WorkerId, WorkerIdentityMetrics,
    WorkerLifecycleState, WorkerMetricsSnapshot, METRICS_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}

fn backend_error_response(e: BackendError) -> Response {
    let status = match e {
        BackendError::ModelNotFound(_) => StatusCode::NOT_FOUND,
        BackendError::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
        BackendError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        BackendError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        BackendError::Cancelled => StatusCode::CONFLICT,
        BackendError::LoadFailed(_)
        | BackendError::GenerationFailed(_)
        | BackendError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, e.to_string())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/metrics", get(metrics))
        .route("/v1/models", get(list_models))
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/:job_id", get(get_job))
        .route("/v1/jobs/:job_id/cancel", post(cancel_job))
        .route("/v1/models/load", post(load_model))
        .route("/v1/models/unload", post(unload_model))
        .route("/v1/generate", post(generate))
        .route("/v1/embed", post(embed_unsupported))
        .route("/v1/rerank", post(rerank_unsupported))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if let Some(expected) = &state.auth_token {
        let provided = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(expected.as_str()) {
            return error_response(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
        }
    }
    next.run(req).await
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    worker_id: String,
    coordinator_id: Option<String>,
    uptime_seconds: u64,
    backend_available: bool,
    backend_detail: String,
    active_jobs: usize,
    queued_jobs: usize,
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let backend_health = state.runtime.backend().health().await;
    let (available, detail) = match backend_health {
        Ok(h) => (h.available, h.detail),
        Err(e) => (false, e.to_string()),
    };
    Json(HealthResponse {
        status: if available { "ok" } else { "degraded" }.to_string(),
        worker_id: state.config.id.clone(),
        coordinator_id: state.config.coordinator_id.clone(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        backend_available: available,
        backend_detail: detail,
        active_jobs: state.runtime.active_jobs().await,
        queued_jobs: state.runtime.queue_depth(),
    })
}

#[derive(Debug, Serialize)]
struct CapabilitiesResponse {
    worker_id: String,
    hardware: crate::hardware::HardwareInfo,
    backend: Option<crate::backend::BackendCapabilities>,
    max_concurrent_jobs: usize,
    max_queued_jobs: usize,
    known_models: Vec<String>,
}

async fn capabilities(State(state): State<Arc<AppState>>) -> Json<CapabilitiesResponse> {
    let backend = state.runtime.backend().capabilities().await.ok();
    Json(CapabilitiesResponse {
        worker_id: state.config.id.clone(),
        hardware: state.hardware.clone(),
        backend,
        max_concurrent_jobs: state.config.max_concurrent_jobs,
        max_queued_jobs: state.config.max_queued_jobs,
        known_models: state
            .registry
            .all_resolved()
            .into_iter()
            .map(|r| r.entry.id.0)
            .collect(),
    })
}

/// TTL for the re-probed (live) hardware reading `/v1/metrics` uses for
/// system/accelerator utilization and VRAM. `discover()` shells out to
/// `nvidia-smi`/reads `/proc`, so this bounds how often that happens under request
/// load rather than doing it on every single request.
const HARDWARE_CACHE_TTL: Duration = Duration::from_secs(2);

/// Refreshes `state.hardware_cache` only when missing or stale, via
/// `spawn_blocking` since `discover()` does blocking I/O (process spawn, file
/// reads) and nothing before this endpoint has ever called it from inside the
/// async runtime.
async fn live_hardware(state: &AppState) -> crate::hardware::HardwareInfo {
    if let Some((fetched_at, info)) = &*state.hardware_cache.lock().unwrap() {
        if fetched_at.elapsed() < HARDWARE_CACHE_TTL {
            return info.clone();
        }
    }
    let fresh = tokio::task::spawn_blocking(crate::hardware::discover)
        .await
        .unwrap_or_default();
    *state.hardware_cache.lock().unwrap() = Some((Instant::now(), fresh.clone()));
    fresh
}

async fn metrics(State(state): State<Arc<AppState>>) -> Json<WorkerMetricsSnapshot> {
    let job_metrics = state.runtime.job_metrics().await;
    let models = state
        .runtime
        .backend()
        .list_models()
        .await
        .unwrap_or_default();
    let loaded_model_id = models.iter().find(|m| m.loaded).map(|m| m.model_id.clone());

    let lifecycle_state = if job_metrics.currently_loading {
        WorkerLifecycleState::Loading
    } else if job_metrics.counts.active > 0 {
        WorkerLifecycleState::Running
    } else {
        WorkerLifecycleState::Idle
    };
    let load_state = if job_metrics.currently_loading {
        ModelLoadState::Loading
    } else if loaded_model_id.is_some() {
        ModelLoadState::Loaded
    } else {
        ModelLoadState::NotLoaded
    };

    // `state.hardware` is the one-time startup snapshot - the same source
    // `/v1/capabilities` reports from, and the authority on *whether* an
    // accelerator exists at all. `live` is a fresh (TTL-cached) re-probe, trusted
    // only for numbers that legitimately change over time (available memory/VRAM,
    // utilization) - never for identity. Without this split, a re-probe on a
    // machine whose *real* hardware differs from what this worker was configured/
    // tested with (e.g. a dev machine's own GPU) would contradict `/v1/capabilities`
    // and report accelerator data for a device this worker isn't actually using.
    let live = live_hardware(&state).await;
    let accelerator_present = state.hardware.accelerator.is_some();

    Json(WorkerMetricsSnapshot {
        schema_version: METRICS_SCHEMA_VERSION,
        timestamp_unix_ms: crate::job::now_ms(),
        worker: WorkerIdentityMetrics {
            worker_id: WorkerId::from(state.config.id.clone()),
            uptime_seconds: state.started_at.elapsed().as_secs(),
            lifecycle_state,
        },
        queue: QueueMetrics {
            capacity: state.config.max_queued_jobs,
            depth: state.runtime.queue_depth(),
        },
        jobs: job_metrics.counts,
        model: ModelMetrics {
            loaded_model_id,
            load_state,
            last_load_duration_ms: job_metrics.last_load_duration_ms,
        },
        system: SystemMetrics {
            memory_used_bytes: match (
                state.hardware.total_memory_bytes,
                live.available_memory_bytes,
            ) {
                (Some(total), Some(available)) => Some(total.saturating_sub(available)),
                _ => None,
            },
            memory_total_bytes: state.hardware.total_memory_bytes,
            cpu_utilization_percent: crate::hardware::cpu_utilization_percent(
                state.hardware.logical_cpus,
            ),
        },
        accelerator: AcceleratorMetrics {
            kind: state.hardware.accelerator,
            name: state.hardware.accelerator_name.clone(),
            utilization_percent: accelerator_present
                .then_some(live.accelerator_utilization_percent)
                .flatten(),
            memory_used_bytes: accelerator_present
                .then(
                    || match (state.hardware.total_vram_bytes, live.available_vram_bytes) {
                        (Some(total), Some(available)) => Some(total.saturating_sub(available)),
                        _ => None,
                    },
                )
                .flatten(),
            memory_total_bytes: accelerator_present
                .then_some(state.hardware.total_vram_bytes)
                .flatten(),
        },
        throughput: ThroughputMetrics {
            last_prompt_tokens_per_second: job_metrics.last_prompt_tokens_per_second,
            last_generation_tokens_per_second: job_metrics.last_generation_tokens_per_second,
        },
    })
}

async fn list_models(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<pig_core::model::ResolvedModelEntry>> {
    Json(state.registry.all_resolved())
}

async fn list_jobs(State(state): State<Arc<AppState>>) -> Json<Vec<JobRecord>> {
    Json(state.runtime.list_jobs().await)
}

async fn get_job(State(state): State<Arc<AppState>>, Path(job_id): Path<String>) -> Response {
    match state.runtime.job(&JobId::from(job_id)).await {
        Some(record) => Json(record).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "job not found"),
    }
}

async fn cancel_job(State(state): State<Arc<AppState>>, Path(job_id): Path<String>) -> Response {
    if state.runtime.cancel(&JobId::from(job_id)).await {
        StatusCode::ACCEPTED.into_response()
    } else {
        error_response(StatusCode::NOT_FOUND, "job not found or already finished")
    }
}

#[derive(Debug, Deserialize)]
struct LoadModelHttpRequest {
    model_id: String,
    #[serde(default)]
    execution_config: serde_json::Value,
}

async fn load_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoadModelHttpRequest>,
) -> Response {
    let model_id = ModelId::from(req.model_id);
    let Some(entry) = state.registry.get(&model_id).cloned() else {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("unknown model id '{}'", model_id),
        );
    };
    if !entry.path.is_file() {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("model file not found: {}", entry.path.display()),
        );
    }
    let result = state
        .runtime
        .backend()
        .load_model(LoadModelRequest {
            model_id,
            path: entry.path,
            context_size: entry.context_tokens,
            execution_config: req.execution_config,
        })
        .await;
    match result {
        Ok(loaded) => Json(loaded).into_response(),
        Err(e) => backend_error_response(e),
    }
}

#[derive(Debug, Deserialize)]
struct UnloadModelHttpRequest {
    model_id: String,
}

async fn unload_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UnloadModelHttpRequest>,
) -> Response {
    match state
        .runtime
        .backend()
        .unload_model(&ModelId::from(req.model_id))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => backend_error_response(e),
    }
}

fn resolve_model(
    state: &AppState,
    request: &ModelRequest,
) -> Result<ResolvedGeneration, (StatusCode, String)> {
    let entry = match &request.model {
        Some(ModelSelector::Id(id)) => state.registry.get(id).cloned(),
        Some(ModelSelector::Alias(alias)) => {
            state.registry.get(&ModelId::from(alias.clone())).cloned()
        }
        None => state
            .registry
            .candidates_for_role(&request.role)
            .into_iter()
            .find(|e| e.path.is_file())
            .cloned(),
    };
    let Some(entry) = entry else {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "no available model for role '{}' or the given selector",
                request.role
            ),
        ));
    };
    if !entry.path.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("model file not found: {}", entry.path.display()),
        ));
    }
    Ok(ResolvedGeneration {
        model_id: entry.id,
        model_path: entry.path,
        context_size: entry.context_tokens,
        execution_config: serde_json::Value::Null,
    })
}

#[derive(Debug, Deserialize)]
struct GenerateHttpRequest {
    #[serde(flatten)]
    request: ModelRequest,
    #[serde(default)]
    stream: bool,
}

async fn generate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GenerateHttpRequest>,
) -> Response {
    if let Err(e) = body.request.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }
    let resolved = match resolve_model(&state, &body.request) {
        Ok(r) => r,
        Err((status, message)) => return error_response(status, message),
    };
    let timeout_override = body
        .request
        .requirements
        .maximum_execution_ms
        .map(Duration::from_millis);

    let (job_id, events) = match state
        .runtime
        .submit(body.request, resolved, timeout_override)
        .await
    {
        Ok(v) => v,
        Err(QueueError::QueueFull) => {
            return error_response(StatusCode::TOO_MANY_REQUESTS, "worker queue is full")
        }
    };

    if body.stream {
        let chunk_stream = tokio_stream::wrappers::ReceiverStream::new(events).map(|chunk| {
            let payload = serde_json::to_string(&chunk).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("chunk").data(payload))
        });

        let runtime = state.runtime.clone();
        let final_stream = futures::stream::once(async move {
            loop {
                if let Some(record) = runtime.job(&job_id).await {
                    if let Some(response) = record.response {
                        let payload = serde_json::to_string(&response).unwrap_or_default();
                        return Ok::<_, Infallible>(
                            Event::default().event("response").data(payload),
                        );
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        Sse::new(chunk_stream.chain(final_stream)).into_response()
    } else {
        // Drain rather than drop: a dropped receiver looks like a disconnected
        // streaming client to the backend (see FakeBackend/LlamaCppBackend, which stop
        // producing tokens once `send` fails), which would truncate generation to
        // ~1 token for every non-streaming request. Draining keeps the channel open
        // so the job runs to completion; we just don't forward the events anywhere.
        tokio::spawn(async move {
            let mut events = events;
            while events.recv().await.is_some() {}
        });
        loop {
            if let Some(record) = state.runtime.job(&job_id).await {
                if let Some(response) = record.response {
                    return Json(response).into_response();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn embed_unsupported() -> Response {
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "embedding is not supported by this worker's backend",
    )
}

async fn rerank_unsupported() -> Response {
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "reranking is not supported by this worker's backend",
    )
}

/// Serve until a shutdown signal (Ctrl-C or SIGTERM) is received, then wait up to
/// `shutdown_grace` for in-flight jobs to finish before returning.
pub async fn serve(state: Arc<AppState>) -> std::io::Result<()> {
    let addr = state.config.bind_addr().expect("validated at config load");
    let shutdown_grace = state.config.shutdown_grace();
    let runtime = state.runtime.clone();
    let app = router(state.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(worker_id = %state.config.id, %addr, "worker listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("shutdown signal received, draining active jobs");
    let drain_start = std::time::Instant::now();
    while runtime.active_jobs().await > 0 && drain_start.elapsed() < shutdown_grace {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let remaining = runtime.active_jobs().await;
    if remaining > 0 {
        tracing::warn!(
            remaining,
            "shutdown grace period elapsed with jobs still active"
        );
    } else {
        tracing::info!("all jobs drained, shutting down cleanly");
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
