//! Integration tests for the worker's HTTP surface, against the fake backend (no
//! installed model runtime required). Each test spins up a real axum server on an
//! ephemeral port and talks to it over real HTTP via reqwest.

use pig_core::model::{ModelEntry, ModelId, ModelRegistry, ModelRole};
use pig_worker::backend::fake::FakeBackend;
use pig_worker::config::WorkerConfig;
use pig_worker::hardware::HardwareInfo;
use pig_worker::job::WorkerRuntime;
use pig_worker::state::AppState;
use std::collections::BTreeMap;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

async fn spawn_test_server(
    max_concurrent: usize,
    max_queued: usize,
    token_delay: Option<Duration>,
    auth_token: Option<String>,
) -> (String, Arc<AppState>) {
    let std_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();

    let model_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(model_file.path(), b"fake gguf bytes").unwrap();
    // Leak the tempfile so it outlives the test server (dropped at process exit,
    // acceptable for a short-lived test binary).
    let model_path = model_file.path().to_path_buf();
    std::mem::forget(model_file);

    let entry = ModelEntry {
        id: ModelId::from("m1"),
        format: "gguf".to_string(),
        path: model_path,
        backend: "fake".to_string(),
        context_tokens: Some(4096),
        estimated_memory_bytes: None,
        roles: vec![ModelRole::Reasoning],
    };
    let mut roles = BTreeMap::new();
    roles.insert(ModelRole::Reasoning, vec![ModelId::from("m1")]);
    let registry = ModelRegistry::new(vec![entry], roles).unwrap();

    let backend: Arc<dyn pig_worker::backend::ModelBackend> = match token_delay {
        Some(d) => Arc::new(FakeBackend::with_token_delay(d)),
        None => Arc::new(FakeBackend::new()),
    };
    let runtime = Arc::new(WorkerRuntime::new(
        "test-worker".to_string(),
        "test-host".to_string(),
        backend,
        "fake".to_string(),
        max_concurrent,
        max_queued,
        Duration::from_secs(5),
    ));

    let config_toml = format!("[worker]\nid = \"test-worker\"\nbind = \"{}\"\n", addr);
    let mut config = WorkerConfig::from_toml_str(&config_toml).unwrap();
    config.max_concurrent_jobs = max_concurrent;
    config.max_queued_jobs = max_queued;

    let state = Arc::new(AppState {
        config,
        runtime,
        registry,
        hardware: HardwareInfo::default(),
        started_at: Instant::now(),
        auth_token,
        backend_name: "fake".to_string(),
        hardware_cache: std::sync::Mutex::new(None),
    });

    let app = pig_worker::server::router(state.clone());
    let tokio_listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    tokio::spawn(async move {
        axum::serve(tokio_listener, app).await.ok();
    });

    (format!("http://{}", addr), state)
}

