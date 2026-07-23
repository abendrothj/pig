//! End-to-end: real `Coordinator` (blocking HTTP client + scheduler) against real
//! worker HTTP servers (fake backend), covering routing, failover to a second worker
//! when the first is unreachable, and total unavailability.

use pig_core::model::{
    GenerationParameters, ModelEntry, ModelId, ModelInvoker, ModelMessage, ModelRegistry,
    ModelRequest, ModelRequirements, ModelResponseStatus, ModelRole, RequestId,
};
use pig_worker::backend::fake::FakeBackend;
use pig_worker::config::WorkerConfig;
use pig_worker::coordinator::{Coordinator, WorkerEndpointConfig};
use pig_worker::hardware::HardwareInfo;
use pig_worker::job::WorkerRuntime;
use pig_worker::state::AppState;
use std::collections::BTreeMap;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Instant;

fn model_registry_with_local_entry() -> (ModelRegistry, String) {
    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf bytes").unwrap();
    let path = model_file.path().to_path_buf();
    std::mem::forget(model_file);

    let entry = ModelEntry {
        id: ModelId::from("m1"),
        format: "gguf".to_string(),
        path,
        backend: "fake".to_string(),
        context_tokens: Some(4096),
        estimated_memory_bytes: None,
        roles: vec![ModelRole::Reasoning],
        execution_config: serde_json::Value::Null,
    };
    let mut roles = BTreeMap::new();
    roles.insert(ModelRole::Reasoning, vec![ModelId::from("m1")]);
    let registry = ModelRegistry::new(vec![entry], roles).unwrap();
    (registry, "m1".to_string())
}

async fn spawn_worker(id: &str) -> String {
    let (registry, _model_id) = model_registry_with_local_entry();
    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();

    let backend: Arc<dyn pig_worker::backend::ModelBackend> = Arc::new(FakeBackend::new());
    let runtime = Arc::new(WorkerRuntime::new(
        id.to_string(),
        "test-host".to_string(),
        backend,
        "fake".to_string(),
        2,
        8,
        std::time::Duration::from_secs(5),
    ));
    let config_toml = format!("[worker]\nid = \"{}\"\nbind = \"{}\"\n", id, addr);
    let config = WorkerConfig::from_toml_str(&config_toml).unwrap();
    let state = Arc::new(AppState {
        config,
        runtime,
        registry,
        hardware: HardwareInfo::default(),
        started_at: Instant::now(),
        auth_token: None,
        backend_name: "fake".to_string(),
        hardware_cache: std::sync::Mutex::new(None),
    });
    let app = pig_worker::server::router(state);
    let tokio_listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    tokio::spawn(async move {
        axum::serve(tokio_listener, app).await.ok();
    });
    format!("http://{}", addr)
}

fn request() -> ModelRequest {
    ModelRequest {
        request_id: RequestId::generate(),
        role: ModelRole::Reasoning,
        model: None,
        messages: vec![ModelMessage::user("one two three")],
        parameters: GenerationParameters {
            max_tokens: Some(3),
            ..Default::default()
        },
        requirements: ModelRequirements::default(),
        inputs: vec![],
        metadata: BTreeMap::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_routes_to_and_generates_via_a_real_worker() {
    let url = spawn_worker("w1").await;
    let (registry, _) = model_registry_with_local_entry();

    let response = tokio::task::spawn_blocking(move || {
        // reqwest::blocking must be constructed *and* dropped outside any tokio
        // async context, so both happen inside this same blocking closure.
        let coordinator = Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "w1".to_string(),
                url,
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        );
        coordinator.invoke(request())
    })
    .await
    .unwrap();
    assert_eq!(response.status, ModelResponseStatus::Success);
    assert_eq!(response.execution.worker_id.0, "w1");
    assert_eq!(response.execution.generated_tokens, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_fails_over_to_the_second_worker_when_the_first_is_unreachable() {
    let healthy_url = spawn_worker("w2").await;
    let (registry, _) = model_registry_with_local_entry();

    let response = tokio::task::spawn_blocking(move || {
        let coordinator = Coordinator::new(
            vec![
                WorkerEndpointConfig {
                    id: "w1-dead".to_string(),
                    url: "http://127.0.0.1:1".to_string(), // nothing listens here
                    auth_token_env: None,
                    priority: 0,
                },
                WorkerEndpointConfig {
                    id: "w2".to_string(),
                    url: healthy_url,
                    auth_token_env: None,
                    priority: 0,
                },
            ],
            registry,
        );
        coordinator.invoke(request())
    })
    .await
    .unwrap();
    assert_eq!(response.status, ModelResponseStatus::Success);
    assert_eq!(response.execution.worker_id.0, "w2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_reports_structured_failure_when_all_workers_are_unavailable() {
    let (registry, _) = model_registry_with_local_entry();

    let response = tokio::task::spawn_blocking(move || {
        let coordinator = Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "dead".to_string(),
                url: "http://127.0.0.1:1".to_string(),
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        );
        coordinator.invoke(request())
    })
    .await
    .unwrap();
    assert_eq!(response.status, ModelResponseStatus::Failed);
    assert!(response.error.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn route_explanation_names_the_selected_worker_before_generation() {
    let url = spawn_worker("w1").await;
    let (registry, _) = model_registry_with_local_entry();

    let explanation = tokio::task::spawn_blocking(move || {
        let coordinator = Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "w1".to_string(),
                url,
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        );
        coordinator.route(&request(), &Default::default())
    })
    .await
    .unwrap();
    let selected = explanation.selected.expect("a worker should be selected");
    assert_eq!(selected.worker_id.0, "w1");
    assert_eq!(selected.model_id.0, "m1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_server_health_reports_worker_counts() {
    let worker_url = spawn_worker("ws1").await;
    let (registry, _) = model_registry_with_local_entry();

    // reqwest::blocking::Client::builder().build() panics when called from within a
    // tokio async context — spawn_blocking lets reqwest initialize on a blocking thread.
    let coordinator = tokio::task::spawn_blocking(move || {
        Arc::new(Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "ws1".to_string(),
                url: worker_url,
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        ))
    })
    .await
    .unwrap();

    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let coord_addr = std_listener.local_addr().unwrap();
    let state = Arc::new(pig_worker::coordinator_server::CoordinatorServerState::new(
        coordinator,
        None,
    ));
    let app = pig_worker::coordinator_server::router(state);
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let coord_url = format!("http://{}", coord_addr);
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v1/health", coord_url))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["workers_total"], 1);
    assert_eq!(body["workers_healthy"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_server_generate_routes_through_to_worker() {
    let worker_url = spawn_worker("ws2").await;
    let (registry, _) = model_registry_with_local_entry();

    let coordinator = tokio::task::spawn_blocking(move || {
        Arc::new(Coordinator::new(
            vec![WorkerEndpointConfig {
                id: "ws2".to_string(),
                url: worker_url,
                auth_token_env: None,
                priority: 0,
            }],
            registry,
        ))
    })
    .await
    .unwrap();

    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let coord_addr = std_listener.local_addr().unwrap();
    let state = Arc::new(pig_worker::coordinator_server::CoordinatorServerState::new(
        coordinator,
        None,
    ));
    let app = pig_worker::coordinator_server::router(state);
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let coord_url = format!("http://{}", coord_addr);
    let client = reqwest::Client::new();
    let req = request();
    let resp = client
        .post(format!("{}/v1/generate", coord_url))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let response: pig_core::model::ModelResponse = resp.json().await.unwrap();
    assert_eq!(response.status, ModelResponseStatus::Success);
    assert_eq!(response.execution.worker_id.0, "ws2");
}
