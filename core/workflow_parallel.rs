use std::collections::HashMap;
use std::env as std_env;
use std::fs;
use std::{thread, time::Duration};

use crate::cross_platform::PathUtils;
use crate::plugin_result::PluginRunResult;
use crate::plugins::*;
use crate::state_manager::WorkflowStateManager;
use crate::trust::TrustPolicy;
use crate::workflow_dag::*;
use crate::workflow_helpers::*;
use crate::workflow_state::{StepResult, StepStatus, WorkflowState};
use crate::workflow_types::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Execute a step with loop/iteration support
/// Returns Vec of outputs if collecting results, or last output if not
pub fn execute_with_loop(
    step: &WorkflowStep,
    loop_config: &LoopConfig,
    base_params: &serde_yaml::Value,
    outputs: &HashMap<String, String>,
    registry: &std::sync::Arc<std::sync::Mutex<PluginRegistry>>,
) -> Result<Vec<String>, String> {
    // Resolve items to iterate over
    let items = match &loop_config.items {
        LoopItems::Array(arr) => arr.clone(),
        LoopItems::Reference(ref_path) => {
            if let Some(output_str) = outputs.get(ref_path) {
                serde_json::from_str::<Vec<serde_yaml::Value>>(output_str)
                    .map_err(|e| format!("Failed to parse loop items from {}: {}", ref_path, e))?
            } else {
                return Err(format!(
                    "Loop reference '{}' not found in outputs",
                    ref_path
                ));
            }
        }
    };

    let mut results = Vec::new();
    let chunk_size = loop_config.max_parallel;

    for chunk in items.chunks(chunk_size) {
        let mut handles = Vec::new();

        for item in chunk {
            let step_clone = step.clone();
            let mut params = base_params.clone();
            let registry_clone = registry.clone();
            let item_clone = item.clone();
            let var_name = loop_config.var.clone();

            let handle = std::thread::spawn(move || {
                if let Some(mapping) = params.as_mapping_mut() {
                    mapping.insert(serde_yaml::Value::String(var_name), item_clone);
                }

                let plugin_input = build_plugin_input(&params);
                let reg_guard = registry_clone
                    .lock()
                    .expect("plugin registry mutex poisoned");
                let plugin_opt = reg_guard.get(&step_clone.run);

                if let Some(plugin) = plugin_opt {
                    let result = plugin.run_plugin(&plugin_input);
                    if result.is_success() {
                        Ok(result.output.unwrap_or_default())
                    } else {
                        Err(result.display_error())
                    }
                } else {
                    Err(format!("Plugin '{}' not found", step_clone.run))
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            match handle.join() {
                Ok(Ok(output)) => {
                    if loop_config.collect_results {
                        results.push(output);
                    }
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err("Thread panicked during loop execution".to_string()),
            }
        }
    }

    if loop_config.collect_results {
        Ok(results)
    } else {
        Ok(vec![results.last().cloned().unwrap_or_default()])
    }
}

// Group nodes by execution level (nodes at same level can run in parallel)
pub fn group_by_execution_levels(dag: &[DagNode]) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    let mut remaining: std::collections::HashSet<String> =
        dag.iter().map(|n| n.id.clone()).collect();
    let node_map: std::collections::HashMap<String, &DagNode> =
        dag.iter().map(|n| (n.id.clone(), n)).collect();

    while !remaining.is_empty() {
        let mut current_level = Vec::new();
        for node_id in remaining.iter() {
            if let Some(node) = node_map.get(node_id) {
                let all_parents_done = node
                    .parents
                    .iter()
                    .all(|parent_id| !remaining.contains(parent_id));
                if all_parents_done {
                    current_level.push(node_id.clone());
                }
            }
        }

        if current_level.is_empty() {
            break;
        }

        for node_id in &current_level {
            remaining.remove(node_id);
        }

        levels.push(current_level);
    }

    levels
}

fn plugin_input_text(params: &serde_yaml::Value) -> String {
    if let Some(mapping) = params.as_mapping() {
        if let Some(input_val) = mapping.get("input") {
            if let Some(s) = input_val.as_str() {
                return s.to_string();
            }
        }
    }
    serde_yaml::to_string(params).unwrap_or_default()
}

fn new_workflow_run_id(path: &str) -> String {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    if let Ok(duration) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        duration.as_nanos().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn record_workflow_state(path: &str, workflow: &Workflow, dag_len: usize, logs: &[StepLog]) {
    let state_dir = std_env::var("LAO_STATE_DIR").unwrap_or_else(|_| "workflow_states".to_string());
    let Ok(mut manager) = WorkflowStateManager::new(&state_dir) else {
        tracing::warn!("Failed to open workflow state dir: {}", state_dir);
        return;
    };

    let workflow_id = new_workflow_run_id(path);
    let mut state = WorkflowState::new(workflow_id, workflow.workflow.clone(), dag_len);
    state.workflow_path = Some(path.to_string());
    state.start();

    for log in logs {
        let status = if log.error.is_some() {
            StepStatus::Failed
        } else if log.validation.as_deref().is_some_and(|v| v == "skipped") {
            StepStatus::Skipped
        } else {
            StepStatus::Success
        };
        state.add_step_result(StepResult {
            step_id: log.step_id.clone(),
            plugin_name: log.runner.clone(),
            status,
            output: log.output.clone(),
            error: log.error.clone(),
            started_at: std::time::SystemTime::now(),
            completed_at: Some(std::time::SystemTime::now()),
            duration_ms: None,
            retry_count: log.attempt.saturating_sub(1),
        });
        if let Some(out) = &log.output {
            state.outputs.insert(log.step_id.clone(), out.clone());
        }
    }

    if logs.iter().any(|l| l.error.is_some()) {
        let msg = logs
            .iter()
            .find_map(|l| l.error.as_ref())
            .cloned()
            .unwrap_or_else(|| "workflow step failed".to_string());
        state.fail(msg);
    } else {
        state.complete();
    }

    if let Err(e) = manager.save_state(&state) {
        tracing::warn!("Failed to persist workflow state: {}", e);
    }
}

// Parallel execution by levels (nodes on same level run concurrently)
// Strategy: Continue on error (other parallel steps still execute even if one fails)
pub fn run_workflow_yaml_parallel_with_callback<F>(
    path: &str,
    mut on_event: F,
) -> Result<Vec<StepLog>, String>
where
    F: FnMut(StepEvent) + Send,
{
    let workflow = load_workflow_yaml(path)?;
    let trust_policy = TrustPolicy::load_default();
    trust_policy.validate_workflow(&workflow)?;
    let dag = build_dag(&workflow.steps)?;
    let registry = std::sync::Arc::new(std::sync::Mutex::new(PluginRegistry::default_registry()));

    {
        let reg_guard = registry.lock().expect("plugin registry mutex poisoned");
        let plugin_count = reg_guard.plugin_count();
        let plugin_names = reg_guard.plugin_names();

        if plugin_count == 0 {
            tracing::error!(" No plugins loaded! Cannot validate workflow.");
            tracing::info!(
                " Expected plugins directory: {}",
                PathUtils::plugin_dir().display()
            );
            tracing::info!(" Make sure plugins are built: bash scripts/build-plugins.sh");
            return Err(format!(
                "No plugins loaded. Plugin directory: {}",
                PathUtils::plugin_dir().display()
            ));
        }

        tracing::debug!(
            " Validating workflow with {} loaded plugins: {:?}",
            plugin_count,
            plugin_names
        );

        let errors = validate_workflow_types(&dag, &reg_guard);
        if !errors.is_empty() {
            tracing::error!(" Workflow validation failed with {} errors", errors.len());
            tracing::info!(" Loaded plugins: {:?}", plugin_names);
            for (step_idx, error_msg) in &errors {
                tracing::error!(" Step {}: {}", step_idx, error_msg);
            }
            return Err(format!("Workflow validation failed: {:?}", errors));
        }
    }

    let execution_levels = group_by_execution_levels(&dag);
    let node_map: std::collections::HashMap<String, &DagNode> =
        dag.iter().map(|n| (n.id.clone(), n)).collect();

    let logs_mutex = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StepLog>::new()));
    let outputs = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
    let step_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let (event_tx, event_rx) = std::sync::mpsc::channel::<StepEvent>();

    for level in execution_levels {
        let mut handles = Vec::new();

        for node_id in level {
            let node_id_clone = node_id.clone();
            let node = match node_map.get(&node_id_clone) {
                Some(n) => n,
                None => {
                    tracing::error!(
                        " Node '{}' not found in DAG during parallel execution",
                        node_id_clone
                    );
                    continue;
                }
            };
            let step = node.step.clone();
            let outputs_clone = outputs.clone();
            let logs_clone = logs_mutex.clone();
            let step_counter_clone = step_counter.clone();
            let event_tx_clone = event_tx.clone();
            let registry_clone = registry.clone();
            let trust_clone = trust_policy.clone();

            let handle = std::thread::spawn(move || {
                let step_idx = step_counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                // Check if step should be executed based on conditions
                let should_execute = {
                    let logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                    let dependent_step = step.depends_on.as_ref().and_then(|deps| deps.first());
                    should_execute_step(&step, &logs_guard, dependent_step.map(|s| s.as_str()))
                };

                if !should_execute {
                    let _ = event_tx_clone.send(StepEvent {
                        step: step_idx,
                        step_id: node_id_clone.clone(),
                        runner: step.run.clone(),
                        status: "skipped".to_string(),
                        attempt: 1,
                        message: Some("condition not met".to_string()),
                        output: None,
                        error: None,
                    });
                    let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                    logs_guard.push(StepLog {
                        step: step_idx,
                        step_id: node_id_clone,
                        runner: step.run.clone(),
                        input: step.params.clone(),
                        output: Some("skipped due to condition".to_string()),
                        error: None,
                        attempt: 1,
                        input_type: None,
                        output_type: None,
                        validation: Some("skipped".to_string()),
                    });
                    return;
                }

                // Build params with outputs from previous steps
                let mut params = step.params.clone();
                {
                    let outputs_guard = outputs_clone.lock().expect("outputs mutex poisoned");
                    substitute_params(&mut params, &outputs_guard);

                    if let Some(input_from) = &step.input_from {
                        if let Some(step_output) = outputs_guard.get(input_from) {
                            if let Some(mapping) = params.as_mapping_mut() {
                                mapping.insert(
                                    serde_yaml::Value::String("input".to_string()),
                                    serde_yaml::Value::String(step_output.clone()),
                                );
                            } else {
                                let mut new_mapping = serde_yaml::Mapping::new();
                                new_mapping.insert(
                                    serde_yaml::Value::String("input".to_string()),
                                    serde_yaml::Value::String(step_output.clone()),
                                );
                                params = serde_yaml::Value::Mapping(new_mapping);
                            }
                        }
                    }
                }

                // Check if this step has loop configuration
                if let Some(loop_config) = &step.for_each {
                    let outputs_guard = outputs_clone.lock().expect("outputs mutex poisoned");
                    let outputs_map: HashMap<String, String> = outputs_guard.clone();
                    drop(outputs_guard);

                    let _ = event_tx_clone.send(StepEvent {
                        step: step_idx,
                        step_id: node_id_clone.clone(),
                        runner: step.run.clone(),
                        status: "running".to_string(),
                        attempt: 1,
                        message: Some("Executing loop".to_string()),
                        output: None,
                        error: None,
                    });

                    match execute_with_loop(
                        &step,
                        loop_config,
                        &params,
                        &outputs_map,
                        &registry_clone,
                    ) {
                        Ok(loop_results) => {
                            let output_str = serde_json::to_string(&loop_results)
                                .unwrap_or_else(|_| "[]".to_string());

                            let _ = event_tx_clone.send(StepEvent {
                                step: step_idx,
                                step_id: node_id_clone.clone(),
                                runner: step.run.clone(),
                                status: "success".to_string(),
                                attempt: 1,
                                message: Some(format!(
                                    "Loop completed: {} iterations",
                                    loop_results.len()
                                )),
                                output: Some(output_str.clone()),
                                error: None,
                            });

                            let mut outputs_guard =
                                outputs_clone.lock().expect("outputs mutex poisoned");
                            outputs_guard.insert(node_id_clone.clone(), output_str.clone());
                            drop(outputs_guard);

                            let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                            logs_guard.push(StepLog {
                                step: step_idx,
                                step_id: node_id_clone,
                                runner: step.run.clone(),
                                input: params,
                                output: Some(output_str),
                                error: None,
                                attempt: 1,
                                input_type: None,
                                output_type: None,
                                validation: Some(format!("loop:{} iterations", loop_results.len())),
                            });
                            return;
                        }
                        Err(e) => {
                            let _ = event_tx_clone.send(StepEvent {
                                step: step_idx,
                                step_id: node_id_clone.clone(),
                                runner: step.run.clone(),
                                status: "error".to_string(),
                                attempt: 1,
                                message: None,
                                output: None,
                                error: Some(e.clone()),
                            });

                            let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                            logs_guard.push(StepLog {
                                step: step_idx,
                                step_id: node_id_clone,
                                runner: step.run.clone(),
                                input: params,
                                output: None,
                                error: Some(e),
                                attempt: 1,
                                input_type: None,
                                output_type: None,
                                validation: None,
                            });
                            return;
                        }
                    }
                }

                let plugin_input = build_plugin_input(&params);
                let input_text = plugin_input_text(&params);

                // Get plugin info (need version for cache key)
                let plugin_info = {
                    let reg_guard = registry_clone
                        .lock()
                        .expect("plugin registry mutex poisoned");
                    reg_guard
                        .get(&step.run)
                        .map(|p| (p.info.name.clone(), p.info.version.clone()))
                };

                if plugin_info.is_none() {
                    let _ = event_tx_clone.send(StepEvent {
                        step: step_idx,
                        step_id: node_id_clone.clone(),
                        runner: step.run.clone(),
                        status: "error".to_string(),
                        attempt: 1,
                        message: Some(format!("Plugin '{}' not found", step.run)),
                        output: None,
                        error: Some(format!("Plugin '{}' not found", step.run)),
                    });
                    let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                    logs_guard.push(StepLog {
                        step: step_idx,
                        step_id: node_id_clone,
                        runner: step.run.clone(),
                        input: params,
                        output: None,
                        error: Some("Plugin not found".to_string()),
                        attempt: 1,
                        input_type: None,
                        output_type: None,
                        validation: None,
                    });
                    return;
                }

                let plugin_name = step.run.clone();
                let (_, plugin_version) = plugin_info.expect("checked above");
                let max_attempts = step.retries.unwrap_or(1) + 1;
                let mut last_error = None;

                // Check cache on first attempt
                let mut cache_status = None;
                let cache_key_effective = if let Some(k) = &step.cache_key {
                    k.clone()
                } else {
                    compute_default_cache_key(&step, &plugin_version)
                };
                let cache_dir =
                    std_env::var("LAO_CACHE_DIR").unwrap_or_else(|_| "cache".to_string());
                let cache_path = format!("{}/{}.json", cache_dir, cache_key_effective);

                // Try cache first
                if let Ok(cached) = fs::read_to_string(&cache_path) {
                    if let Ok(cached_output) = serde_json::from_str::<String>(&cached) {
                        cache_status = Some("cache".to_string());
                        let mut outputs_guard =
                            outputs_clone.lock().expect("outputs mutex poisoned");
                        outputs_guard.insert(node_id_clone.clone(), cached_output.clone());
                        let _ = event_tx_clone.send(StepEvent {
                            step: step_idx,
                            step_id: node_id_clone.clone(),
                            runner: plugin_name.clone(),
                            status: "cache".to_string(),
                            attempt: 1,
                            message: Some("cache hit".to_string()),
                            output: Some(cached_output.clone()),
                            error: None,
                        });
                        let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                        logs_guard.push(StepLog {
                            step: step_idx,
                            step_id: node_id_clone,
                            runner: plugin_name,
                            input: params,
                            output: Some(cached_output),
                            error: None,
                            attempt: 1,
                            input_type: None,
                            output_type: None,
                            validation: cache_status,
                        });
                        return;
                    }
                }

                // Execute with retries
                for attempt in 1..=max_attempts {
                    let _ = event_tx_clone.send(StepEvent {
                        step: step_idx,
                        step_id: node_id_clone.clone(),
                        runner: plugin_name.clone(),
                        status: "running".to_string(),
                        attempt,
                        message: if attempt > 1 {
                            Some("retrying".to_string())
                        } else {
                            None
                        },
                        output: None,
                        error: None,
                    });

                    // Execute plugin (serialized access through registry lock)
                    let run_result: PluginRunResult = {
                        if let Err(e) = trust_clone.validate_step_input(&plugin_name, &input_text) {
                            PluginRunResult::runtime_error(e)
                        } else {
                            let reg_guard = registry_clone
                                .lock()
                                .expect("plugin registry mutex poisoned");
                            if let Some(plugin) = reg_guard.get(&plugin_name) {
                                plugin.run_plugin(&plugin_input)
                            } else {
                                PluginRunResult::runtime_error(format!(
                                    "Plugin '{}' not found",
                                    plugin_name
                                ))
                            }
                        }
                    };

                    if run_result.is_success() {
                        let output_str = run_result.output.unwrap_or_default();
                        let mut outputs_guard =
                            outputs_clone.lock().expect("outputs mutex poisoned");
                        outputs_guard.insert(node_id_clone.clone(), output_str.clone());

                        // Save to cache if cache_key is set
                        if step.cache_key.is_some() {
                            fs::create_dir_all(&cache_dir).ok();
                            if fs::write(
                                &cache_path,
                                serde_json::to_string(&output_str).unwrap_or_default(),
                            )
                            .is_ok()
                            {
                                cache_status = Some("saved".to_string());
                            }
                        }

                        let _ = event_tx_clone.send(StepEvent {
                            step: step_idx,
                            step_id: node_id_clone.clone(),
                            runner: plugin_name.clone(),
                            status: "success".to_string(),
                            attempt,
                            message: None,
                            output: Some(output_str.clone()),
                            error: None,
                        });

                        let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                        logs_guard.push(StepLog {
                            step: step_idx,
                            step_id: node_id_clone,
                            runner: plugin_name,
                            input: params,
                            output: Some(output_str),
                            error: None,
                            attempt,
                            input_type: None,
                            output_type: None,
                            validation: cache_status,
                        });
                        return;
                    } else {
                        let output_str = run_result.display_error();
                        last_error = Some(output_str.clone());
                        let _ = event_tx_clone.send(StepEvent {
                            step: step_idx,
                            step_id: node_id_clone.clone(),
                            runner: plugin_name.clone(),
                            status: "error".to_string(),
                            attempt,
                            message: Some("attempt failed".to_string()),
                            output: None,
                            error: Some(output_str.clone()),
                        });

                        if attempt < max_attempts {
                            let retry_delay = step.retry_delay.unwrap_or(1000);
                            thread::sleep(Duration::from_millis(retry_delay));
                        }
                    }
                }

                // All attempts failed
                if let Some(error) = last_error {
                    let mut logs_guard = logs_clone.lock().expect("logs mutex poisoned");
                    logs_guard.push(StepLog {
                        step: step_idx,
                        step_id: node_id_clone,
                        runner: plugin_name,
                        input: params,
                        output: None,
                        error: Some(error),
                        attempt: max_attempts,
                        input_type: None,
                        output_type: None,
                        validation: None,
                    });
                }
            });

            handles.push(handle);
        }

        // Wait for all nodes in this level to complete
        for handle in handles {
            if let Err(e) = handle.join() {
                tracing::error!(" Thread panicked during parallel execution: {:?}", e);
            }
        }
    }

    // Close event channel and collect all events
    drop(event_tx);

    let mut collected_events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        collected_events.push(event);
    }
    while let Ok(event) = event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
        collected_events.push(event);
    }

    // Process events in order
    collected_events.sort_by_key(|e| e.step);
    for event in collected_events {
        on_event(event);
    }

    let mut logs = {
        let logs_guard = logs_mutex.lock().expect("logs mutex poisoned");
        logs_guard.iter().cloned().collect::<Vec<StepLog>>()
    };

    logs.sort_by_key(|l| l.step);
    record_workflow_state(path, &workflow, dag.len(), &logs);
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dag_node(id: &str, parents: Vec<&str>, plugin: &str) -> DagNode {
        DagNode {
            id: id.to_string(),
            step: WorkflowStep {
                run: plugin.to_string(),
                params: serde_yaml::Value::Null,
                depends_on: if parents.is_empty() {
                    None
                } else {
                    Some(parents.iter().map(|p| p.to_string()).collect())
                },
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                for_each: None,
                condition: None,
            },
            parents: parents.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn test_group_single_node() {
        let dag = vec![make_dag_node("a", vec![], "EchoPlugin")];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0], vec!["a"]);
    }

    #[test]
    fn test_group_linear_chain() {
        // a -> b -> c
        let dag = vec![
            make_dag_node("a", vec![], "EchoPlugin"),
            make_dag_node("b", vec!["a"], "EchoPlugin"),
            make_dag_node("c", vec!["b"], "EchoPlugin"),
        ];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], vec!["a"]);
        assert_eq!(levels[1], vec!["b"]);
        assert_eq!(levels[2], vec!["c"]);
    }

    #[test]
    fn test_group_parallel_nodes() {
        // a and b are independent, c depends on both
        let dag = vec![
            make_dag_node("a", vec![], "EchoPlugin"),
            make_dag_node("b", vec![], "EchoPlugin"),
            make_dag_node("c", vec!["a", "b"], "EchoPlugin"),
        ];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 2);

        // First level should contain both a and b (order may vary)
        assert_eq!(levels[0].len(), 2);
        assert!(levels[0].contains(&"a".to_string()));
        assert!(levels[0].contains(&"b".to_string()));

        // Second level is just c
        assert_eq!(levels[1], vec!["c"]);
    }

    #[test]
    fn test_group_diamond_dag() {
        //     a
        //    / \
        //   b   c
        //    \ /
        //     d
        let dag = vec![
            make_dag_node("a", vec![], "EchoPlugin"),
            make_dag_node("b", vec!["a"], "EchoPlugin"),
            make_dag_node("c", vec!["a"], "EchoPlugin"),
            make_dag_node("d", vec!["b", "c"], "EchoPlugin"),
        ];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 3);

        assert_eq!(levels[0], vec!["a"]);
        assert_eq!(levels[1].len(), 2);
        assert!(levels[1].contains(&"b".to_string()));
        assert!(levels[1].contains(&"c".to_string()));
        assert_eq!(levels[2], vec!["d"]);
    }

    #[test]
    fn test_group_empty_dag() {
        let dag: Vec<DagNode> = vec![];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 0);
    }

    #[test]
    fn test_group_wide_fan_out() {
        // a -> b, c, d, e (all parallel)
        let dag = vec![
            make_dag_node("a", vec![], "EchoPlugin"),
            make_dag_node("b", vec!["a"], "EchoPlugin"),
            make_dag_node("c", vec!["a"], "EchoPlugin"),
            make_dag_node("d", vec!["a"], "EchoPlugin"),
            make_dag_node("e", vec!["a"], "EchoPlugin"),
        ];
        let levels = group_by_execution_levels(&dag);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].len(), 1);
        assert_eq!(levels[1].len(), 4);
    }
}
