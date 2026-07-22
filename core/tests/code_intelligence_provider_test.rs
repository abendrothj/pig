//! Integration tests for `CodebaseMemoryCliProvider` against a real child process (the
//! `fake_codebase_memory_mcp` fixture binary), covering the safety properties the
//! adapter must hold regardless of what the underlying tool does: distinguishing
//! process failure from malformed output, enforcing a timeout with actual process
//! termination, and capping output size.

use lao_orchestrator_core::code_intelligence::{
    CodeIntelligenceProvider, CodebaseMemoryCliProvider, GraphOperation, ProviderError,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake_codebase_memory_mcp"))
}

fn provider(timeout: Duration, max_bytes: usize) -> CodebaseMemoryCliProvider {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core/ has a parent directory")
        .to_path_buf();
    CodebaseMemoryCliProvider::new(repo_root, Some(fake_binary()), timeout, max_bytes)
        .expect("fake provider binary should resolve")
}

#[test]
fn successful_query_returns_artifact_with_payload() {
    let p = provider(Duration::from_secs(5), 10_000_000);
    let artifact = p
        .query(
            GraphOperation::SearchGraph,
            serde_json::json!({"fake_mode": "success"}),
        )
        .expect("query should succeed");
    assert_eq!(artifact.operation, "search_graph");
    assert_eq!(artifact.provider, "codebase-memory-mcp");
    assert_eq!(artifact.payload["ok"], serde_json::json!(true));
}

#[test]
fn health_reports_available_for_a_resolvable_binary() {
    let p = provider(Duration::from_secs(5), 10_000_000);
    let health = p.health().expect("health check should not error");
    assert!(health.available);
}

#[test]
fn malformed_json_output_is_reported_distinctly_from_process_failure() {
    let p = provider(Duration::from_secs(5), 10_000_000);
    let err = p
        .query(
            GraphOperation::SearchGraph,
            serde_json::json!({"fake_mode": "malformed"}),
        )
        .unwrap_err();
    assert!(matches!(err, ProviderError::MalformedOutput(_)));
}

#[test]
fn nonzero_exit_is_reported_as_provider_error_with_captured_stderr() {
    let p = provider(Duration::from_secs(5), 10_000_000);
    let err = p
        .query(
            GraphOperation::SearchGraph,
            serde_json::json!({"fake_mode": "error"}),
        )
        .unwrap_err();
    match err {
        ProviderError::NonZeroExit { code, stderr } => {
            assert_eq!(code, Some(1));
            assert!(stderr.contains("simulated provider error"));
        }
        other => panic!("expected NonZeroExit, got {:?}", other),
    }
}

#[test]
fn slow_provider_is_killed_and_reported_as_timeout() {
    let p = provider(Duration::from_millis(200), 10_000_000);
    let start = Instant::now();
    let err = p
        .query(
            GraphOperation::SearchGraph,
            serde_json::json!({"fake_mode": "timeout"}),
        )
        .unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "child should be killed promptly on timeout, not left to run its full 60s sleep"
    );
}

#[test]
fn excessive_output_is_capped_and_rejected() {
    let p = provider(Duration::from_secs(5), 1000);
    let err = p
        .query(
            GraphOperation::SearchGraph,
            serde_json::json!({"fake_mode": "large"}),
        )
        .unwrap_err();
    assert_eq!(err, ProviderError::OutputTooLarge);
}

#[test]
fn unknown_binary_path_is_reported_as_not_found() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let err = CodebaseMemoryCliProvider::new(
        repo_root,
        Some(PathBuf::from("/definitely/not/a/real/path_xyz.txt")),
        Duration::from_secs(5),
        10_000_000,
    )
    .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)));
}
