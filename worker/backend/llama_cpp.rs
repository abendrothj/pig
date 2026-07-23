//! Supervises `llama-server` as a subprocess and talks to its OpenAI-compatible HTTP
//! API. LAO never links llama.cpp into its own process — this backend launches the
//! real executable directly (never a shell), waits for it to report ready, and
//! forwards generation requests to it.
//!
//! One `llama-server` process is supervised at a time per backend instance: loading a
//! different model stops the previous server and starts a new one. Running several
//! models concurrently is a job for several workers (see the scheduler), not several
//! servers behind one backend — this keeps process ownership unambiguous ("one
//! authoritative owner for each child process").

use super::{
    BackendCapabilities, BackendError, BackendGenerationRequest, BackendGenerationResponse,
    BackendHealth, LoadModelRequest, LoadedModel, ModelAvailability, ModelBackend,
    ModelEventSender,
};
use async_trait::async_trait;
use futures::StreamExt;
use lao_orchestrator_core::model::{
    AcceleratorKind, FinishReason, MessageRole, ModelChunk, ModelId, ModelToolCall,
    ModelToolFunction, ReasoningMode,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const STDERR_CAP_BYTES: usize = 64 * 1024;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlamaCppExecutionConfig {
    pub context_size: Option<u32>,
    pub cpu_threads: Option<u32>,
    pub cpu_threads_batch: Option<u32>,
    pub gpu_layers: Option<i32>,
    pub batch_size: Option<u32>,
    pub micro_batch_size: Option<u32>,
    pub flash_attention: Option<bool>,
    pub mmap: Option<bool>,
    pub mlock: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LlamaCppConfig {
    pub server_executable: PathBuf,
    pub host: String,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            server_executable: PathBuf::from("llama-server"),
            host: "127.0.0.1".to_string(),
            startup_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(600),
        }
    }
}

struct SupervisedServer {
    child: Child,
    model_id: ModelId,
    base_url: String,
    stderr_tail: Arc<Mutex<String>>,
    loaded: LoadedModel,
}

pub struct LlamaCppBackend {
    config: LlamaCppConfig,
    client: reqwest::Client,
    server: Mutex<Option<SupervisedServer>>,
    help_text: Mutex<Option<String>>,
}

