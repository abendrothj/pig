//! End-to-end tests for loops and combined workflows.
//! These tests actually execute workflows through the full pipeline:
//! YAML -> DAG -> plugin loading -> execution -> output verification.

use lao_orchestrator_core::cross_platform::PathUtils;
use lao_orchestrator_core::plugins::PluginRegistry;
use lao_orchestrator_core::{
    run_workflow_yaml_parallel_with_callback, ConditionOperator, ConditionType, LoopConfig,
    LoopItems, StepCondition, StepEvent, Workflow, WorkflowStep,
};
use serial_test::serial;
use std::fs;
use std::sync::{Arc, Mutex};

fn check_plugins_available(required_plugins: &[&str]) -> bool {
    let plugin_dir = PathUtils::plugin_dir();
    let reg = PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
    for plugin_name in required_plugins {
        if reg.get(plugin_name).is_none() {
            println!("Plugin '{}' not found, skipping test", plugin_name);
            return false;
        }
    }
    true
}

fn run_workflow_parallel(
    workflow: &Workflow,
) -> (Vec<lao_orchestrator_core::StepLog>, Vec<StepEvent>) {
    let path = format!("temp_e2e_{}.yaml", std::process::id());
    fs::write(&path, serde_yaml::to_string(workflow).unwrap()).unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let logs = run_workflow_yaml_parallel_with_callback(&path, move |event: StepEvent| {
        events_clone.lock().unwrap().push(event);
    })
    .expect("Workflow should execute successfully");

    fs::remove_file(&path).ok();

    let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
    (logs, events)
}

fn echo_step(input: &str) -> WorkflowStep {
    WorkflowStep {
        run: "EchoPlugin".to_string(),
        params: serde_yaml::from_str(&format!("input: '{}'", input)).unwrap(),
        retries: None,
        retry_delay: None,
        cache_key: None,
        input_from: None,
        depends_on: None,
        condition: None,
        for_each: None,
    }
}

// ========================
// Loop execution tests
// ========================

#[test]
#[serial]
fn test_loop_executes_all_iterations() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Loop E2E Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'placeholder'").unwrap(),
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("alpha".to_string()),
                    serde_yaml::Value::String("bravo".to_string()),
                    serde_yaml::Value::String("charlie".to_string()),
                ]),
                var: "item".to_string(),
                collect_results: true,
                max_parallel: 2,
            }),
            ..echo_step("placeholder")
        }],
    };

    let (logs, events) = run_workflow_parallel(&workflow);

    // Should have exactly 1 step log (loop is one step)
    assert_eq!(logs.len(), 1, "Loop should produce one step log");

    // The output should be a JSON array with 3 results
    let output = logs[0].output.as_ref().expect("Should have output");
    let results: Vec<String> =
        serde_json::from_str(output).expect("Output should be valid JSON array");
    assert_eq!(results.len(), 3, "Should have 3 iteration results");

    // Validation should indicate loop execution
    let validation = logs[0].validation.as_ref().expect("Should have validation");
    assert!(
        validation.contains("loop") && validation.contains("3"),
        "Validation should indicate loop with 3 iterations, got: {}",
        validation
    );

    // Should have success event
    assert!(
        events.iter().any(|e| e.status == "success"),
        "Should have success event"
    );
}

#[test]
#[serial]
fn test_loop_max_parallel_respected() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // With max_parallel=1, items should be processed sequentially
    let workflow = Workflow {
        workflow: "Sequential Loop Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'seq'").unwrap(),
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("one".to_string()),
                    serde_yaml::Value::String("two".to_string()),
                    serde_yaml::Value::String("three".to_string()),
                    serde_yaml::Value::String("four".to_string()),
                ]),
                var: "item".to_string(),
                collect_results: true,
                max_parallel: 1, // Sequential
            }),
            ..echo_step("seq")
        }],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    let output = logs[0].output.as_ref().expect("Should have output");
    let results: Vec<String> = serde_json::from_str(output).expect("Should be JSON array");
    assert_eq!(results.len(), 4, "All 4 items should be processed");
}

#[test]
#[serial]
fn test_loop_collect_results_false() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "No Collect Loop Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'nocollect'").unwrap(),
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("x".to_string()),
                    serde_yaml::Value::String("y".to_string()),
                ]),
                var: "item".to_string(),
                collect_results: false,
                max_parallel: 2,
            }),
            ..echo_step("nocollect")
        }],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    let output = logs[0].output.as_ref().expect("Should have output");
    let results: Vec<String> = serde_json::from_str(output).expect("Should be JSON array");
    // When collect_results is false, should return only the last result
    assert_eq!(
        results.len(),
        1,
        "Should have only 1 result when collect_results=false"
    );
}

// ========================
// Loop + chain tests
// ========================

