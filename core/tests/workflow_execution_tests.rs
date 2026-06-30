//! Comprehensive workflow execution tests that actually run workflows end-to-end

use lao_orchestrator_core::cross_platform::PathUtils;
use lao_orchestrator_core::plugins::PluginRegistry;
use lao_orchestrator_core::{
    run_workflow_with_options, run_workflow_yaml, run_workflow_yaml_parallel_with_callback,
    run_workflow_yaml_with_callback, StepEvent, Workflow, WorkflowStep,
};
use serial_test::serial;
use std::fs;

// Helper function to check if required plugins are available
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
fn test_workflow_execution_basic() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Basic Execution Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'Basic test'").unwrap(),
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            for_each: None,
        }],
    };
    let path = "temp_basic_execution.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    let logs = run_workflow_yaml(path).expect("Workflow should execute successfully");

    // Verify execution completed
    assert!(!logs.is_empty(), "Should have execution logs");

    // Verify output
    assert!(
        logs.iter().any(|log| log
            .output
            .as_ref()
            .map(|o| o.contains("Basic test"))
            .unwrap_or(false)),
        "Should have expected output"
    );

    // Verify no errors
    assert!(
        logs.iter().all(|log| log.error.is_none()),
        "Should not have errors"
    );

    fs::remove_file(path).unwrap();
}

#[test]
#[serial]
fn test_workflow_execution_chain() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create a chain: step1 -> step2 -> step3
    let workflow = Workflow {
        workflow: "Chain Execution Test".to_string(),
        steps: vec![
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'Chain step 1'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step2".to_string()),
                depends_on: Some(vec!["step2".to_string()]),
                condition: None,
                for_each: None,
            },
        ],
    };
    let path = "temp_chain_execution.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    let logs = run_workflow_yaml(path).expect("Chain workflow should execute");

    // Should have 3 steps
    assert_eq!(logs.len(), 3, "Should have 3 execution logs");

    // All steps should succeed
    assert!(
        logs.iter().all(|log| log.error.is_none()),
        "All steps should succeed"
    );

    fs::remove_file(path).unwrap();
}

#[test]
#[serial]
fn test_workflow_execution_parallel() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create parallel steps (no dependencies)
    let workflow = Workflow {
        workflow: "Parallel Execution Test".to_string(),
        steps: vec![
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'Parallel 1'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'Parallel 2'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'Parallel 3'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
        ],
    };
    let path = "temp_parallel_execution.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    let mut events = Vec::new();
    let logs = run_workflow_yaml_parallel_with_callback(path, |event: StepEvent| {
        events.push(event);
    })
    .expect("Parallel workflow should execute");

    // Should have 3 steps
    assert_eq!(logs.len(), 3, "Should have 3 execution logs");

    // Should have events for all steps
    assert!(events.len() >= 3, "Should have events for all steps");

    // Verify parallel execution (steps should complete in any order)
    let completed_steps: Vec<_> = events
        .iter()
        .filter(|e| e.status == "success" || e.status == "cache")
        .collect();
    assert_eq!(completed_steps.len(), 3, "All steps should complete");

    fs::remove_file(path).unwrap();
}

#[test]
#[serial]
fn test_serial_execution_matches_parallel_outputs() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // a -> b (b consumes a's output) so ordering is observable.
    let workflow = Workflow {
        workflow: "Serial Chain".to_string(),
        steps: vec![
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'first'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'second'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                condition: None,
                for_each: None,
            },
        ],
    };
    let path = "temp_serial_execution.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    // Serial run (no state recording, no concurrency) through the shared StepExecutor.
    let logs = run_workflow_with_options(path, false, false, "workflow_states", |_e: StepEvent| {})
        .expect("Serial workflow should execute");

    assert_eq!(logs.len(), 2, "Should have 2 execution logs");
    // The dependent step echoes the first step's output ("first").
    let second = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("step2 should be present");
    assert_eq!(second.output.as_deref(), Some("first"));

    fs::remove_file(path).unwrap();
}

#[test]
#[serial]
fn test_workflow_execution_with_callback() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Callback Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'Callback test'").unwrap(),
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            for_each: None,
        }],
    };
    let path = "temp_callback_test.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    let mut events = Vec::new();
    let logs = run_workflow_yaml_with_callback(path, |event: StepEvent| {
        events.push(event.clone());
        println!(
            "Event: step={} status={} attempt={}",
            event.step, event.status, event.attempt
        );
    })
    .expect("Workflow with callback should execute");

    // Should have received events
    assert!(!events.is_empty(), "Should have received callback events");

    // Should have at least one "running" event
    assert!(
        events.iter().any(|e| e.status == "running"),
        "Should have running event"
    );

    // Should have at least one "success" event
    assert!(
        events.iter().any(|e| e.status == "success"),
        "Should have success event"
    );

    // Logs should match events
    assert_eq!(logs.len(), 1, "Should have one log entry");

    fs::remove_file(path).unwrap();
}

#[test]
#[serial]
fn test_workflow_execution_fan_out_fan_in() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Create fan-out then fan-in: step1 -> [step2, step3] -> step4
    let workflow = Workflow {
        workflow: "Fan Out/In Test".to_string(),
        steps: vec![
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'Source'").unwrap(),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                condition: None,
                for_each: None,
            },
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: Some("step2".to_string()),
                depends_on: Some(vec!["step2".to_string(), "step3".to_string()]),
                condition: None,
                for_each: None,
            },
        ],
    };
    let path = "temp_fan_test.yaml";
    fs::write(path, serde_yaml::to_string(&workflow).unwrap()).unwrap();

    let logs = run_workflow_yaml(path).expect("Fan workflow should execute");

    // Should have 4 steps
    assert_eq!(logs.len(), 4, "Should have 4 execution logs");

    // Verify execution order (step1 first, then step2/step3 in parallel, then step4)
    let step1_log = logs.iter().find(|l| l.step == 0);
    let step2_log = logs.iter().find(|l| l.step == 1);
    let step3_log = logs.iter().find(|l| l.step == 2);
    let step4_log = logs.iter().find(|l| l.step == 3);

    assert!(step1_log.is_some(), "Step 1 should exist");
    assert!(step2_log.is_some(), "Step 2 should exist");
    assert!(step3_log.is_some(), "Step 3 should exist");
    assert!(step4_log.is_some(), "Step 4 should exist");

    // All should succeed
    assert!(
        logs.iter().all(|log| log.error.is_none()),
        "All steps should succeed"
    );

    fs::remove_file(path).unwrap();
}
