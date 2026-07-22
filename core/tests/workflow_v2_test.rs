//! Regression coverage for workflow schema v2 and the v1/v2 normalization pass.
//!
//! Also closes a pre-existing gap: `condition`/`for_each` were previously exercised
//! only via Rust-constructed `WorkflowStep` structs, never through an actual YAML
//! fixture (see `workflows/test_condition.yaml`, `workflows/v2_condition.yaml`).

use lao_orchestrator_core::cross_platform::PathUtils;
use lao_orchestrator_core::plugins::PluginRegistry;
use lao_orchestrator_core::{load_workflow_yaml, run_workflow_yaml};
use serial_test::serial;
use std::path::PathBuf;

fn check_plugins_available(required_plugins: &[&str]) -> bool {
    let plugin_dir = PathUtils::plugin_dir();
    let reg = PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
    for plugin_name in required_plugins {
        if reg.get(plugin_name).is_none() {
            println!("⚠️  Plugin '{}' not found, skipping test", plugin_name);
            return false;
        }
    }
    true
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core/ has a parent directory")
        .to_path_buf()
}

fn fixture(name: &str) -> String {
    repo_root()
        .join("workflows")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Every committed workflow fixture — v1 and v2 alike — must still parse and pass
/// schema validation. This is the "regression tests: every existing workflow" gate.
#[test]
fn all_workflow_fixtures_parse_and_validate() {
    let dir = repo_root().join("workflows");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("workflows/ directory should exist") {
        let entry = entry.expect("readable dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let result = load_workflow_yaml(path.to_str().unwrap());
        assert!(
            result.is_ok(),
            "fixture {:?} failed to parse/validate: {:?}",
            path,
            result.err()
        );
        checked += 1;
    }
    assert!(checked > 0, "expected at least one workflow fixture");
}

#[test]
#[serial]
fn v2_chain_executes_end_to_end() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let logs = run_workflow_yaml(&fixture("v2_chain.yaml")).expect("v2 chain should execute");
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().all(|l| l.error.is_none()));
    let summarize = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("summarize step present");
    assert_eq!(summarize.output.as_deref(), Some("First step"));
}

#[test]
#[serial]
fn v2_fan_out_executes_end_to_end() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let logs = run_workflow_yaml(&fixture("v2_fan_out.yaml")).expect("v2 fan-out should execute");
    assert_eq!(logs.len(), 3);
    assert!(logs.iter().all(|l| l.error.is_none()));
}

#[test]
#[serial]
fn v2_loop_executes_end_to_end() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let logs = run_workflow_yaml(&fixture("v2_loop.yaml")).expect("v2 loop should execute");
    assert_eq!(logs.len(), 1);
    assert!(logs.iter().all(|l| l.error.is_none()));
    let output = logs[0].output.as_deref().unwrap_or_default();
    let items: Vec<String> = serde_json::from_str(output).expect("loop output is a JSON array");
    assert_eq!(items.len(), 3);
}

#[test]
#[serial]
fn v2_condition_follow_up_runs_when_condition_met() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let logs =
        run_workflow_yaml(&fixture("v2_condition.yaml")).expect("v2 condition should execute");
    let follow_up = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("follow_up step present");
    assert_ne!(follow_up.validation.as_deref(), Some("skipped"));
    assert_eq!(follow_up.output.as_deref(), Some("trigger"));
}

#[test]
#[serial]
fn v1_condition_fixture_executes_via_yaml() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }
    let logs = run_workflow_yaml(&fixture("test_condition.yaml"))
        .expect("v1 condition fixture should execute");
    let follow_up = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("second step present");
    assert_ne!(follow_up.validation.as_deref(), Some("skipped"));
    assert_eq!(follow_up.output.as_deref(), Some("trigger"));
}
