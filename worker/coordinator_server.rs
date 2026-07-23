//! HTTP server that wraps the coordinator, enabling homelab mode: deploy this to a
//! persistent host (e.g. HP Spectre) and have any number of clients route inference
//! through it without knowing which worker is available or best suited for the request.

use crate::coordinator::{Coordinator, CoordinatorStreamError};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use pig_core::model::{
    FinishReason, GenerationParameters, MessageRole, ModelChunk, ModelInstance, ModelInvoker,
    ModelMessage, ModelRequest, ModelRequirements, ModelResponseStatus, ModelRole, ModelSelector,
    ModelToolCall, ModelToolFunction, RequestId, RoutingExplanation, SchedulingOverrides, WorkerId,
    WorkerSnapshot,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::task;

pub struct CoordinatorServerState {
    coordinator: Arc<Coordinator>,
    pub started_at: Instant,
    pub auth_token: Option<String>,
}

impl CoordinatorServerState {
    pub fn new(coordinator: Arc<Coordinator>, auth_token: Option<String>) -> Self {
        Self {
            coordinator,
            started_at: Instant::now(),
            auth_token,
        }
    }
}

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

async fn auth_middleware(
    State(state): State<Arc<CoordinatorServerState>>,
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
struct CoordinatorHealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    workers_total: usize,
    workers_healthy: usize,
}

async fn health(State(state): State<Arc<CoordinatorServerState>>) -> Response {
    let snapshots = state.coordinator.async_snapshots().await;
    let workers_total = snapshots.len();
    let workers_healthy = snapshots.iter().filter(|s| s.healthy).count();
    Json(CoordinatorHealthResponse {
        status: if workers_healthy > 0 && workers_healthy == workers_total {
            "ok"
        } else if workers_healthy > 0 {
            "degraded"
        } else {
            "unavailable"
        },
        uptime_seconds: state.started_at.elapsed().as_secs(),
        workers_total,
        workers_healthy,
    })
    .into_response()
}

async fn workers_list(
    State(state): State<Arc<CoordinatorServerState>>,
) -> Json<Vec<WorkerSnapshot>> {
    let snapshots = state.coordinator.async_snapshots().await;
    Json(snapshots)
}

async fn instances_list(
    State(state): State<Arc<CoordinatorServerState>>,
) -> Json<Vec<ModelInstance>> {
    Json(state.coordinator.model_instances().await)
}

#[derive(Debug, Deserialize)]
struct GenerateBody {
    #[serde(flatten)]
    request: ModelRequest,
}

async fn generate(
    State(state): State<Arc<CoordinatorServerState>>,
    Json(body): Json<GenerateBody>,
) -> Response {
    if let Err(e) = body.request.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }
    let coord = Arc::clone(&state.coordinator);
    let request = body.request;
    match task::spawn_blocking(move || coord.invoke(request)).await {
        Ok(response) => {
            let worker_id = response.execution.worker_id.0.clone();
            let model_id = response.execution.model_id.0.clone();
            let mut resp = Json(response).into_response();
            let headers = resp.headers_mut();
            if let Ok(v) = worker_id.parse() {
                headers.insert("x-pig-worker-id", v);
            }
            if let Ok(v) = model_id.parse() {
                headers.insert("x-pig-model-id", v);
            }
            resp
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    tools: Vec<serde_json::Value>,
    tool_choice: Option<serde_json::Value>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    #[serde(default)]
    stop: Vec<String>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: OpenAiToolFunction,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiToolFunction {
    name: String,
    arguments: String,
}

fn openai_content(content: Option<serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(content)) => content,
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

fn normalize_openai_request(
    request: OpenAiChatRequest,
) -> Result<(String, ModelRequest, bool), String> {
    let messages = request
        .messages
        .into_iter()
        .map(|message| {
            let role = match message.role.as_str() {
                "system" | "developer" => MessageRole::System,
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                other => return Err(format!("unsupported message role '{}'", other)),
            };
            let tool_calls = message
                .tool_calls
                .into_iter()
                .map(|call| {
                    if call.kind != "function" {
                        return Err(format!("unsupported tool call type '{}'", call.kind));
                    }
                    Ok(ModelToolCall {
                        id: call.id,
                        function: ModelToolFunction {
                            name: call.function.name,
                            arguments: call.function.arguments,
                        },
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ModelMessage {
                role,
                content: openai_content(message.content),
                tool_calls,
                tool_call_id: message.tool_call_id,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let role = if let Some(alias) = request.model.strip_prefix("pig-") {
        ModelRole::parse(alias)
    } else {
        ModelRole::Reasoning
    };
    let model = if request.model.starts_with("pig-") {
        None
    } else {
        Some(ModelSelector::Alias(request.model.clone()))
    };
    let model_id = request.model.clone();
    Ok((
        model_id,
        ModelRequest {
            request_id: RequestId::generate(),
            role,
            model,
            messages,
            parameters: GenerationParameters {
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                stop: request.stop,
                tools: request.tools,
                tool_choice: request.tool_choice,
                ..Default::default()
            },
            requirements: Default::default(),
            inputs: vec![],
            metadata: Default::default(),
        },
        request.stream,
    ))
}

async fn openai_models(
    State(state): State<Arc<CoordinatorServerState>>,
) -> Json<serde_json::Value> {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Role aliases are always valid selectors regardless of what models are registered.
    let mut ids: Vec<String> = vec![
        "pig-coding".to_string(),
        "pig-reasoning".to_string(),
        "pig-verification".to_string(),
    ];
    for resolved in state.coordinator.registry().all_resolved() {
        ids.push(resolved.entry.id.0.clone());
    }
    Json(serde_json::json!({
        "object": "list",
        "data": ids.into_iter().map(|id| serde_json::json!({
            "id": id,
            "object": "model",
            "created": created,
            "owned_by": "pig"
        })).collect::<Vec<_>>()
    }))
}

#[derive(Debug, Deserialize)]
struct PipelineStep {
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    role: Option<ModelRole>,
    #[serde(default)]
    requirements: ModelRequirements,
    #[serde(default)]
    inject_previous: bool,
}

#[derive(Debug, Deserialize)]
struct PipelineRequest {
    steps: Vec<PipelineStep>,
    /// When true, all steps after the first are pinned to the worker that served step 0.
    #[serde(default)]
    session_affinity: bool,
}

#[derive(Debug, Serialize)]
struct PipelineStepOutput {
    step: usize,
    content: String,
}

#[derive(Debug, Serialize)]
struct PipelineResponse {
    steps: Vec<PipelineStepOutput>,
}

async fn pipeline(
    State(state): State<Arc<CoordinatorServerState>>,
    Json(body): Json<PipelineRequest>,
) -> Response {
    if body.steps.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "pipeline must have at least one step");
    }
    let session_affinity = body.session_affinity;
    let mut outputs: Vec<PipelineStepOutput> = Vec::with_capacity(body.steps.len());
    let mut previous_content: Option<String> = None;
    let mut pinned_worker: Option<WorkerId> = None;

    for (i, step) in body.steps.into_iter().enumerate() {
        let mut messages: Vec<ModelMessage> = Vec::new();
        if step.inject_previous {
            if let Some(ref prev) = previous_content {
                messages.push(ModelMessage {
                    role: MessageRole::Assistant,
                    content: prev.clone(),
                    tool_calls: vec![],
                    tool_call_id: None,
                });
            }
        }
        for msg in step.messages {
            let role = match msg.role.as_str() {
                "system" | "developer" => MessageRole::System,
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                other => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!("step {i}: unsupported message role '{other}'"),
                    )
                }
            };
            messages.push(ModelMessage {
                role,
                content: openai_content(msg.content),
                tool_calls: vec![],
                tool_call_id: msg.tool_call_id,
            });
        }

        let request = ModelRequest {
            request_id: RequestId::generate(),
            role: step.role.unwrap_or(ModelRole::Reasoning),
            model: None,
            messages,
            parameters: GenerationParameters::default(),
            requirements: step.requirements,
            inputs: vec![],
            metadata: Default::default(),
        };
        if let Err(e) = request.validate() {
            return error_response(StatusCode::BAD_REQUEST, format!("step {i}: {e}"));
        }

        let coord = Arc::clone(&state.coordinator);
        let overrides = SchedulingOverrides {
            force_worker: pinned_worker.clone(),
            ..Default::default()
        };
        let response =
            match task::spawn_blocking(move || coord.invoke_with_overrides(request, overrides))
                .await
            {
                Ok(r) => r,
                Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
        if response.status != ModelResponseStatus::Success {
            let msg = response
                .error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "inference failed".to_string());
            return error_response(StatusCode::BAD_GATEWAY, format!("step {i}: {msg}"));
        }

        if session_affinity && pinned_worker.is_none() {
            pinned_worker = Some(response.execution.worker_id.clone());
        }

        let content = match response.output {
            pig_core::artifact::Artifact::Text(text) => text,
            other => serde_json::to_string(&other).unwrap_or_default(),
        };
        previous_content = Some(content.clone());
        outputs.push(PipelineStepOutput { step: i, content });
    }

    Json(PipelineResponse { steps: outputs }).into_response()
}

async fn openai_chat_completions(
    State(state): State<Arc<CoordinatorServerState>>,
    Json(body): Json<OpenAiChatRequest>,
) -> Response {
    let (requested_model, request, stream) = match normalize_openai_request(body) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    if let Err(error) = request.validate() {
        return error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    if stream {
        let completion_id = format!("chatcmpl-{}", request.request_id.0);
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let coordinator = Arc::clone(&state.coordinator);
        let chunks = match coordinator.stream(request).await {
            Ok(chunks) => chunks,
            Err(CoordinatorStreamError::ToolsUnsupported) => {
                return error_response(
                    StatusCode::NOT_IMPLEMENTED,
                    "the selected pig worker cannot honor requested tool calling",
                );
            }
            Err(error) => return error_response(StatusCode::BAD_GATEWAY, error.to_string()),
        };
        return openai_sse_response(chunks, completion_id, requested_model, created);
    }
    let coordinator = Arc::clone(&state.coordinator);
    if !request.parameters.tools.is_empty() {
        let request_for_capabilities = request.clone();
        let supports_tools = task::spawn_blocking(move || {
            coordinator.selected_worker_supports_tools(&request_for_capabilities)
        })
        .await
        .unwrap_or(false);
        if !supports_tools {
            return error_response(
                StatusCode::NOT_IMPLEMENTED,
                "the selected pig worker cannot honor requested tool calling",
            );
        }
    }
    let coordinator = Arc::clone(&state.coordinator);
    match task::spawn_blocking(move || coordinator.invoke(request)).await {
        Ok(response) if response.status == ModelResponseStatus::Success => {
            let content = match response.output {
                pig_core::artifact::Artifact::Text(text) => text,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            let finish_reason = match response.finish_reason {
                FinishReason::Length => "length",
                FinishReason::ToolCalls => "tool_calls",
                _ => {
                    if response.tool_calls.is_empty() {
                        "stop"
                    } else {
                        "tool_calls"
                    }
                }
            };
            let tool_calls = response.tool_calls.into_iter().map(|call| serde_json::json!({"id":call.id,"type":"function","function":{"name":call.function.name,"arguments":call.function.arguments}})).collect::<Vec<_>>();
            Json(serde_json::json!({"id": format!("chatcmpl-{}", response.request_id.0), "object":"chat.completion", "model": requested_model, "choices":[{"index":0,"message":{"role":"assistant","content":content,"tool_calls":tool_calls},"finish_reason":finish_reason}],"usage":{"prompt_tokens":response.usage.prompt_tokens,"completion_tokens":response.usage.completion_tokens,"total_tokens":response.usage.total_tokens}})).into_response()
        }
        Ok(response) => error_response(
            StatusCode::BAD_GATEWAY,
            response
                .error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "inference failed".to_string()),
        ),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn openai_sse_response(
    chunks: crate::coordinator::ModelChunkStream,
    completion_id: String,
    model: String,
    created: u64,
) -> Response {
    let initial = openai_sse_event(
        &completion_id,
        &model,
        created,
        serde_json::json!({"role":"assistant"}),
        None,
    );
    let stream_id = completion_id.clone();
    let stream_model = model.clone();
    let relayed = chunks.map(move |chunk| {
        let event = match chunk {
            Ok(ModelChunk::TextDelta { text }) => openai_sse_event(
                &stream_id,
                &stream_model,
                created,
                serde_json::json!({"content":text}),
                None,
            ),
            Ok(ModelChunk::ToolCallDelta {
                index,
                id,
                function_name,
                arguments_delta,
            }) => {
                let mut call = serde_json::json!({"index":index,"type":"function","function":{}});
                if let Some(id) = id {
                    call["id"] = serde_json::json!(id);
                }
                if let Some(name) = function_name {
                    call["function"]["name"] = serde_json::json!(name);
                }
                if let Some(arguments) = arguments_delta {
                    call["function"]["arguments"] = serde_json::json!(arguments);
                }
                openai_sse_event(
                    &stream_id,
                    &stream_model,
                    created,
                    serde_json::json!({"tool_calls":[call]}),
                    None,
                )
            }
            Ok(ModelChunk::Finished { finish_reason, .. }) => openai_sse_event(
                &stream_id,
                &stream_model,
                created,
                serde_json::json!({}),
                Some(openai_finish_reason(finish_reason)),
            ),
            Err(error) => Event::default()
                .event("error")
                .data(serde_json::json!({"error":{"message":error}}).to_string()),
        };
        Ok::<_, Infallible>(event)
    });
    let done =
        futures::stream::once(async { Ok::<_, Infallible>(Event::default().data("[DONE]")) });
    Sse::new(
        futures::stream::once(async move { Ok::<_, Infallible>(initial) })
            .chain(relayed)
            .chain(done),
    )
    .into_response()
}

fn openai_sse_event(
    completion_id: &str,
    model: &str,
    created: u64,
    delta: serde_json::Value,
    finish_reason: Option<&str>,
) -> Event {
    Event::default()
        .data(openai_sse_payload(completion_id, model, created, delta, finish_reason).to_string())
}

fn openai_sse_payload(
    completion_id: &str,
    model: &str,
    created: u64,
    delta: serde_json::Value,
    finish_reason: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": completion_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    })
}

fn openai_finish_reason(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Cancelled | FinishReason::TimedOut | FinishReason::Error => "stop",
        FinishReason::Stop => "stop",
    }
}

async fn route_explain(
    State(state): State<Arc<CoordinatorServerState>>,
    Json(body): Json<GenerateBody>,
) -> Json<RoutingExplanation> {
    let coord = Arc::clone(&state.coordinator);
    let request = body.request;
    let explanation =
        task::spawn_blocking(move || coord.route(&request, &SchedulingOverrides::default()))
            .await
            .unwrap_or_else(|_| RoutingExplanation {
                selected: None,
                rejected: vec![],
                all_candidates: vec![],
            });
    Json(explanation)
}

#[derive(Debug, Deserialize)]
struct WorkerQuery {
    worker: String,
}

async fn jobs_list(
    State(state): State<Arc<CoordinatorServerState>>,
    Query(query): Query<WorkerQuery>,
) -> Response {
    let coordinator = Arc::clone(&state.coordinator);
    match task::spawn_blocking(move || {
        coordinator.job_request(&query.worker, reqwest::Method::GET, "v1/jobs")
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => error_response(StatusCode::BAD_GATEWAY, error),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn job_inspect(
    State(state): State<Arc<CoordinatorServerState>>,
    Path(job_id): Path<String>,
    Query(query): Query<WorkerQuery>,
) -> Response {
    let coordinator = Arc::clone(&state.coordinator);
    match task::spawn_blocking(move || {
        coordinator.job_request(
            &query.worker,
            reqwest::Method::GET,
            &format!("v1/jobs/{}", job_id),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => error_response(StatusCode::BAD_GATEWAY, error),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn job_cancel(
    State(state): State<Arc<CoordinatorServerState>>,
    Path(job_id): Path<String>,
    Query(query): Query<WorkerQuery>,
) -> Response {
    let coordinator = Arc::clone(&state.coordinator);
    match task::spawn_blocking(move || {
        coordinator.job_request(
            &query.worker,
            reqwest::Method::POST,
            &format!("v1/jobs/{}/cancel", job_id),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => error_response(StatusCode::BAD_GATEWAY, error),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

pub fn router(state: Arc<CoordinatorServerState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/workers", get(workers_list))
        .route("/v1/instances", get(instances_list))
        .route("/v1/generate", post(generate))
        .route("/v1/route", post(route_explain))
        .route("/v1/models", get(openai_models))
        .route("/v1/pipeline", post(pipeline))
        .route("/v1/chat/completions", post(openai_chat_completions))
        .route("/v1/jobs", get(jobs_list))
        .route("/v1/jobs/:job_id", get(job_inspect))
        .route("/v1/jobs/:job_id/cancel", post(job_cancel))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// Serve until a shutdown signal (Ctrl-C or SIGTERM), then return.
pub async fn serve(state: Arc<CoordinatorServerState>, bind: &str) -> std::io::Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "coordinator listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
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

#[cfg(test)]
mod openai_tests {
    use super::*;

    #[test]
    fn logical_coding_model_normalizes_to_a_coding_request() {
        let (_, request, stream) = normalize_openai_request(OpenAiChatRequest {
            model: "pig-coding".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(serde_json::json!("fix the test")),
                tool_calls: vec![],
                tool_call_id: None,
            }],
            tools: vec![],
            tool_choice: None,
            max_tokens: Some(64),
            temperature: Some(0.2),
            stop: vec![],
            stream: false,
        })
        .unwrap();
        assert_eq!(request.role, ModelRole::Coding);
        assert!(request.model.is_none());
        assert!(!stream);
    }

    #[test]
    fn tool_definitions_are_preserved_for_the_backend() {
        let tool = serde_json::json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}});
        let (_, request, _) = normalize_openai_request(OpenAiChatRequest {
            model: "pig-coding".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(serde_json::json!("inspect")),
                tool_calls: vec![],
                tool_call_id: None,
            }],
            tools: vec![tool.clone()],
            tool_choice: Some(serde_json::json!("auto")),
            max_tokens: None,
            temperature: None,
            stop: vec![],
            stream: false,
        })
        .unwrap();
        assert_eq!(request.parameters.tools, vec![tool]);
        assert_eq!(
            request.parameters.tool_choice,
            Some(serde_json::json!("auto"))
        );
    }

    #[test]
    fn assistant_tool_calls_and_tool_results_preserve_order_and_ids() {
        let (_, request, _) = normalize_openai_request(OpenAiChatRequest {
            model: "pig-coding".to_string(),
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: Some(serde_json::json!("Use tools when needed.")),
                    tool_calls: vec![],
                    tool_call_id: None,
                },
                OpenAiMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: vec![
                        OpenAiToolCall {
                            id: "call_read".to_string(),
                            kind: "function".to_string(),
                            function: OpenAiToolFunction {
                                name: "read_file".to_string(),
                                arguments: r#"{"path":"src/lib.rs"}"#.to_string(),
                            },
                        },
                        OpenAiToolCall {
                            id: "call_search".to_string(),
                            kind: "function".to_string(),
                            function: OpenAiToolFunction {
                                name: "search_code".to_string(),
                                arguments: r#"{"query":"ModelRequest"}"#.to_string(),
                            },
                        },
                    ],
                    tool_call_id: None,
                },
                OpenAiMessage {
                    role: "tool".to_string(),
                    content: Some(serde_json::json!("pub struct ModelRequest { ... }")),
                    tool_calls: vec![],
                    tool_call_id: Some("call_read".to_string()),
                },
                OpenAiMessage {
                    role: "tool".to_string(),
                    content: Some(serde_json::json!("src/lib.rs: ModelRequest")),
                    tool_calls: vec![],
                    tool_call_id: Some("call_search".to_string()),
                },
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            stop: vec![],
            stream: false,
        })
        .unwrap();

        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[1].role, MessageRole::Assistant);
        assert_eq!(request.messages[1].tool_calls.len(), 2);
        assert_eq!(request.messages[1].tool_calls[0].id, "call_read");
        assert_eq!(
            request.messages[1].tool_calls[0].function.arguments,
            r#"{"path":"src/lib.rs"}"#
        );
        assert_eq!(request.messages[1].tool_calls[1].id, "call_search");
        assert_eq!(request.messages[2].role, MessageRole::Tool);
        assert_eq!(
            request.messages[2].tool_call_id.as_deref(),
            Some("call_read")
        );
        assert_eq!(
            request.messages[3].tool_call_id.as_deref(),
            Some("call_search")
        );

        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: ModelRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn non_function_tool_calls_are_rejected_during_normalization() {
        let error = normalize_openai_request(OpenAiChatRequest {
            model: "pig-coding".to_string(),
            messages: vec![OpenAiMessage {
                role: "assistant".to_string(),
                content: None,
                tool_calls: vec![OpenAiToolCall {
                    id: "call_1".to_string(),
                    kind: "custom".to_string(),
                    function: OpenAiToolFunction {
                        name: "ignored".to_string(),
                        arguments: String::new(),
                    },
                }],
                tool_call_id: None,
            }],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            stop: vec![],
            stream: false,
        })
        .unwrap_err();

        assert!(error.contains("unsupported tool call type 'custom'"));
    }

    #[test]
    fn openai_stream_finish_reasons_keep_tool_calls_distinct() {
        assert_eq!(openai_finish_reason(FinishReason::Stop), "stop");
        assert_eq!(openai_finish_reason(FinishReason::Length), "length");
        assert_eq!(openai_finish_reason(FinishReason::ToolCalls), "tool_calls");
    }

    #[test]
    fn openai_stream_payload_keeps_tool_call_deltas_structured() {
        let payload = openai_sse_payload(
            "chatcmpl-test",
            "pig-coding",
            123,
            serde_json::json!({"tool_calls":[{
                "index": 1,
                "id": "call_read",
                "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\":"}
            }]}),
            None,
        );
        assert_eq!(payload["object"], "chat.completion.chunk");
        assert_eq!(payload["choices"][0]["delta"]["tool_calls"][0]["index"], 1);
        assert_eq!(
            payload["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            r#"{"path":"#
        );
    }
}
