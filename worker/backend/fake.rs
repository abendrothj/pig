//! Deterministic fake backend for CI: no installed model runtime required. Behavior is
//! triggered by well-known model IDs (`fail-to-load`, `fail-to-generate`) so tests can
//! exercise every failure path without a real runtime.

use super::{
    BackendCapabilities, BackendError, BackendGenerationRequest, BackendGenerationResponse,
    BackendHealth, LoadModelRequest, LoadedModel, ModelAvailability, ModelBackend,
    ModelEventSender,
};
use async_trait::async_trait;
use lao_orchestrator_core::model::{
    AcceleratorKind, FinishReason, MessageRole, ModelChunk, ModelId,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub struct FakeBackend {
    loaded: RwLock<HashMap<ModelId, LoadedModel>>,
    token_delay: Duration,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            loaded: RwLock::new(HashMap::new()),
            token_delay: Duration::ZERO,
        }
    }

    /// A per-token delay, useful for deterministically exercising cancellation/timeout
    /// behavior against a slow generation without a real model.
    pub fn with_token_delay(delay: Duration) -> Self {
        Self {
            loaded: RwLock::new(HashMap::new()),
            token_delay: delay,
        }
    }
}

#[async_trait]
impl ModelBackend for FakeBackend {
    async fn health(&self) -> Result<BackendHealth, BackendError> {
        Ok(BackendHealth {
            available: true,
            detail: "fake backend".to_string(),
            version: Some("0.0.0-fake".to_string()),
        })
    }

    async fn capabilities(&self) -> Result<BackendCapabilities, BackendError> {
        Ok(BackendCapabilities {
            backend: "fake".to_string(),
            version: Some("0.0.0-fake".to_string()),
            accelerators: vec![AcceleratorKind::Cpu],
            supports_streaming: true,
            supports_tools: false,
            supports_embedding: false,
            supports_reranking: false,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelAvailability>, BackendError> {
        let loaded = self.loaded.read().await;
        Ok(loaded
            .keys()
            .map(|id| ModelAvailability {
                model_id: id.clone(),
                path: None,
                loaded: true,
            })
            .collect())
    }

    async fn load_model(&self, request: LoadModelRequest) -> Result<LoadedModel, BackendError> {
        if request.model_id.0 == "fail-to-load" {
            return Err(BackendError::LoadFailed(
                "simulated load failure".to_string(),
            ));
        }
        let mut loaded = self.loaded.write().await;
        if let Some(existing) = loaded.get(&request.model_id) {
            let mut reused = existing.clone();
            reused.already_loaded = true;
            return Ok(reused);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        let model = LoadedModel {
            model_id: request.model_id.clone(),
            context_tokens: request.context_size.or(Some(4096)),
            already_loaded: false,
            load_ms: 5,
            accelerator: Some(AcceleratorKind::Cpu),
            cpu_threads: Some(4),
            gpu_layers: None,
            batch_size: Some(512),
        };
        loaded.insert(request.model_id.clone(), model.clone());
        Ok(model)
    }

    async fn unload_model(&self, model: &ModelId) -> Result<(), BackendError> {
        self.loaded.write().await.remove(model);
        Ok(())
    }

    async fn generate(
        &self,
        request: BackendGenerationRequest,
        events: ModelEventSender,
        cancellation: CancellationToken,
    ) -> Result<BackendGenerationResponse, BackendError> {
        if request.model_id.0 == "fail-to-generate" {
            return Err(BackendError::GenerationFailed(
                "simulated generation failure".to_string(),
            ));
        }

        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let words: Vec<&str> = last_user.split_whitespace().collect();
        let prompt_tokens = words.len().max(1) as u32;
        let max_tokens = request.parameters.max_tokens.unwrap_or(16) as usize;

        let mut produced = String::new();
        let mut count = 0usize;
        let source: Vec<&str> = if words.is_empty() { vec!["ok"] } else { words };

        for word in source.iter().cycle() {
            if count >= max_tokens {
                break;
            }
            if cancellation.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let piece = format!("{} ", word);
            produced.push_str(&piece);
            if events
                .send(ModelChunk::TextDelta { text: piece })
                .await
                .is_err()
            {
                // Receiver dropped (e.g. client disconnected); stop producing more.
                break;
            }
            count += 1;

            if !self.token_delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(self.token_delay) => {}
                    _ = cancellation.cancelled() => return Err(BackendError::Cancelled),
                }
            }
        }
        let generation_ms = (count as u64) * self.token_delay.as_millis() as u64;
        Ok(BackendGenerationResponse {
            finish_reason: if count >= max_tokens {
                FinishReason::Length
            } else {
                FinishReason::Stop
            },
            content: produced.trim_end().to_string(),
            prompt_tokens,
            completion_tokens: count as u32,
            prompt_ms: 1,
            generation_ms,
            prompt_tokens_per_second: Some(1000.0),
            generation_tokens_per_second: if generation_ms == 0 {
                None
            } else {
                Some(count as f64 / (generation_ms as f64 / 1000.0))
            },
            tool_calls: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lao_orchestrator_core::model::{GenerationParameters, ModelMessage, RequestId};

    fn request(model: &str, prompt: &str, max_tokens: u32) -> BackendGenerationRequest {
        BackendGenerationRequest {
            request_id: RequestId::generate(),
            model_id: ModelId::from(model),
            messages: vec![ModelMessage::user(prompt)],
            parameters: GenerationParameters {
                max_tokens: Some(max_tokens),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn load_then_reuse_reports_already_loaded() {
        let backend = FakeBackend::new();
        let req = LoadModelRequest {
            model_id: ModelId::from("m1"),
            path: "/models/m1.gguf".into(),
            context_size: Some(2048),
            execution_config: serde_json::Value::Null,
        };
        let first = backend.load_model(req.clone()).await.unwrap();
        assert!(!first.already_loaded);
        let second = backend.load_model(req).await.unwrap();
        assert!(second.already_loaded);
    }

    #[tokio::test]
    async fn load_failure_model_id_is_reported() {
        let backend = FakeBackend::new();
        let req = LoadModelRequest {
            model_id: ModelId::from("fail-to-load"),
            path: "/models/x.gguf".into(),
            context_size: None,
            execution_config: serde_json::Value::Null,
        };
        assert!(matches!(
            backend.load_model(req).await,
            Err(BackendError::LoadFailed(_))
        ));
    }

    #[tokio::test]
    async fn generation_streams_tokens_and_completes() {
        let backend = FakeBackend::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let cancel = CancellationToken::new();
        let resp = backend
            .generate(request("m1", "one two three", 3), tx, cancel)
            .await
            .unwrap();
        assert_eq!(resp.completion_tokens, 3);
        assert_eq!(resp.finish_reason, FinishReason::Length);
        let mut tokens = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ModelChunk::TextDelta { text } => tokens.push(text),
                _ => {}
            }
        }
        assert_eq!(tokens.len(), 3);
    }

    #[tokio::test]
    async fn generation_failure_model_id_is_reported() {
        let backend = FakeBackend::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let cancel = CancellationToken::new();
        let err = backend
            .generate(request("fail-to-generate", "hi", 5), tx, cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::GenerationFailed(_)));
    }

    #[tokio::test]
    async fn cancellation_token_stops_generation() {
        let backend = FakeBackend::with_token_delay(Duration::from_millis(50));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel_clone.cancel();
        });
        let err = backend
            .generate(request("m1", "one two three four five", 100), tx, cancel)
            .await
            .unwrap_err();
        assert_eq!(err, BackendError::Cancelled);
    }
}
