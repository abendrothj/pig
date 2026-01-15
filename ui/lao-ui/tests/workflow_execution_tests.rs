//! End-to-end workflow execution tests that actually run workflows

use lao_ui::backend::{get_workflow_graph, run_workflow_stream, BackendState};
use lao_orchestrator_core::cross_platform::PathUtils;
use lao_orchestrator_core::plugins::PluginRegistry;
use serial_test::serial;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

// Helper to check if plugins are available
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

#[test]
#[serial]
fn test_workflow_execution_single_step() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create a simple workflow file
    let workflow_yaml = r#"workflow: "Single Step Test"
steps:
  - run: EchoPlugin
    input: "Hello from test!"
"#;
    let test_path = "./temp_single_step_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    // Create backend state
    let state = Arc::new(Mutex::new(BackendState::default()));

    // Execute workflow
    let result = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));
    
    // Wait a bit for execution to complete
    thread::sleep(Duration::from_millis(500));

    // Check results
    let state_guard = state.lock().unwrap();
    
    // Should have logs
    assert!(!state_guard.live_logs.is_empty(), "Should have execution logs");
    
    // Should have a workflow result
    assert!(state_guard.workflow_result.is_some(), "Should have workflow result");
    
    if let Some(ref result) = state_guard.workflow_result {
        // Check that we got some output
        assert!(result.completed_steps > 0 || result.failed_steps > 0, "Should have executed at least one step");
        
        // Check logs contain expected output
        let has_hello = state_guard.live_logs.iter()
            .any(|log| log.contains("Hello from test!") || log.contains("success"));
        assert!(has_hello, "Logs should contain execution output");
    }

    // Cleanup
    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_multi_step_sequential() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create a sequential workflow
    let workflow_yaml = r#"workflow: "Sequential Test"
steps:
  - run: EchoPlugin
    id: step1
    input: "First step"
  - run: EchoPlugin
    id: step2
    input_from: step1
    depends_on: [step1]
"#;
    let test_path = "./temp_sequential_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    let state = Arc::new(Mutex::new(BackendState::default()));
    let _ = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));
    
    thread::sleep(Duration::from_millis(1000));

    let state_guard = state.lock().unwrap();
    assert!(!state_guard.live_logs.is_empty());
    assert!(state_guard.workflow_result.is_some());
    
    if let Some(ref result) = state_guard.workflow_result {
        assert!(result.completed_steps >= 1, "Should complete at least one step");
    }

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_parallel() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create a parallel workflow (multiple independent steps)
    let workflow_yaml = r#"workflow: "Parallel Test"
steps:
  - run: EchoPlugin
    id: step1
    input: "Parallel step 1"
  - run: EchoPlugin
    id: step2
    input: "Parallel step 2"
  - run: EchoPlugin
    id: step3
    input: "Parallel step 3"
"#;
    let test_path = "./temp_parallel_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    // Create backend state and load the graph (needed for parallel metrics)
    let mut state = BackendState::default();
    if let Ok(graph) = get_workflow_graph(test_path) {
        state.graph = Some(graph);
    }
    let state = Arc::new(Mutex::new(state));

    let _ = run_workflow_stream(test_path.to_string(), true, Arc::clone(&state));

    thread::sleep(Duration::from_millis(1000));

    let state_guard = state.lock().unwrap();
    assert!(!state_guard.live_logs.is_empty());
    assert!(state_guard.workflow_result.is_some());

    if let Some(ref result) = state_guard.workflow_result {
        // Parallel execution should complete multiple steps
        assert!(result.completed_steps >= 2, "Should complete multiple steps in parallel");

        // Should have parallel execution metrics (requires graph to be loaded)
        assert!(result.parallel_execution.is_some(), "Should have parallel execution metrics");

        if let Some(ref metrics) = result.parallel_execution {
            assert!(metrics.execution_levels > 0, "Should have execution levels");
            // All 3 steps are independent, so they should all run in parallel (1 level)
            assert!(metrics.max_parallelism >= 2, "Should have parallelism >= 2");
        }
    }

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_with_errors() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create a workflow with an invalid step (non-existent plugin)
    let workflow_yaml = r#"workflow: "Error Test"