#[tokio::test]
async fn health_and_capabilities_report_backend_state() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();

    let health: serde_json::Value = client
        .get(format!("{}/v1/health", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["worker_id"], "test-worker");

    let caps: serde_json::Value = client
        .get(format!("{}/v1/capabilities", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps["backend"]["backend"], "fake");
    assert_eq!(caps["max_concurrent_jobs"], 2);
}

#[tokio::test]
async fn models_list_reports_registry_entries() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    let models: serde_json::Value = client
        .get(format!("{}/v1/models", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = models.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["entry"]["id"], "m1");
    assert_eq!(list[0]["available"], true);
}

#[tokio::test]
async fn load_then_generate_non_streaming_succeeds() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();

    let loaded: serde_json::Value = client
        .post(format!("{}/v1/models/load", base))
        .json(&serde_json::json!({"model_id": "m1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(loaded["model_id"], "m1");
    assert_eq!(loaded["already_loaded"], false);

    let response = client
        .post(format!("{}/v1/generate", base))
        .json(&serde_json::json!({
            "request_id": "r1",
            "role": "reasoning",
            "model": {"type": "id", "value": "m1"},
            "messages": [{"role": "user", "content": "one two three"}],
            "parameters": {"max_tokens": 3},
            "stream": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "success");
    assert_eq!(body["execution"]["generated_tokens"], 3);
    assert!(body["execution"]["model_already_loaded"].as_bool().unwrap());
}

#[tokio::test]
async fn generate_streams_sse_chunks_then_final_response() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();

    let mut resp = client
        .post(format!("{}/v1/generate", base))
        .json(&serde_json::json!({
            "request_id": "r2",
            "role": "reasoning",
            "messages": [{"role": "user", "content": "alpha beta gamma"}],
            "parameters": {"max_tokens": 3},
            "stream": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let mut body = String::new();
    while let Some(chunk) = resp.chunk().await.unwrap() {
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.contains("event: response") {
            break;
        }
    }
    assert!(body.contains("event: chunk"));
    assert!(body.contains("event: response"));
}

#[tokio::test]
async fn cancel_stops_a_running_job() {
    let (base, _state) = spawn_test_server(1, 4, Some(Duration::from_millis(50)), None).await;
    let client = reqwest::Client::new();

    // Non-blocking submit: fire the request in the background so we can cancel it.
    let base_clone = base.clone();
    let handle = tokio::spawn(async move {
        let client = reqwest::Client::new();
        client
            .post(format!("{}/v1/generate", base_clone))
            .json(&serde_json::json!({
                "request_id": "r3",
                "role": "reasoning",
                "messages": [{"role": "user", "content": "one two three four five"}],
                "parameters": {"max_tokens": 100},
                "stream": false
            }))
            .send()
            .await
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    let jobs: serde_json::Value = client
        .get(format!("{}/v1/jobs", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job_id = jobs[0]["job_id"].as_str().unwrap().to_string();

    let cancel_status = client
        .post(format!("{}/v1/jobs/{}/cancel", base, job_id))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(cancel_status, 202);

    let resp = handle.await.unwrap().unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "cancelled");
}

/// Poll `/v1/health` until `predicate(active_jobs, queued_jobs)` holds, or panic with
/// the last-observed counts once `timeout` elapses. Used instead of a fixed sleep so
/// tests prove the server has actually reached the state they assert on.
async fn wait_for_queue_state(
    client: &reqwest::Client,
    base: &str,
    timeout: Duration,
    predicate: impl Fn(usize, usize) -> bool,
) {
    let last_seen = std::sync::Mutex::new((0usize, 0usize));
    let poll = async {
        loop {
            if let Ok(resp) = client.get(format!("{}/v1/health", base)).send().await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let active = body["active_jobs"].as_u64().unwrap_or(0) as usize;
                    let queued = body["queued_jobs"].as_u64().unwrap_or(0) as usize;
                    *last_seen.lock().unwrap() = (active, queued);
                    if predicate(active, queued) {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    };
    if tokio::time::timeout(timeout, poll).await.is_err() {
        let (active, queued) = *last_seen.lock().unwrap();
        panic!(
            "queue never reached the expected state within {:?}; \
             last observed active_jobs={} queued_jobs={}",
            timeout, active, queued
        );
    }
}

#[tokio::test]
async fn queue_overflow_returns_429() {
    let (base, _state) = spawn_test_server(1, 1, Some(Duration::from_millis(100)), None).await;
    let client = reqwest::Client::new();

    let gen_request = |base: String| {
        let client = reqwest::Client::new();
        async move {
            client
                .post(format!("{}/v1/generate", base))
                .json(&serde_json::json!({
                    "request_id": uuid::Uuid::new_v4().to_string(),
                    "role": "reasoning",
                    "messages": [{"role": "user", "content": "one two three"}],
                    "parameters": {"max_tokens": 20},
                    "stream": false
                }))
                .send()
                .await
        }
    };

    // Submit the first job and wait for confirmation that it is actually running
    // (not merely queued) before submitting the second. Firing both "at once" via
    // two racing spawned tasks left a narrow but real TOCTOU window in the server's
    // admission check (`worker/job.rs::submit`): a job counts toward `queued_jobs`
    // from the moment it's admitted until it acquires its concurrency-semaphore
    // permit, and for an uncontended first job that window can close before a truly
    // concurrent second request reaches the same check — occasionally rejecting it
    // outright instead of queueing it. Waiting for confirmed-active here guarantees
    // the first job has already released that transient slot, so the second
    // request's admission is deterministic.
    let _first = tokio::spawn(gen_request(base.clone()));
    wait_for_queue_state(&client, &base, Duration::from_secs(5), |active, _queued| {
        active == 1
    })
    .await;

    let _second = tokio::spawn(gen_request(base.clone()));
    wait_for_queue_state(&client, &base, Duration::from_secs(5), |active, queued| {
        active == 1 && queued == 1
    })
    .await;

    let third = client
        .post(format!("{}/v1/generate", base))
        .json(&serde_json::json!({
            "request_id": "overflow",
            "role": "reasoning",
            "messages": [{"role": "user", "content": "one"}],
            "parameters": {"max_tokens": 5},
            "stream": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        third.status(),
        429,
        "expected the third request to be rejected once the active job and the one queue slot are both occupied"
    );
}

#[tokio::test]
async fn unknown_job_id_returns_404() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v1/jobs/does-not-exist", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn embed_and_rerank_are_explicitly_unsupported() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    for path in ["embed", "rerank"] {
        let resp = client
            .post(format!("{}/v1/{}", base, path))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 501);
    }
}

#[tokio::test]
async fn auth_enabled_rejects_missing_or_wrong_token() {
    let (base, _state) = spawn_test_server(2, 8, None, Some("secret-token".to_string())).await;
    let client = reqwest::Client::new();

    let unauthenticated = client
        .get(format!("{}/v1/health", base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), 401);

    let wrong_token = client
        .get(format!("{}/v1/health", base))
        .bearer_auth("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_token.status(), 401);

    let authenticated = client
        .get(format!("{}/v1/health", base))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(authenticated.status(), 200);
}

#[tokio::test]
async fn metrics_endpoint_requires_auth_like_every_other_endpoint() {
    let (base, _state) = spawn_test_server(2, 8, None, Some("secret-token".to_string())).await;
    let client = reqwest::Client::new();

    let unauthenticated = client
        .get(format!("{}/v1/metrics", base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), 401);

    let authenticated = client
        .get(format!("{}/v1/metrics", base))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(authenticated.status(), 200);
}

#[tokio::test]
async fn metrics_endpoint_before_any_model_load_reports_not_loaded_not_zero() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("{}/v1/metrics", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["worker"]["lifecycle_state"], "idle");
    assert_eq!(body["queue"]["depth"], 0);
    assert_eq!(body["jobs"]["active"], 0);
    assert_eq!(body["jobs"]["completed"], 0);
    assert!(body["model"]["loaded_model_id"].is_null());
    assert_eq!(body["model"]["load_state"], "not_loaded");
}

/// The `FakeBackend` test harness never has a real GPU, so `AppState.hardware` here
/// naturally has no accelerator - this is already the default case, not a special
/// stub, and it's exactly the scenario the endpoint must degrade gracefully under.
#[tokio::test]
async fn metrics_endpoint_reports_null_not_zero_when_hardware_telemetry_is_unavailable() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("{}/v1/metrics", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["accelerator"]["kind"].is_null());
    assert!(body["accelerator"]["name"].is_null());
    assert!(body["accelerator"]["utilization_percent"].is_null());
    assert!(body["throughput"]["last_prompt_tokens_per_second"].is_null());
    assert!(body["throughput"]["last_generation_tokens_per_second"].is_null());
}

#[tokio::test]
async fn load_of_unknown_model_id_is_404() {
    let (base, _state) = spawn_test_server(2, 8, None, None).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/models/load", base))
        .json(&serde_json::json!({"model_id": "does-not-exist"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