impl LlamaCppBackend {
    pub fn new(config: LlamaCppConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            server: Mutex::new(None),
            help_text: Mutex::new(None),
        }
    }

    async fn executable_help(&self) -> String {
        let mut cached = self.help_text.lock().await;
        if let Some(text) = cached.as_ref() {
            return text.clone();
        }
        let text = Command::new(&self.config.server_executable)
            .arg("--help")
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        *cached = Some(text.clone());
        text
    }

    async fn executable_version(&self) -> Option<String> {
        let output = Command::new(&self.config.server_executable)
            .arg("--version")
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // "version: 9960 (a935fbffe)\nbuilt with ..." -> "9960 (a935fbffe)"
        text.lines()
            .next()
            .and_then(|l| l.strip_prefix("version:"))
            .map(|s| s.trim().to_string())
    }

    fn find_free_port() -> Result<u16, BackendError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| {
            BackendError::Internal(format!("failed to reserve a local port: {}", e))
        })?;
        listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| BackendError::Internal(format!("failed to read reserved port: {}", e)))
    }

    /// Only pass flags the installed executable actually advertises in `--help`,
    /// rather than assuming every build supports the same surface.
    async fn build_args(
        &self,
        model_path: &std::path::Path,
        port: u16,
        execution: &LlamaCppExecutionConfig,
    ) -> Vec<String> {
        let help = self.executable_help().await;
        let supports = |flag: &str| help.contains(flag);

        let mut args = vec![
            "-m".to_string(),
            model_path.to_string_lossy().into_owned(),
            "--host".to_string(),
            self.config.host.clone(),
            "--port".to_string(),
            port.to_string(),
        ];

        if let Some(v) = execution.context_size {
            if supports("--ctx-size") {
                args.push("-c".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(v) = execution.cpu_threads {
            if supports("--threads") {
                args.push("-t".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(v) = execution.cpu_threads_batch {
            if supports("--threads-batch") {
                args.push("-tb".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(v) = execution.gpu_layers {
            if supports("--gpu-layers") {
                args.push("-ngl".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(v) = execution.batch_size {
            if supports("--batch-size") {
                args.push("-b".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(v) = execution.micro_batch_size {
            if supports("--ubatch-size") {
                args.push("-ub".to_string());
                args.push(v.to_string());
            }
        }
        if let Some(on) = execution.flash_attention {
            if supports("--flash-attn") {
                args.push("--flash-attn".to_string());
                args.push(if on { "on" } else { "off" }.to_string());
            }
        }
        if let Some(mmap) = execution.mmap {
            if !mmap && supports("--no-mmap") {
                args.push("--no-mmap".to_string());
            }
        }
        if execution.mlock == Some(true) && supports("--mlock") {
            args.push("--mlock".to_string());
        }

        args
    }

    async fn detect_accelerator(&self) -> Option<AcceleratorKind> {
        let output = Command::new(&self.config.server_executable)
            .arg("--list-devices")
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("CUDA") {
            Some(AcceleratorKind::Cuda)
        } else if text.contains("MTL") || text.contains("Metal") {
            Some(AcceleratorKind::Metal)
        } else if text.contains("Vulkan") {
            Some(AcceleratorKind::Vulkan)
        } else if text.contains("ROCm") {
            Some(AcceleratorKind::Rocm)
        } else {
            None
        }
    }

    async fn stop_current(&self, guard: &mut Option<SupervisedServer>) {
        if let Some(mut server) = guard.take() {
            let _ = server.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), server.child.wait()).await;
        }
    }
}

/// Injects a Qwen3 reasoning control token into the message list.
///
/// When the final message is a user turn, the token is appended inline — the
/// canonical position Qwen3 is trained on. When the final message is a tool
/// result (agentic continuation), modifying an already-sent user message would
/// be semantically wrong; instead the token is injected via the system message
/// so the directive applies to this completion only.
fn apply_reasoning_mode(messages: &mut Vec<serde_json::Value>, mode: ReasoningMode) {
    let (inline_token, system_token) = match mode {
        ReasoningMode::Enabled => (" /think", "/think"),
        ReasoningMode::Disabled => (" /no_think", "/no_think"),
        ReasoningMode::Auto => return,
    };

    let last_is_user = messages
        .last()
        .and_then(|m| m.get("role").and_then(|r| r.as_str()))
        == Some("user");

    if last_is_user {
        let m = messages.last_mut().unwrap();
        if let Some(c) = m.get("content").and_then(|c| c.as_str()) {
            let new = format!("{}{}", c, inline_token);
            m["content"] = serde_json::json!(new);
        }
    } else {
        // Tool continuation: inject via system message to avoid retroactively
        // modifying a historical user message.
        if let Some(sys) = messages
            .iter_mut()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        {
            if let Some(c) = sys.get("content").and_then(|c| c.as_str()) {
                let new = format!("{}\n{}", c, system_token);
                sys["content"] = serde_json::json!(new);
            }
        } else {
            messages.insert(
                0,
                serde_json::json!({"role": "system", "content": system_token}),
            );
        }
    }
}

fn openai_messages(
    messages: &[lao_orchestrator_core::model::ModelMessage],
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            let mut value = serde_json::json!({"role": role, "content": message.content});
            if !message.tool_calls.is_empty() {
                value["tool_calls"] = serde_json::json!(message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.function.name,
                                "arguments": call.function.arguments,
                            },
                        })
                    })
                    .collect::<Vec<_>>());
            }
            if let Some(tool_call_id) = &message.tool_call_id {
                value["tool_call_id"] = serde_json::json!(tool_call_id);
            }
            value
        })
        .collect()
}

#[async_trait]
impl ModelBackend for LlamaCppBackend {
    async fn health(&self) -> Result<BackendHealth, BackendError> {
        let version = self.executable_version().await;
        let output = Command::new(&self.config.server_executable)
            .arg("--version")
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => Ok(BackendHealth {
                available: true,
                detail: format!("{}", self.config.server_executable.display()),
                version,
            }),
            Ok(o) => Ok(BackendHealth {
                available: false,
                detail: format!("executable exited with status {:?}", o.status.code()),
                version: None,
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BackendHealth {
                available: false,
                detail: format!("{} not found", self.config.server_executable.display()),
                version: None,
            }),
            Err(e) => Ok(BackendHealth {
                available: false,
                detail: e.to_string(),
                version: None,
            }),
        }
    }

    async fn capabilities(&self) -> Result<BackendCapabilities, BackendError> {
        let version = self.executable_version().await;
        // Read the accelerator from the running server rather than probing via
        // --list-devices: that probe fails when llama-server already holds the GPU.
        let from_server = self
            .server
            .lock()
            .await
            .as_ref()
            .and_then(|s| s.loaded.accelerator.clone());
        let accelerator = match from_server {
            Some(a) => Some(a),
            None => self.detect_accelerator().await,
        };
        Ok(BackendCapabilities {
            backend: "llama_cpp".to_string(),
            version,
            accelerators: accelerator
                .into_iter()
                .chain(std::iter::once(AcceleratorKind::Cpu))
                .collect(),
            supports_streaming: true,
            supports_tools: true,
            supports_embedding: false,
            supports_reranking: false,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelAvailability>, BackendError> {
        let guard = self.server.lock().await;
        Ok(guard
            .as_ref()
            .map(|s| {
                vec![ModelAvailability {
                    model_id: s.model_id.clone(),
                    path: None,
                    loaded: true,
                }]
            })
            .unwrap_or_default())
    }

    async fn load_model(&self, request: LoadModelRequest) -> Result<LoadedModel, BackendError> {
        let mut guard = self.server.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.model_id == request.model_id {
                let mut reused = existing.loaded.clone();
                reused.already_loaded = true;
                return Ok(reused);
            }
        }

        if !request.path.is_file() {
            return Err(BackendError::LoadFailed(format!(
                "model file not found: {}",
                request.path.display()
            )));
        }

        let execution: LlamaCppExecutionConfig = if request.execution_config.is_null() {
            LlamaCppExecutionConfig::default()
        } else {
            serde_json::from_value(request.execution_config.clone())
                .map_err(|e| BackendError::LoadFailed(format!("invalid execution_config: {}", e)))?
        };

        self.stop_current(&mut guard).await;

        let port = Self::find_free_port()?;
        let args = self.build_args(&request.path, port, &execution).await;

        let mut command = Command::new(&self.config.server_executable);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let start = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|e| BackendError::LoadFailed(format!("failed to spawn server: {}", e)))?;

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut buf = tail.lock().await;
                    buf.push_str(&line);
                    buf.push('\n');
                    if buf.len() > STDERR_CAP_BYTES {
                        let excess = buf.len() - STDERR_CAP_BYTES;
                        buf.drain(0..excess);
                    }
                }
            });
        }

        let base_url = format!("http://{}:{}", self.config.host, port);
        let health_url = format!("{}/health", base_url);

        loop {
            if start.elapsed() > self.config.startup_timeout {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let stderr = stderr_tail.lock().await.clone();
                return Err(BackendError::LoadFailed(format!(
                    "server did not become ready within {:?}; stderr tail:\n{}",
                    self.config.startup_timeout, stderr
                )));
            }
            if let Ok(Some(status)) = child.try_wait() {
                let stderr = stderr_tail.lock().await.clone();
                return Err(BackendError::LoadFailed(format!(
                    "server exited during startup (status {:?}); stderr tail:\n{}",
                    status.code(),
                    stderr
                )));
            }
            if let Ok(resp) = self.client.get(&health_url).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }

        let load_ms = start.elapsed().as_millis() as u64;
        let accelerator = self.detect_accelerator().await;
        let loaded = LoadedModel {
            model_id: request.model_id.clone(),
            context_tokens: execution.context_size,
            already_loaded: false,
            load_ms,
            accelerator,
            cpu_threads: execution.cpu_threads,
            gpu_layers: execution.gpu_layers,
            batch_size: execution.batch_size,
        };

        *guard = Some(SupervisedServer {
            child,
            model_id: request.model_id,
            base_url,
            stderr_tail,
            loaded: loaded.clone(),
        });

        Ok(loaded)
    }

    async fn unload_model(&self, model: &ModelId) -> Result<(), BackendError> {
        let mut guard = self.server.lock().await;
        if guard.as_ref().map(|s| &s.model_id) == Some(model) {
            self.stop_current(&mut guard).await;
        }
        Ok(())
    }

    async fn generate(
        &self,
        request: BackendGenerationRequest,
        events: ModelEventSender,
        cancellation: CancellationToken,
    ) -> Result<BackendGenerationResponse, BackendError> {
        let (base_url, _stderr_tail) = {
            let guard = self.server.lock().await;
            let server = guard
                .as_ref()
                .filter(|s| s.model_id == request.model_id)
                .ok_or_else(|| BackendError::ModelNotFound(request.model_id.0.clone()))?;
            (server.base_url.clone(), server.stderr_tail.clone())
        };

        let mut messages = openai_messages(&request.messages);
        apply_reasoning_mode(&mut messages, request.parameters.reasoning_mode);

        let mut body = serde_json::json!({
            "messages": messages,
            "stream": true,
        });
        let p = &request.parameters;
        if let Some(v) = p.max_tokens {
            body["max_tokens"] = serde_json::json!(v);
        }
        if let Some(v) = p.temperature {
            body["temperature"] = serde_json::json!(v);
        }
        if let Some(v) = p.top_p {
            body["top_p"] = serde_json::json!(v);
        }
        if let Some(v) = p.top_k {
            body["top_k"] = serde_json::json!(v);
        }
        if let Some(v) = p.min_p {
            body["min_p"] = serde_json::json!(v);
        }
        if let Some(v) = p.seed {
            body["seed"] = serde_json::json!(v);
        }
        if !p.stop.is_empty() {
            body["stop"] = serde_json::json!(p.stop);
        }
        if !p.tools.is_empty() {
            body["tools"] = serde_json::json!(p.tools);
        }
        if let Some(tool_choice) = &p.tool_choice {
            body["tool_choice"] = tool_choice.clone();
        }

        let url = format!("{}/v1/chat/completions", base_url);
        let request_fut = self.client.post(&url).json(&body).send();

        let response = tokio::select! {
            result = request_fut => result.map_err(|e| BackendError::GenerationFailed(e.to_string()))?,
            _ = cancellation.cancelled() => return Err(BackendError::Cancelled),
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(BackendError::GenerationFailed(format!(
                "server returned {}: {}",
                status, text
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut finish_reason = FinishReason::Stop;
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        let mut prompt_ms = 0u64;
        let mut generation_ms = 0u64;
        let mut prompt_tps = None;
        let mut gen_tps = None;
        let mut tool_call_fragments = BTreeMap::new();

        loop {
            let chunk = tokio::select! {
                chunk = stream.next() => chunk,
                _ = cancellation.cancelled() => return Err(BackendError::Cancelled),
            };
            let Some(chunk) = chunk else { break };
            let bytes = chunk.map_err(|e| BackendError::GenerationFailed(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);
                let Some(payload) = line.strip_prefix("data: ") else {
                    continue;
                };
                if payload.trim() == "[DONE]" {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                    continue;
                };

                if let Some(choice) = value.get("choices").and_then(|c| c.get(0)) {
                    // Reasoning models (e.g. Qwen3) emit chain-of-thought under a
                    // separate `reasoning_content` field, distinct from `content` -
                    // confirmed against the real installed llama-server. Treating only
                    // `content` as the output means a "thinking" model can hit
                    // finish_reason=length having produced only reasoning tokens,
                    // yielding a silently empty response. Both are captured as output.
                    let delta = choice.get("delta");
                    let delta_text = delta
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                        .or_else(|| {
                            delta
                                .and_then(|d| d.get("reasoning_content"))
                                .and_then(|c| c.as_str())
                        });
                    if let Some(delta_text) = delta_text {
                        content.push_str(delta_text);
                        completion_tokens += 1;
                        let _ = events
                            .send(ModelChunk::TextDelta {
                                text: delta_text.to_string(),
                            })
                            .await;
                    }
                    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        finish_reason = match reason {
                            "length" => FinishReason::Length,
                            "stop" => FinishReason::Stop,
                            "tool_calls" => FinishReason::ToolCalls,
                            _ => FinishReason::Stop,
                        };
                    }
                    if let Some(calls) = delta
                        .and_then(|value| value.get("tool_calls"))
                        .and_then(|value| value.as_array())
                    {
                        for (fallback_index, call) in calls.iter().enumerate() {
                            let _ = events.send(tool_call_delta(call, fallback_index)).await;
                        }
                        absorb_tool_call_fragments(&mut tool_call_fragments, calls);
                    }
                }
                if let Some(usage) = value.get("usage") {
                    prompt_tokens = usage
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(prompt_tokens as u64) as u32;
                    completion_tokens = usage
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(completion_tokens as u64)
                        as u32;
                }
                if let Some(timings) = value.get("timings") {
                    prompt_ms = timings
                        .get("prompt_ms")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u64;
                    generation_ms = timings
                        .get("predicted_ms")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u64;
                    prompt_tps = timings.get("prompt_per_second").and_then(|v| v.as_f64());
                    gen_tps = timings.get("predicted_per_second").and_then(|v| v.as_f64());
                    // This build of llama-server does not include a `usage` object in
                    // streaming responses (confirmed live) - `timings.{prompt,predicted}_n`
                    // are the authoritative token counts in that case, and arrive in the
                    // same final chunk as `finish_reason`.
                    if let Some(n) = timings.get("prompt_n").and_then(|v| v.as_u64()) {
                        prompt_tokens = n as u32;
                    }
                    if let Some(n) = timings.get("predicted_n").and_then(|v| v.as_u64()) {
                        completion_tokens = n as u32;
                    }
                }
            }
        }

        Ok(BackendGenerationResponse {
            finish_reason,
            content,
            prompt_tokens,
            completion_tokens,
            prompt_ms,
            generation_ms,
            prompt_tokens_per_second: prompt_tps,
            generation_tokens_per_second: gen_tps,
            tool_calls: finish_tool_call_fragments(tool_call_fragments),
        })
    }
}

#[derive(Default)]
struct ToolCallFragments {
    id: Option<String>,
    name: String,
    arguments: String,
}

/// OpenAI-compatible streaming responses may split a call across several deltas.
/// Keep those pieces structured until the completed worker response is assembled.
fn absorb_tool_call_fragments(
    fragments: &mut BTreeMap<usize, ToolCallFragments>,
    calls: &[serde_json::Value],
) {
    for (fallback_index, call) in calls.iter().enumerate() {
        let index = call
            .get("index")
            .and_then(|index| index.as_u64())
            .map(|index| index as usize)
            .unwrap_or(fallback_index);
        let fragment = fragments.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(|id| id.as_str()) {
            fragment.id = Some(id.to_string());
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(|name| name.as_str()) {
                fragment.name.push_str(name);
            }
            if let Some(arguments) = function.get("arguments").and_then(|value| value.as_str()) {
                fragment.arguments.push_str(arguments);
            }
        }
    }
}

fn tool_call_delta(value: &serde_json::Value, fallback_index: usize) -> ModelChunk {
    let index = value
        .get("index")
        .and_then(|index| index.as_u64())
        .map(|index| index as usize)
        .unwrap_or(fallback_index);
    let function = value.get("function");
    ModelChunk::ToolCallDelta {
        index,
        id: value
            .get("id")
            .and_then(|id| id.as_str())
            .map(str::to_string),
        function_name: function
            .and_then(|function| function.get("name"))
            .and_then(|name| name.as_str())
            .map(str::to_string),
        arguments_delta: function
            .and_then(|function| function.get("arguments"))
            .and_then(|arguments| arguments.as_str())
            .map(str::to_string),
    }
}

fn finish_tool_call_fragments(fragments: BTreeMap<usize, ToolCallFragments>) -> Vec<ModelToolCall> {
    fragments
        .into_values()
        .filter_map(|fragment| {
            Some(ModelToolCall {
                id: fragment.id?,
                function: ModelToolFunction {
                    name: fragment.name,
                    arguments: fragment.arguments,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lao_orchestrator_core::model::{ModelMessage, ModelToolCall, ModelToolFunction};

    #[test]
    fn execution_config_round_trips_through_json() {
        let cfg = LlamaCppExecutionConfig {
            context_size: Some(16384),
            cpu_threads: Some(10),
            cpu_threads_batch: Some(10),
            gpu_layers: Some(99),
            batch_size: Some(2048),
            micro_batch_size: Some(512),
            flash_attention: Some(true),
            mmap: Some(true),
            mlock: Some(false),
        };
        let value = serde_json::to_value(&cfg).unwrap();
        let back: LlamaCppExecutionConfig = serde_json::from_value(value).unwrap();
        assert_eq!(cfg.context_size, back.context_size);
        assert_eq!(cfg.gpu_layers, back.gpu_layers);
    }

    #[tokio::test]
    async fn health_reports_unavailable_for_a_missing_executable() {
        let backend = LlamaCppBackend::new(LlamaCppConfig {
            server_executable: PathBuf::from("/definitely/not/a/real/llama-server-binary"),
            ..Default::default()
        });
        let health = backend.health().await.unwrap();
        assert!(!health.available);
    }

    #[test]
    fn typed_tool_history_is_sent_as_structured_openai_messages() {
        let messages = openai_messages(&[
            ModelMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: vec![ModelToolCall {
                    id: "call_lookup".to_string(),
                    function: ModelToolFunction {
                        name: "lookup".to_string(),
                        arguments: r#"{"id":42}"#.to_string(),
                    },
                }],
                tool_call_id: None,
            },
            ModelMessage {
                role: MessageRole::Tool,
                content: r#"{"name":"LAO"}"#.to_string(),
                tool_calls: vec![],
                tool_call_id: Some("call_lookup".to_string()),
            },
        ]);

        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call_lookup");
        assert_eq!(messages[0]["tool_calls"][0]["type"], "function");
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["arguments"],
            r#"{"id":42}"#
        );
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_lookup");
    }

    #[test]
    fn streamed_tool_call_fragments_are_reassembled_by_index() {
        let mut fragments = BTreeMap::new();
        absorb_tool_call_fragments(
            &mut fragments,
            &[serde_json::json!({
                "index": 0,
                "id": "call_1",
                "function": {"name": "read", "arguments": "{\"path\":"}
            })],
        );
        absorb_tool_call_fragments(
            &mut fragments,
            &[serde_json::json!({
                "index": 0,
                "function": {"arguments": "\"src/lib.rs\"}"}
            })],
        );

        assert_eq!(
            finish_tool_call_fragments(fragments),
            vec![ModelToolCall {
                id: "call_1".to_string(),
                function: ModelToolFunction {
                    name: "read".to_string(),
                    arguments: r#"{"path":"src/lib.rs"}"#.to_string(),
                },
            }]
        );
    }

    #[test]
    fn reasoning_mode_disabled_appends_no_think_to_last_user_message() {
        let mut msgs = openai_messages(&[
            ModelMessage::system("You are helpful."),
            ModelMessage::user("explain entropy"),
        ]);
        apply_reasoning_mode(&mut msgs, ReasoningMode::Disabled);
        assert_eq!(msgs[1]["content"], "explain entropy /no_think");
        assert_eq!(msgs[0]["content"], "You are helpful."); // system unchanged
    }

    #[test]
    fn reasoning_mode_enabled_appends_think_to_last_user_message() {
        let mut msgs = openai_messages(&[ModelMessage::user("deep question")]);
        apply_reasoning_mode(&mut msgs, ReasoningMode::Enabled);
        assert_eq!(msgs[0]["content"], "deep question /think");
    }

    #[test]
    fn reasoning_mode_auto_leaves_messages_unchanged() {
        let mut msgs = openai_messages(&[ModelMessage::user("hello")]);
        apply_reasoning_mode(&mut msgs, ReasoningMode::Auto);
        assert_eq!(msgs[0]["content"], "hello");
    }

    #[test]
    fn reasoning_mode_tool_continuation_appends_to_existing_system_message() {
        // Agentic loop: last message is a tool result, not a user message.
        let mut msgs = openai_messages(&[
            ModelMessage::system("You are a coding assistant."),
            ModelMessage::user("list files"),
            ModelMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: vec![ModelToolCall {
                    id: "call_1".to_string(),
                    function: ModelToolFunction {
                        name: "list_dir".to_string(),
                        arguments: "{}".to_string(),
                    },
                }],
                tool_call_id: None,
            },
            ModelMessage {
                role: MessageRole::Tool,
                content: r#"["src/","Cargo.toml"]"#.to_string(),
                tool_calls: vec![],
                tool_call_id: Some("call_1".to_string()),
            },
        ]);
        apply_reasoning_mode(&mut msgs, ReasoningMode::Disabled);
        // System message gets the token; no historical message is modified.
        assert_eq!(msgs[0]["content"], "You are a coding assistant.\n/no_think");
        assert_eq!(msgs[1]["content"], "list files"); // user message untouched
    }

    #[test]
    fn reasoning_mode_tool_continuation_inserts_system_message_when_none_exists() {
        let mut msgs = openai_messages(&[
            ModelMessage::user("search the web"),
            ModelMessage {
                role: MessageRole::Tool,
                content: "results".to_string(),
                tool_calls: vec![],
                tool_call_id: Some("call_2".to_string()),
            },
        ]);
        apply_reasoning_mode(&mut msgs, ReasoningMode::Enabled);
        // A system message is prepended.
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "/think");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn tool_call_delta_preserves_partial_arguments_and_call_identity() {
        assert_eq!(
            tool_call_delta(
                &serde_json::json!({
                    "index": 2,
                    "id": "call_read",
                    "function": {"name": "read_file", "arguments": "{\"path\":"}
                }),
                0,
            ),
            ModelChunk::ToolCallDelta {
                index: 2,
                id: Some("call_read".to_string()),
                function_name: Some("read_file".to_string()),
                arguments_delta: Some(r#"{"path":"#.to_string()),
            }
        );
    }
}