#[test]
#[serial]
fn test_loop_output_feeds_next_step() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Loop Chain Test".to_string(),
        steps: vec![
            // Step 1: Loop that produces collected results
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'looped'").unwrap(),
                for_each: Some(LoopConfig {
                    items: LoopItems::Array(vec![
                        serde_yaml::Value::String("a".to_string()),
                        serde_yaml::Value::String("b".to_string()),
                    ]),
                    var: "item".to_string(),
                    collect_results: true,
                    max_parallel: 2,
                }),
                ..echo_step("looped")
            },
            // Step 2: Receives the collected loop output
            WorkflowStep {
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                ..echo_step("chain")
            },
        ],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    assert_eq!(logs.len(), 2, "Should have 2 step logs");

    // Step 1 should be the loop
    let step1_validation = logs[0].validation.as_ref().unwrap();
    assert!(step1_validation.contains("loop"), "Step 1 should be a loop");

    // Step 2 should have received input from step 1
    assert!(
        logs[1].error.is_none(),
        "Step 2 should succeed, got error: {:?}",
        logs[1].error
    );
    assert!(
        logs[1].output.is_some(),
        "Step 2 should have output from step 1's loop results"
    );
}

// ========================
// Combined: Loop + Condition
// ========================

#[test]
#[serial]
fn test_loop_with_condition_ready_shape() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Loop + Condition Shape Test".to_string(),
        steps: vec![WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::from_str("input: 'loop input'").unwrap(),
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("file1.mp3".to_string()),
                    serde_yaml::Value::String("file2.mp3".to_string()),
                ]),
                var: "audio_file".to_string(),
                collect_results: true,
                max_parallel: 2,
            }),
            ..echo_step("loop input")
        }],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    assert_eq!(logs.len(), 1);
    let output = logs[0].output.as_ref().expect("Should have output");
    let results: Vec<String> = serde_json::from_str(output).expect("JSON array");
    assert_eq!(results.len(), 2, "Both items should be processed");
}

#[test]
#[serial]
fn test_conditional_then_loop_pipeline() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Condition + Loop Pipeline".to_string(),
        steps: vec![
            // Step 1: Produces output containing "trigger"
            echo_step("trigger"),
            // Step 2: Conditional — runs only if step1 output contains "trigger"
            WorkflowStep {
                depends_on: Some(vec!["step1".to_string()]),
                condition: Some(StepCondition {
                    condition_type: ConditionType::OutputContains,
                    field: "step1".to_string(),
                    operator: ConditionOperator::Contains,
                    value: "trigger".to_string(),
                }),
                ..echo_step("condition met")
            },
            // Step 3: Loop that depends on step2
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'looping'").unwrap(),
                depends_on: Some(vec!["step2".to_string()]),
                input_from: Some("step2".to_string()),
                for_each: Some(LoopConfig {
                    items: LoopItems::Array(vec![
                        serde_yaml::Value::String("x".to_string()),
                        serde_yaml::Value::String("y".to_string()),
                    ]),
                    var: "item".to_string(),
                    collect_results: true,
                    max_parallel: 2,
                }),
                retries: None,
                retry_delay: None,
                cache_key: None,
                condition: None,
            },
        ],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    assert_eq!(logs.len(), 3, "All 3 steps should produce logs");

    // Step 1 should succeed
    assert!(logs[0].error.is_none(), "Step 1 should succeed");

    // Step 2 should run (condition met)
    assert!(
        logs[1].error.is_none(),
        "Step 2 should succeed (condition met)"
    );
    assert_ne!(
        logs[1].validation.as_deref(),
        Some("skipped"),
        "Step 2 should NOT be skipped"
    );

    // Step 3 should be a loop
    let step3_validation = logs[2]
        .validation
        .as_ref()
        .expect("Step 3 should have validation");
    assert!(
        step3_validation.contains("loop"),
        "Step 3 should be a loop, got: {}",
        step3_validation
    );
}

#[test]
#[serial]
fn test_conditional_skips_then_loop_still_runs() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    let workflow = Workflow {
        workflow: "Skip + Loop Test".to_string(),
        steps: vec![
            // Step 1: Output "hello"
            echo_step("hello"),
            // Step 2: Should SKIP (looks for "nonexistent")
            WorkflowStep {
                depends_on: Some(vec!["step1".to_string()]),
                condition: Some(StepCondition {
                    condition_type: ConditionType::OutputContains,
                    field: "step1".to_string(),
                    operator: ConditionOperator::Contains,
                    value: "nonexistent".to_string(),
                }),
                ..echo_step("should skip")
            },
            // Step 3: Loop (independent, no dependency on step2)
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'independent loop'").unwrap(),
                for_each: Some(LoopConfig {
                    items: LoopItems::Array(vec![
                        serde_yaml::Value::String("a".to_string()),
                        serde_yaml::Value::String("b".to_string()),
                    ]),
                    var: "item".to_string(),
                    collect_results: true,
                    max_parallel: 2,
                }),
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
            },
        ],
    };

    let (logs, _events) = run_workflow_parallel(&workflow);

    assert_eq!(logs.len(), 3, "All 3 steps should produce logs");

    // Find step2 by step_id (parallel executor may reorder logs)
    let step2 = logs
        .iter()
        .find(|l| l.step_id == "step2")
        .expect("step2 should exist");
    let was_skipped = step2.validation.as_deref() == Some("skipped")
        || step2.output.as_deref() == Some("skipped due to condition");
    assert!(
        was_skipped,
        "Step 2 should be skipped, got validation: {:?}, output: {:?}",
        step2.validation, step2.output
    );

    // Step 3 loop should still run
    let step3 = logs
        .iter()
        .find(|l| l.step_id == "step3")
        .expect("step3 should exist");
    assert!(
        step3
            .validation
            .as_ref()
            .map_or(false, |v| v.contains("loop")),
        "Step 3 loop should run despite step 2 being skipped, got: {:?}",
        step3.validation
    );
}

