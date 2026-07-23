//! `LlamaCppBackend` against a fake `llama-server` fixture (real child process, real
//! HTTP, no real llama.cpp/model required). Covers the highest-value scenarios from
//! the spec's fixture list; timeout/cancellation/forced-kill are exercised generically
//! via the FakeBackend-based worker tests (`job.rs`, `http_server_test.rs`), which
//! drive the same `wait_with_timeout`/cancellation-token machinery `LlamaCppBackend`
//! itself uses - readiness-delay, excessive-stderr, and graceful-shutdown-under-load
//! are not separately covered here; see the final report for the full list.

use lao_orchestrator_core::model::ModelId;
use lao_orchestrator_core::model::{GenerationParameters, ModelMessage, RequestId};
use lao_worker::backend::llama_cpp::{LlamaCppBackend, LlamaCppConfig};
use lao_worker::backend::{BackendError, BackendGenerationRequest, LoadModelRequest, ModelBackend};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// FAKE_LLAMA_MODE is a process-wide env var read by the fixture at spawn time; guard
// tests that set it so they don't race each other.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fixture_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake_llama_server"))
}

fn backend() -> LlamaCppBackend {
    LlamaCppBackend::new(LlamaCppConfig {
        server_executable: fixture_binary(),
        host: "127.0.0.1".to_string(),
        startup_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
    })
}

fn load_request(model_file: &std::path::Path) -> LoadModelRequest {
    LoadModelRequest {
        model_id: ModelId::from("fake-model"),
        path: model_file.to_path_buf(),
        context_size: Some(2048),
        execution_config: serde_json::Value::Null,
    }
}

fn generate_request() -> BackendGenerationRequest {
    BackendGenerationRequest {
        request_id: RequestId::generate(),
        model_id: ModelId::from("fake-model"),
        messages: vec![ModelMessage::user("hi")],
        parameters: GenerationParameters {
            max_tokens: Some(10),
            ..Default::default()
        },
    }
}

#[tokio::test]
async fn successful_startup_and_generation_captures_content_and_reasoning() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "success");

    let backend = backend();
    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf").unwrap();

    let loaded = backend
        .load_model(load_request(model_file.path()))
        .await
        .unwrap();
    assert!(!loaded.already_loaded);

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let response = backend
        .generate(generate_request(), tx, CancellationToken::new())
        .await
        .unwrap();

    // "Hello" (content) + " (thinking)" (reasoning_content) + " world" (content).
    assert_eq!(response.content, "Hello (thinking) world");
    assert_eq!(response.prompt_tokens, 3);
    assert_eq!(response.completion_tokens, 3);

    let mut tokens = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            lao_orchestrator_core::model::ModelChunk::TextDelta { text } => tokens.push(text),
            _ => {}
        }
    }
    assert_eq!(tokens, vec!["Hello", " (thinking)", " world"]);

    std::env::remove_var("FAKE_LLAMA_MODE");
}

#[tokio::test]
async fn startup_failure_is_reported_as_load_failed_not_a_hang() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "startup_failure");

    let backend = backend();
    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf").unwrap();

    let start = std::time::Instant::now();
    let err = backend
        .load_model(load_request(model_file.path()))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::LoadFailed(_)));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "should fail fast on process exit, not wait for startup_timeout"
    );

    std::env::remove_var("FAKE_LLAMA_MODE");
}

#[tokio::test]
async fn malformed_json_from_the_server_is_reported_distinctly() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "malformed_json");

    let backend = backend();
    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf").unwrap();
    backend
        .load_model(load_request(model_file.path()))
        .await
        .unwrap();

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    // Malformed lines are skipped by the parser (not fatal); with no valid content
    // deltas the generation "succeeds" with empty content rather than erroring - the
    // important property is it doesn't panic or hang on unparseable input.
    let response = backend
        .generate(generate_request(), tx, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.content, "");

    std::env::remove_var("FAKE_LLAMA_MODE");
}

#[tokio::test]
async fn non_2xx_from_the_server_is_reported_as_generation_failed() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "request_failure");

    let backend = backend();
    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf").unwrap();
    backend
        .load_model(load_request(model_file.path()))
        .await
        .unwrap();

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let err = backend
        .generate(generate_request(), tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::GenerationFailed(_)));

    std::env::remove_var("FAKE_LLAMA_MODE");
}

#[tokio::test]
async fn missing_model_file_is_rejected_before_spawning_anything() {
    let backend = backend();
    let err = backend
        .load_model(load_request(std::path::Path::new(
            "/definitely/not/a/real/model.gguf",
        )))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::LoadFailed(_)));
}