steps:
  - run: EchoPlugin
    id: step1
    input: "Valid input"
  - run: NonExistentPlugin
    id: step2
    input: "This should fail"
"#;
    let test_path = "./temp_error_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    let state = Arc::new(Mutex::new(BackendState::default()));
    let _ = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));

    thread::sleep(Duration::from_millis(1000));

    let state_guard = state.lock().unwrap();
    // When a plugin is not found, the workflow should fail during validation
    // Either we have logs indicating an error, or the workflow_result shows failure
    let has_error_in_logs = state_guard.live_logs.iter()
        .any(|log| log.to_lowercase().contains("error") || log.to_lowercase().contains("not found"));
    let has_failed_result = state_guard.workflow_result.as_ref()
        .map(|r| r.failed_steps > 0 || !r.success)
        .unwrap_or(false);
    let has_error_message = !state_guard.error.is_empty();

    assert!(
        has_error_in_logs || has_failed_result || has_error_message,
        "Should have errors for non-existent plugin. Logs: {:?}, Error: {:?}, Result: {:?}",
        state_guard.live_logs,
        state_guard.error,
        state_guard.workflow_result
    );

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_progress_tracking() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow_yaml = r#"workflow: "Progress Test"
steps:
  - run: EchoPlugin
    input: "Step 1"
  - run: EchoPlugin
    input: "Step 2"
  - run: EchoPlugin
    input: "Step 3"
"#;
    let test_path = "./temp_progress_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    let state = Arc::new(Mutex::new(BackendState::default()));
    let _ = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));

    // Wait for completion
    thread::sleep(Duration::from_millis(500));

    {
        let state_guard = state.lock().unwrap();
        // After completion, progress should be 1.0
        assert_eq!(state_guard.execution_progress, 1.0, "Progress should be 1.0 after completion");
        assert!(!state_guard.is_running, "Should not be running after completion");
        assert!(state_guard.workflow_result.is_some(), "Should have workflow result");
    }

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_state_updates() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow_yaml = r#"workflow: "State Update Test"
steps:
  - run: EchoPlugin
    id: step1
    input: "Test state updates"
"#;
    let test_path = "./temp_state_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    let state = Arc::new(Mutex::new(BackendState::default()));

    // Initially should not be running
    {
        let state_guard = state.lock().unwrap();
        assert!(!state_guard.is_running);
        assert_eq!(state_guard.execution_progress, 0.0);
    }

    let _ = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));

    // Wait for completion (EchoPlugin is fast, so it may complete before we can check is_running)
    thread::sleep(Duration::from_millis(500));

    {
        let state_guard = state.lock().unwrap();
        // After execution completes:
        // - should not be running anymore
        // - progress should be 1.0
        // - should have a workflow result
        assert!(!state_guard.is_running, "Should not be running after completion");
        assert_eq!(state_guard.execution_progress, 1.0, "Progress should be 1.0 after completion");
        assert!(state_guard.workflow_result.is_some(), "Should have workflow result");
    }

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}

#[test]
#[serial]
fn test_workflow_execution_logs_accumulation() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow_yaml = r#"workflow: "Logs Test"
steps:
  - run: EchoPlugin
    id: step1
    input: "Log test 1"
  - run: EchoPlugin
    id: step2
    input: "Log test 2"
"#;
    let test_path = "./temp_logs_test.yaml";
    fs::write(test_path, workflow_yaml).unwrap();

    let state = Arc::new(Mutex::new(BackendState::default()));
    let _ = run_workflow_stream(test_path.to_string(), false, Arc::clone(&state));
    
    thread::sleep(Duration::from_millis(1000));

    let state_guard = state.lock().unwrap();
    
    // Should have accumulated logs
    assert!(!state_guard.live_logs.is_empty(), "Should have logs");
    
    // Logs should contain step information
    let has_step1 = state_guard.live_logs.iter().any(|log| log.contains("step1"));
    let has_step2 = state_guard.live_logs.iter().any(|log| log.contains("step2"));
    
    assert!(has_step1 || has_step2, "Logs should contain step references");
    
    // Logs should contain plugin name
    let has_echo = state_guard.live_logs.iter().any(|log| log.contains("EchoPlugin"));
    assert!(has_echo, "Logs should contain plugin name");

    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }
}