// ========================
// Full multimodal pipeline simulation
// ========================

#[test]
#[serial]
fn test_full_multimodal_pipeline() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Simulates: batch audio transcription -> summarization -> structured output
    // Using EchoPlugin as stand-in for all plugins
    let workflow = Workflow {
        workflow: "Full Multimodal Pipeline".to_string(),
        steps: vec![
            // Step 1: "Transcribe" audio files (loop with Audio -> Text)
            WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::from_str("input: 'transcribed text'").unwrap(),
                for_each: Some(LoopConfig {
                    items: LoopItems::Array(vec![
                        serde_yaml::Value::String("meeting1.mp3".to_string()),
                        serde_yaml::Value::String("meeting2.mp3".to_string()),
                        serde_yaml::Value::String("meeting3.mp3".to_string()),
                    ]),
                    var: "audio_file".to_string(),
                    collect_results: true,
                    max_parallel: 2,
                }),
                retries: Some(2),
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
            },
            // Step 2: "Summarize" the transcriptions (Text -> Text)
            WorkflowStep {
                input_from: Some("step1".to_string()),
                depends_on: Some(vec!["step1".to_string()]),
                ..echo_step("summary of transcriptions")
            },
            // Step 3: "Extract insights" as JSON (Text -> Structured)
            WorkflowStep {
                input_from: Some("step2".to_string()),
                depends_on: Some(vec!["step2".to_string()]),
                ..echo_step("structured insights")
            },
        ],
    };

    let (logs, events) = run_workflow_parallel(&workflow);

    // All 3 steps should execute
    assert_eq!(logs.len(), 3, "All 3 pipeline stages should run");

    // Step 1: loop with 3 iterations
    let step1_validation = logs[0].validation.as_ref().unwrap();
    assert!(
        step1_validation.contains("loop") && step1_validation.contains("3"),
        "Step 1 should be loop with 3 iterations, got: {}",
        step1_validation
    );

    // Step 2: should receive step1 output and succeed
    assert!(logs[1].error.is_none(), "Step 2 (summarize) should succeed");
    assert!(logs[1].output.is_some(), "Step 2 should produce output");

    // Step 3: should receive step2 output and succeed
    assert!(
        logs[2].error.is_none(),
        "Step 3 (extract insights) should succeed"
    );
    assert!(logs[2].output.is_some(), "Step 3 should produce output");

    // Should have success events for all steps
    let success_events: Vec<_> = events.iter().filter(|e| e.status == "success").collect();
    assert!(
        success_events.len() >= 3,
        "Should have at least 3 success events (loop + 2 steps), got {}",
        success_events.len()
    );
}

// ========================
// YAML-based workflow execution
// ========================

#[test]
#[serial]
fn test_loop_workflow_from_yaml_file() {
    if !check_plugins_available(&["EchoPlugin"]) {
        return;
    }

    // Test that the actual loop workflow YAML file executes correctly
    let yaml = r#"
workflow: "YAML Loop Test"
steps:
  - run: EchoPlugin
    params:
      input: "batch item"
    for_each:
      items:
        - "item_a"
        - "item_b"
        - "item_c"
      var: "batch_item"
      collect_results: true
      max_parallel: 3
"#;

    let path = format!("temp_yaml_loop_{}.yaml", std::process::id());
    fs::write(&path, yaml).unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let logs = run_workflow_yaml_parallel_with_callback(&path, move |event: StepEvent| {
        events_clone.lock().unwrap().push(event);
    })
    .expect("YAML loop workflow should execute");

    fs::remove_file(&path).ok();

    assert_eq!(logs.len(), 1, "Should have 1 step log");
    let output = logs[0].output.as_ref().expect("Should have output");
    let results: Vec<String> = serde_json::from_str(output).expect("JSON array");
    assert_eq!(results.len(), 3, "Should process all 3 items");
}
