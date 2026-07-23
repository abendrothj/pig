//! Supervises `mlx_lm.server` as a subprocess and talks to its OpenAI-compatible
//! HTTP API. Models are HuggingFace-format directories (not GGUF), and MLX handles
//! Metal acceleration automatically on Apple Silicon — no GPU layer configuration.

use super::{
    apply_reasoning_mode, BackendCapabilities, BackendError, BackendGenerationRequest,
    BackendGenerationResponse, BackendHealth, LoadModelRequest, LoadedModel, ModelAvailability,
    ModelBackend, ModelEventSender,
};
use async_trait::async_trait;
use futures::StreamExt;
use pig_core::model::{AcceleratorKind, FinishReason, MessageRole, ModelChunk, ModelId};
use serde::{Deserialize, Serialize};
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
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MlxExecutionConfig {
    /// If true, pass --trust-remote-code to allow models with custom code.
    pub trust_remote_code: Option<bool>,
    /// Optional seed for reproducible outputs.
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MlxConfig {
    pub server_executable: PathBuf,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for MlxConfig {
    fn default() -> Self {
        Self {
            server_executable: PathBuf::from("mlx_lm.server"),
            startup_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(600),
        }
    }
}

struct SupervisedServer {
    child: Child,
    model_id: ModelId,
    base_url: String,
    // Kept alive so the background stderr-collection task runs until server exits.
    _stderr_tail: Arc<Mutex<String>>,
    loaded: LoadedModel,
}

pub struct MlxBackend {
    config: MlxConfig,
    client: reqwest::Client,
    server: Mutex<Option<SupervisedServer>>,
}

impl MlxBackend {
    pub fn new(config: MlxConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            server: Mutex::new(None),
        }
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
        text.lines().next().map(str::trim).map(str::to_string)
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

    async fn stop_current(&self, guard: &mut Option<SupervisedServer>) {
        if let Some(mut server) = guard.take() {
            let _ = server.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), server.child.wait()).await;
        }
    }
}

#[async_trait]
impl ModelBackend for MlxBackend {
    async fn health(&self) -> Result<BackendHealth, BackendError> {
        let version = self.executable_version().await;
        let output = Command::new(&self.config.server_executable)
            .arg("--help")
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() || !o.stdout.is_empty() => Ok(BackendHealth {
                available: true,
                detail: format!("{}", self.config.server_executable.display()),
                version,
            }),
            Ok(_) => Ok(BackendHealth {
                available: false,
                detail: format!(
                    "{} returned non-zero",
                    self.config.server_executable.display()
                ),
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
        let from_server = self
            .server
            .lock()
            .await
            .as_ref()
            .and_then(|s| s.loaded.accelerator);
        // mlx_lm only runs on Apple Silicon — always Metal when a model is loaded.
        let accelerator = from_server.or(Some(AcceleratorKind::Metal));
        Ok(BackendCapabilities {
            backend: "mlx".to_string(),
            version,
            accelerators: accelerator
                .into_iter()
                .chain(std::iter::once(AcceleratorKind::Cpu))
                .collect(),
            supports_streaming: true,
            supports_tools: false,
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

        if !request.path.exists() {
            return Err(BackendError::LoadFailed(format!(
                "model path not found: {}",
                request.path.display()
            )));
        }

        let execution: MlxExecutionConfig = if request.execution_config.is_null() {
            MlxExecutionConfig::default()
        } else {
            serde_json::from_value(request.execution_config.clone())
                .map_err(|e| BackendError::LoadFailed(format!("invalid execution_config: {}", e)))?
        };

        self.stop_current(&mut guard).await;

        let port = Self::find_free_port()?;
        let mut args = vec![
            "--model".to_string(),
            request.path.to_string_lossy().into_owned(),
            "--port".to_string(),
            port.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
        ];
        if execution.trust_remote_code == Some(true) {
            args.push("--trust-remote-code".to_string());
        }
        if let Some(seed) = execution.seed {
            args.push("--seed".to_string());
            args.push(seed.to_string());
        }

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

        let base_url = format!("http://127.0.0.1:{}", port);
        let models_url = format!("{}/v1/models", base_url);

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
            if let Ok(resp) = self.client.get(&models_url).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }

        let load_ms = start.elapsed().as_millis() as u64;
        let loaded = LoadedModel {
            model_id: request.model_id.clone(),
            context_tokens: request.context_size,
            already_loaded: false,
            load_ms,
            accelerator: Some(AcceleratorKind::Metal),
            cpu_threads: None,
            gpu_layers: None,
            batch_size: None,
        };

        *guard = Some(SupervisedServer {
            child,
            model_id: request.model_id,
            base_url,
            _stderr_tail: stderr_tail,
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
        let base_url = {
            let guard = self.server.lock().await;
            guard
                .as_ref()
                .filter(|s| s.model_id == request.model_id)
                .map(|s| s.base_url.clone())
                .ok_or_else(|| BackendError::ModelNotFound(request.model_id.0.clone()))?
        };

        let mut messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                serde_json::json!({"role": role, "content": m.content})
            })
            .collect();
        apply_reasoning_mode(&mut messages, request.parameters.reasoning_mode);

        let mut body = serde_json::json!({
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
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
        if !p.stop.is_empty() {
            body["stop"] = serde_json::json!(p.stop);
        }
        if let Some(v) = p.seed {
            body["seed"] = serde_json::json!(v);
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
        let mut prompt_tps: Option<f64> = None;
        let mut gen_tps: Option<f64> = None;
        let wall_start = Instant::now();
        let mut first_token_at: Option<Instant> = None;

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
                    if let Some(delta_content) = choice
                        .get("delta")
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        if !delta_content.is_empty() {
                            if first_token_at.is_none() {
                                first_token_at = Some(Instant::now());
                            }
                            content.push_str(delta_content);
                            let _ = events
                                .send(ModelChunk::TextDelta {
                                    text: delta_content.to_string(),
                                })
                                .await;
                        }
                    }
                    if let Some(fr) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        finish_reason = match fr {
                            "length" => FinishReason::Length,
                            _ => FinishReason::Stop,
                        };
                    }
                }

                if let Some(usage) = value.get("usage") {
                    prompt_tokens = usage
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    completion_tokens = usage
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    // mlx_lm.server does not report per-phase timings; derive from wall clock.
                    let wall_elapsed = wall_start.elapsed();
                    if let Some(ttft) = first_token_at {
                        let prefill_ms = ttft.duration_since(wall_start).as_millis() as u64;
                        let decode_ms =
                            wall_elapsed.as_millis().saturating_sub(prefill_ms as u128) as u64;
                        prompt_ms = prefill_ms;
                        generation_ms = decode_ms;
                        if prefill_ms > 0 && prompt_tokens > 0 {
                            prompt_tps = Some(prompt_tokens as f64 / (prefill_ms as f64 / 1000.0));
                        }
                        if decode_ms > 0 && completion_tokens > 0 {
                            gen_tps = Some(completion_tokens as f64 / (decode_ms as f64 / 1000.0));
                        }
                    }
                }
            }
        }

        let _ = events
            .send(ModelChunk::Finished {
                finish_reason,
                usage: Some(pig_core::model::ModelUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                }),
            })
            .await;

        Ok(BackendGenerationResponse {
            finish_reason,
            content,
            prompt_tokens,
            completion_tokens,
            prompt_ms,
            generation_ms,
            prompt_tokens_per_second: prompt_tps,
            generation_tokens_per_second: gen_tps,
            tool_calls: vec![],
        })
    }
}
