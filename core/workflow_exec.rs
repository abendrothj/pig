use std::collections::HashMap;
use std::env as std_env;
use std::fs;
use std::time::Instant;
use std::{thread, time::Duration};

use crate::cross_platform::PathUtils;
use crate::plugins::*;
use crate::workflow_dag::*;
use crate::workflow_helpers::*;
use crate::workflow_types::*;

pub fn run_workflow_yaml(path: &str) -> Result<Vec<StepLog>, String> {
    let workflow = load_workflow_yaml(path)?;
    let dag = build_dag(&workflow.steps)?;
    let registry = PluginRegistry::default_registry();

    let plugin_count = registry.plugin_count();
    let plugin_names = registry.plugin_names();

    if plugin_count == 0 {
        tracing::error!(" No plugins loaded! Cannot execute workflow.");
        tracing::info!(" Expected plugins directory: {}", PathUtils::plugin_dir().display());
        tracing::info!(" Make sure plugins are built: bash scripts/build-plugins.sh");
        return Err(format!("No plugins loaded. Plugin directory: {}", PathUtils::plugin_dir().display()));
    }

    tracing::debug!(" Executing workflow with {} loaded plugins: {:?}", plugin_count, plugin_names);

    // Validate workflow
    let errors = validate_workflow_types(&dag, &registry);
    if !errors.is_empty() {
        tracing::error!(" Workflow validation failed with {} errors", errors.len());
        tracing::info!(" Loaded plugins: {:?}", plugin_names);
        for (step_idx, error_msg) in &errors {
            tracing::error!(" Step {}: {}", step_idx, error_msg);
        }
        return Err(format!("Workflow validation failed: {:?}", errors));
    }

    // Topological sort
    let execution_order = topo_sort(&dag)?;

    let mut logs = Vec::new();
    let mut outputs: HashMap<String, String> = HashMap::new();
    let start_time = Instant::now();

    for (step_idx, node_id) in execution_order.iter().enumerate() {
        let node = dag.iter().find(|n| &n.id == node_id)
            .ok_or_else(|| format!("Node '{}' not found in DAG", node_id))?;
        let step = &node.step;

        // Build input parameters
        let mut params = step.params.clone();

        // Handle input_from: use output from referenced step as input
        if let Some(input_from) = &step.input_from {
            if let Some(step_output) = outputs.get(input_from) {
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

        substitute_params(&mut params, &outputs);

        // Build plugin input
        let plugin_input = build_plugin_input(&params);

        // Get plugin
        let plugin = registry
            .get(&step.run)
            .ok_or_else(|| format!("Plugin '{}' not found", step.run))?;

        // Run with retries
        let mut last_error = None;
        let max_attempts = step.retries.unwrap_or(1) + 1;

        // Check if step should be executed based on conditions
        let dependent_step = step.depends_on.as_ref().and_then(|deps| deps.first());
        if !should_execute_step(step, &logs, dependent_step.map(|s| s.as_str())) {
            logs.push(StepLog {
                step: step_idx,
                step_id: node_id.clone(),
                runner: step.run.clone(),
                input: params.clone(),
                output: Some("skipped due to condition".to_string()),
                error: None,
                attempt: 1,
                input_type: None,
                output_type: None,
                validation: Some("skipped".to_string()),
            });
            continue;
        }

        for attempt in 1..=max_attempts {
            let _attempt_start = Instant::now();

            // Check cache first
            let mut cache_status = None;
            if let Some(cache_key) = &step.cache_key {
                let cache_dir =
                    std_env::var("LAO_CACHE_DIR").unwrap_or_else(|_| "cache".to_string());
                let cache_path = format!("{}/{}.json", cache_dir, cache_key);
                if let Ok(cached) = fs::read_to_string(&cache_path) {
                    if let Ok(cached_output) = serde_json::from_str::<String>(&cached) {
                        cache_status = Some("cache".to_string());
                        outputs.insert(node_id.clone(), cached_output.clone());
                        logs.push(StepLog {
                            step: step_idx,
                            step_id: node_id.clone(),
                            runner: step.run.clone(),
                            input: params.clone(),
                            output: Some(cached_output),
                            error: None,
                            attempt,
                            input_type: None,
                            output_type: None,
                            validation: cache_status,
                        });
                        break;
                    }
                }
            }

            // Run plugin
            let output_str = plugin.run_plugin(&plugin_input)
                .unwrap_or_else(|e| format!("error: {}", e));

            // Check if output indicates success (not empty and doesn't start with "error:")
            let is_success = !output_str.trim().is_empty()
                && !output_str.trim().to_lowercase().starts_with("error:");

            if is_success {
                // Success
                outputs.insert(node_id.clone(), output_str.clone());

                // Save to cache
                if let Some(cache_key) = &step.cache_key {
                    let cache_dir =
                        std_env::var("LAO_CACHE_DIR").unwrap_or_else(|_| "cache".to_string());
                    fs::create_dir_all(&cache_dir).ok();
                    let cache_path = format!("{}/{}.json", cache_dir, cache_key);
                    if let Ok(cache_json) = serde_json::to_string(&output_str) {
                        fs::write(&cache_path, cache_json).ok();
                        cache_status = Some("saved".to_string());
                    }
                }

                logs.push(StepLog {
                    step: step_idx,
                    step_id: node_id.clone(),
                    runner: step.run.clone(),
                    input: params.clone(),
                    output: Some(output_str),
                    error: None,
                    attempt,
                    input_type: None,
                    output_type: None,
                    validation: cache_status,
                });
                break;
            } else {
                // Error
                last_error = Some(output_str);

                if attempt < max_attempts {
                    let retry_delay = step.retry_delay.unwrap_or(1000);
                    let delay = if attempt > 1 {
                        retry_delay * 2u64.pow(attempt - 2)
                    } else {
                        retry_delay
                    };
                    thread::sleep(Duration::from_millis(delay));
                }
            }
        }

        if let Some(error) = last_error {
            logs.push(StepLog {
                step: step_idx,
                step_id: node_id.clone(),
                runner: step.run.clone(),
                input: params.clone(),
                output: None,
                error: Some(error),
                attempt: max_attempts,
                input_type: None,
                output_type: None,
                validation: None,
            });
            // Continue execution instead of failing the entire workflow
        }
    }

    let _duration = start_time.elapsed();
    Ok(logs)
}

// Streaming runner with callback events
pub fn run_workflow_yaml_with_callback<F>(
    path: &str,
    mut on_event: F,
) -> Result<Vec<StepLog>, String>
where
    F: FnMut(StepEvent) + Send,
{
    let workflow = load_workflow_yaml(path)?;
    let dag = build_dag(&workflow.steps)?;
    let registry = PluginRegistry::default_registry();

    let plugin_count = registry.plugin_count();
    let plugin_names = registry.plugin_names();

    if plugin_count == 0 {
        tracing::error!(" No plugins loaded! Cannot execute workflow.");
        tracing::info!(" Expected plugins directory: {}", PathUtils::plugin_dir().display());
        tracing::info!(" Make sure plugins are built: bash scripts/build-plugins.sh");
        return Err(format!("No plugins loaded. Plugin directory: {}", PathUtils::plugin_dir().display()));
    }

    tracing::debug!(" Executing workflow with {} loaded plugins: {:?}", plugin_count, plugin_names);

    let errors = validate_workflow_types(&dag, &registry);
    if !errors.is_empty() {
        tracing::error!(" Workflow validation failed with {} errors", errors.len());
        tracing::info!(" Loaded plugins: {:?}", plugin_names);
        for (step_idx, error_msg) in &errors {
            tracing::error!(" Step {}: {}", step_idx, error_msg);
        }
        return Err(format!("Workflow validation failed: {:?}", errors));
    }

    let execution_order = topo_sort(&dag)?;

    let mut logs = Vec::new();
    let mut outputs: HashMap<String, String> = HashMap::new();

    for (step_idx, node_id) in execution_order.iter().enumerate() {
        let node = dag.iter().find(|n| &n.id == node_id)
            .ok_or_else(|| format!("Node '{}' not found in DAG", node_id))?;
        let step = &node.step;

        let mut params = step.params.clone();

        // Handle input_from: use output from referenced step as input
        if let Some(input_from) = &step.input_from {
            if let Some(step_output) = outputs.get(input_from) {
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

        substitute_params(&mut params, &outputs);

        let plugin_input = build_plugin_input(&params);
        let plugin = registry
            .get(&step.run)
            .ok_or_else(|| format!("Plugin '{}' not found", step.run))?;

        let mut last_error = None;
        let max_attempts = step.retries.unwrap_or(1) + 1;

        // Check if step should be executed based on conditions
        let dependent_step = step.depends_on.as_ref().and_then(|deps| deps.first());
        if !should_execute_step(step, &logs, dependent_step.map(|s| s.as_str())) {
            on_event(StepEvent {
                step: step_idx,
                step_id: node_id.clone(),
                runner: step.run.clone(),
                status: "skipped".to_string(),
                attempt: 1,
                message: Some("condition not met".to_string()),
                output: None,
                error: None,
            });
            logs.push(StepLog {
                step: step_idx,
                step_id: node_id.clone(),
                runner: step.run.clone(),
                input: params.clone(),
                output: Some("skipped due to condition".to_string()),
                error: None,
                attempt: 1,
                input_type: None,
                output_type: None,
                validation: Some("skipped".to_string()),
            });
            continue;
        }

        on_event(StepEvent {
            step: step_idx,
            step_id: node_id.clone(),
            runner: step.run.clone(),
            status: "running".to_string(),
            attempt: 1,
            message: None,
            output: None,
            error: None,
        });

        for attempt in 1..=max_attempts {
            // Check or compute cache key
            let mut cache_status = None;
            let cache_key_effective = if let Some(k) = &step.cache_key {
                k.clone()
            } else {
                compute_default_cache_key(step, &plugin.info.version)
            };
            let cache_dir = std_env::var("LAO_CACHE_DIR").unwrap_or_else(|_| "cache".to_string());
            let cache_path = format!("{}/{}.json", cache_dir, cache_key_effective);

            if attempt == 1 {
                if let Ok(cached) = fs::read_to_string(&cache_path) {
                    if let Ok(cached_output) = serde_json::from_str::<String>(&cached) {
                        cache_status = Some("cache".to_string());
                        outputs.insert(node_id.clone(), cached_output.clone());
                        on_event(StepEvent {
                            step: step_idx,
                            step_id: node_id.clone(),
                            runner: step.run.clone(),
                            status: "cache".to_string(),
                            attempt,
                            message: Some("cache hit".to_string()),
                            output: Some(cached_output.clone()),
                            error: None,
                        });
                        logs.push(StepLog {
                            step: step_idx,
                            step_id: node_id.clone(),
                            runner: step.run.clone(),
                            input: params.clone(),
                            output: Some(cached_output),
                            error: None,
                            attempt,
                            input_type: None,
                            output_type: None,
                            validation: cache_status,
                        });
                        break;
                    }
                }
            }

            let output_str = plugin.run_plugin(&plugin_input)
                .unwrap_or_else(|e| format!("error: {}", e));

            // Check if output indicates success (not empty and doesn't start with "error:")
            let is_success = !output_str.trim().is_empty()
                && !output_str.trim().to_lowercase().starts_with("error:");

            if is_success {
                outputs.insert(node_id.clone(), output_str.clone());
                if step.cache_key.is_some() {
                    fs::create_dir_all(&cache_dir).ok();
                    let _ = fs::write(
                        &cache_path,
                        serde_json::to_string(&output_str).unwrap_or_default(),
                    );
                }
                on_event(StepEvent {
                    step: step_idx,
                    step_id: node_id.clone(),
                    runner: step.run.clone(),
                    status: "success".to_string(),
                    attempt,
                    message: None,
                    output: Some(output_str.clone()),
                    error: None,
                });
                logs.push(StepLog {
                    step: step_idx,
                    step_id: node_id.clone(),
                    runner: step.run.clone(),
                    input: params.clone(),
                    output: Some(output_str),
                    error: None,
                    attempt,
                    input_type: None,
                    output_type: None,
                    validation: cache_status,
                });
                break;
            } else {
                last_error = Some(output_str.clone());
                on_event(StepEvent {
                    step: step_idx,
                    step_id: node_id.clone(),
                    runner: step.run.clone(),
                    status: "error".to_string(),
                    attempt,
                    message: Some("attempt failed".to_string()),
                    output: None,
                    error: Some(output_str.clone()),
                });
                if attempt < max_attempts {
                    let retry_delay = step.retry_delay.unwrap_or(1000);
                    thread::sleep(Duration::from_millis(retry_delay));
                    on_event(StepEvent {
                        step: step_idx,
                        step_id: node_id.clone(),
                        runner: step.run.clone(),
                        status: "running".to_string(),
                        attempt: attempt + 1,
                        message: Some("retrying".to_string()),
                        output: None,
                        error: None,
                    });
                }
            }
        }

        if let Some(error) = last_error {
            logs.push(StepLog {
                step: step_idx,
                step_id: node_id.clone(),
                runner: step.run.clone(),
                input: params.clone(),
                output: None,
                error: Some(error),
                attempt: max_attempts,
                input_type: None,
                output_type: None,
                validation: None,
            });
        }
    }

    Ok(logs)
}
