//! End-to-end coverage for `run: local_llm` workflow steps against a fake
//! `ModelInvoker` — no installed model runtime required, matching the project's
//! existing fake-backend-first testing convention.

use lao_orchestrator_core::cross_platform::PathUtils;
use lao_orchestrator_core::execution::Artifact;
use lao_orchestrator_core::model::{
    FinishReason, ModelExecutionMetadata, ModelId, ModelInvoker, ModelRequest, ModelResponse,
    ModelResponseStatus, ModelUsage, ResolvedModel, WorkerId,
};
use lao_orchestrator_core::plugins::PluginRegistry;
use lao_orchestrator_core::workflow_parallel::run_workflow_with_options_and_invoker;
use std::path::PathBuf;
use std::sync::Arc;

struct FakeInvoker;

impl ModelInvoker for FakeInvoker {
    fn invoke(&self, request: ModelRequest) -> ModelResponse {
        let prompt = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        ModelResponse {
            request_id: request.request_id.clone(),
            status: ModelResponseStatus::Success,
            output: Artifact::Text(format!("ANSWER({}): {}", request.role, prompt)),
            finish_reason: FinishReason::Stop,
            model: ResolvedModel {
                model_id: ModelId::from("fake-model"),
                role: Some(request.role.clone()),
                backend: "fake".to_string(),
                identity: "fake".to_string(),
            },
            execution: ModelExecutionMetadata {
                worker_id: WorkerId::from("fake-worker"),
                host_name: "test-host".to_string(),
                backend: "fake".to_string(),
                backend_version: None,
                model_id: ModelId::from("fake-model"),
                model_identity: "fake".to_string(),
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
                prompt_tokens: 1,
                generated_tokens: 1,
                prompt_tokens_per_second: None,
                generation_tokens_per_second: None,
                model_already_loaded: true,
                cancellation: None,
                peak_memory_bytes: None,
                peak_vram_bytes: None,
            },
            usage: ModelUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
            tool_calls: vec![],
            error: None,
        }
    }
}

struct FailingInvoker;

impl ModelInvoker for FailingInvoker {
    fn invoke(&self, request: ModelRequest) -> ModelResponse {
        ModelResponse {
            request_id: request.request_id.clone(),
            status: ModelResponseStatus::Failed,
            output: Artifact::Null,
            finish_reason: FinishReason::Error,
            model: ResolvedModel {
                model_id: ModelId::from("fake-model"),
                role: Some(request.role.clone()),
                backend: "fake".to_string(),
                identity: "fake".to_string(),
            },
            execution: ModelExecutionMetadata {
                worker_id: WorkerId::from("fake-worker"),
                host_name: "test-host".to_string(),
                backend: "fake".to_string(),
                backend_version: None,
                model_id: ModelId::from("fake-model"),
                model_identity: "fake".to_string(),
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
            error: Some(
                lao_orchestrator_core::model::ModelExecutionError::BackendError {
                    message: "simulated backend failure".to_string(),
                },
            ),
        }
    }
}

fn check_plugins_available(required: &[&str]) -> bool {
    let plugin_dir = PathUtils::plugin_dir();
    let reg = PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
    for name in required {
        if reg.get(name).is_none() {
            println!("skipping: plugin '{}' not built", name);
            return false;
        }
    }
    true
}

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("workflows")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// `local_llm` is deny-by-default (trust.allow_model_inference); tests that need it
/// enabled point LAO_CONFIG at a temp file for the duration of the call. `#[serial]`
/// on every test in this file avoids racing on that process-wide env var.
struct TrustGuard {
    _dir: tempfile::TempDir,
}

fn allow_model_inference() -> TrustGuard {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lao.toml");
    std::fs::write(&path, "[trust]\nallow_model_inference = true\n").unwrap();
    std::env::set_var("LAO_CONFIG", &path);
    TrustGuard { _dir: dir }
}

impl Drop for TrustGuard {
    fn drop(&mut self) {
        std::env::remove_var("LAO_CONFIG");
    }
}

#[test]
#[serial_test::serial]
fn local_llm_step_runs_through_the_configured_invoker_and_sees_upstream_output() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let _guard = allow_model_inference();
    let logs = run_workflow_with_options_and_invoker(
        &fixture("local_llm_example.yaml"),
        false,
        false,
        "workflow_states",
        Some(Arc::new(FakeInvoker) as Arc<dyn ModelInvoker>),
        |_event| {},
    )
    .expect("workflow should execute");

    assert_eq!(logs.len(), 2);
    let reason_step = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("reason step present");
    assert!(reason_step.error.is_none());
    let output = reason_step.output.as_deref().unwrap_or_default();
    assert!(output.contains("ANSWER(reasoning)"));
    assert!(output.contains("trace: execute -> validate -> commit"));
    assert!(output.contains("Attached artifact"));
}

#[test]
#[serial_test::serial]
fn local_llm_step_without_an_invoker_fails_with_a_clear_error() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let _guard = allow_model_inference();
    let logs = run_workflow_with_options_and_invoker(
        &fixture("local_llm_example.yaml"),
        false,
        false,
        "workflow_states",
        None,
        |_event| {},
    )
    .expect("workflow should still complete (per-step failure, not a hard error)");

    let reason_step = logs.iter().find(|l| l.step_id == "step2").unwrap();
    assert!(reason_step
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("ModelInvoker"));
}

#[test]
#[serial_test::serial]
fn local_llm_step_surfaces_backend_failure_as_a_step_error() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let _guard = allow_model_inference();
    let logs = run_workflow_with_options_and_invoker(
        &fixture("local_llm_example.yaml"),
        false,
        false,
        "workflow_states",
        Some(Arc::new(FailingInvoker) as Arc<dyn ModelInvoker>),
        |_event| {},
    )
    .expect("workflow should complete with a step-level failure");

    let reason_step = logs.iter().find(|l| l.step_id == "step2").unwrap();
    assert!(reason_step.error.is_some());
    assert!(reason_step
        .error
        .as_deref()
        .unwrap()
        .contains("simulated backend failure"));
}

#[test]
#[serial_test::serial]
fn local_llm_is_denied_by_default_without_trust_configuration() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    std::env::remove_var("LAO_CONFIG"); // ensure a clean deny-by-default policy
    let logs = run_workflow_with_options_and_invoker(
        &fixture("local_llm_example.yaml"),
        false,
        false,
        "workflow_states",
        Some(Arc::new(FakeInvoker) as Arc<dyn ModelInvoker>),
        |_event| {},
    )
    .expect("workflow should complete with a step-level denial, not a hard error");

    let reason_step = logs.iter().find(|l| l.step_id == "step2").unwrap();
    assert!(reason_step
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("allow_model_inference"));
}
