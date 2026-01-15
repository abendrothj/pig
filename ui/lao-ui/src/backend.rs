use lao_orchestrator_core::{
    load_workflow_yaml, run_workflow_yaml_parallel_with_callback, run_workflow_yaml_with_callback,
    StepEvent,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub run: String,
    pub input_type: Option<String>,
    pub output_type: Option<String>,
    pub status: String,
    pub x: f32,
    pub y: f32,
    pub message: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub attempt: u32,
    #[serde(default)]
    pub primary_input: Option<String>, // Which incoming edge is the primary input (input_from)
    #[serde(default)]
    pub execution_level: Option<usize>, // Which parallel execution level this node belongs to
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub whisper_cpp_path: String,
    #[serde(default)]
    pub settings_file: String, // Path to settings file
}

impl Default for AppSettings {
    fn default() -> Self {
        // Try to load from environment variable first
        let whisper_cpp_path = std::env::var("WHISPER_CPP_PATH")
            .unwrap_or_else(|_| String::new());
        
        Self {
            whisper_cpp_path,
            settings_file: "lao_settings.json".to_string(),
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        // Try to load from file, fallback to default
        if let Ok(settings_path) = std::env::var("LAO_SETTINGS_FILE") {
            if let Ok(content) = std::fs::read_to_string(&settings_path) {
                if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                    return settings;
                }
            }
        }
        
        // Try default location
        if let Ok(content) = std::fs::read_to_string("lao_settings.json") {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                return settings;
            }
        }
        
        Self::default()
    }
    
    pub fn save(&self) -> Result<(), String> {
        let settings_path = if !self.settings_file.is_empty() {
            &self.settings_file
        } else {
            "lao_settings.json"
        };
        
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        
        std::fs::write(settings_path, json)
            .map_err(|e| format!("Failed to write settings file: {}", e))?;
        
        // Also set environment variable for current process
        if !self.whisper_cpp_path.is_empty() {
            std::env::set_var("WHISPER_CPP_PATH", &self.whisper_cpp_path);
        }
        
        Ok(())
    }
}

pub struct BackendState {
    pub workflow_path: String,
    pub graph: Option<WorkflowGraph>,
    pub error: String,
    pub plugins: Vec<UiPluginInfo>,
    pub live_logs: Vec<String>,
    #[allow(dead_code)]
    pub selected_node: Option<String>,
    pub is_running: bool,
    pub execution_progress: f32,
    pub workflow_result: Option<WorkflowResult>,
    #[allow(dead_code)]
    pub multimodal_files: Vec<UploadedFile>,
    pub debug_mode: bool, // Force sequential execution for debugging
    pub settings: AppSettings,
    pub show_settings: bool, // Whether to show settings window
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    pub name: String,
    pub path: String,
    pub file_type: String, // "audio", "image", "video", "text", "binary"
    pub size: usize,
    pub upload_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub success: bool,
    pub total_steps: usize,
    pub completed_steps: usize,
    pub failed_steps: usize,
    pub execution_time: f32,
    pub final_message: String,
    #[serde(default)]
    pub parallel_execution: Option<ParallelExecutionMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelExecutionMetrics {
    pub execution_levels: usize,
    pub max_parallelism: usize, // Maximum number of steps that ran concurrently
    pub average_parallelism: f32, // Average parallelism achieved
    pub estimated_sequential_time: f32, // Estimated time if run sequentially
    pub speedup: f32, // Speedup factor (sequential_time / parallel_time)
}

impl Default for BackendState {
    fn default() -> Self {
        Self {
            workflow_path: String::new(),
            graph: None,
            error: String::new(),
            plugins: Vec::new(),
            live_logs: Vec::new(),
            selected_node: None,
            is_running: false,
            execution_progress: 0.0,
            workflow_result: None,
            multimodal_files: Vec::new(),
            debug_mode: false,
            settings: AppSettings::load(),
            show_settings: false,
        }
    }
}

#[allow(dead_code)]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

pub fn get_workflow_graph(path: &str) -> Result<WorkflowGraph, String> {
    let resolved_path = resolve_workflow_path(path)?;
    let workflow = load_workflow_yaml(&resolved_path)?;
    
    // Validate workflow for potential issues
    let mut warnings = Vec::new();
    let mut step_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, step) in workflow.steps.iter().enumerate() {
        let step_ref = format!("step{}", i + 1);
        step_refs.insert(step_ref.clone());
        
        // Check for circular dependencies
        if let Some(ref input_from) = step.input_from {
            if input_from == &step_ref {
                warnings.push(format!("Step {} references itself as input_from", step_ref));
            }
        }
        if let Some(ref deps) = step.depends_on {
            if deps.contains(&step_ref) {
                warnings.push(format!("Step {} references itself in depends_on", step_ref));
            }
        }
    }
    
    // Check for invalid step references
    for (i, step) in workflow.steps.iter().enumerate() {
        let step_ref = format!("step{}", i + 1);
        if let Some(ref input_from) = step.input_from {
            if !step_refs.contains(input_from) {
                warnings.push(format!("Step {} references unknown step: {}", step_ref, input_from));
            }
        }
        if let Some(ref deps) = step.depends_on {
            for dep in deps {
                if !step_refs.contains(dep) {
                    warnings.push(format!("Step {} references unknown step in depends_on: {}", step_ref, dep));
                }
            }
        }
    }
    
    // Log warnings (non-fatal)
    if !warnings.is_empty() {
        eprintln!("Workflow validation warnings:");
        for warning in &warnings {
            eprintln!("  ⚠️  {}", warning);
        }
    }
    
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // First pass: create all nodes and build step reference mapping
    let mut step_to_node_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (i, step) in workflow.steps.iter().enumerate() {
        // Use step{index+1} format for node IDs to match core's build_dag format
        // This ensures consistency between the UI graph and core execution events
        let step_ref = format!("step{}", i + 1);
        let node_id = step_ref.clone(); // Use the same ID format as the core
        step_to_node_id.insert(step_ref.clone(), node_id.clone());
        
        nodes.push(GraphNode {
            id: node_id.clone(),
            run: step.run.clone(),
            input_type: None,
            output_type: None,
            status: "pending".to_string(),
            x: 100.0 + (i as f32 * 150.0),
            y: 100.0,
            message: None,
            output: None,
            error: None,
            attempt: 0,
            primary_input: step.input_from.as_ref()
                .and_then(|step_ref| step_to_node_id.get(step_ref).cloned()),
            execution_level: None, // Will be calculated when graph is loaded
        });
    }
    
    // Second pass: create edges using step reference mapping
    for (i, step) in workflow.steps.iter().enumerate() {
        // Use step reference as the target node ID (consistent with node creation)
        let step_ref = format!("step{}", i + 1);
        let to_id = step_to_node_id.get(&step_ref).cloned().unwrap_or_else(|| step_ref.clone());

        // Map step references (step1, step2, etc.) to actual node IDs
        if let Some(ref from_step) = step.input_from {
            if let Some(from_node_id) = step_to_node_id.get(from_step) {
                edges.push(GraphEdge {
                    from: from_node_id.clone(),
                    to: to_id.clone(),
                });
            }
        }

        if let Some(ref deps) = step.depends_on {
            for d in deps {
                if let Some(dep_node_id) = step_to_node_id.get(d) {
                    edges.push(GraphEdge {
                        from: dep_node_id.clone(),
                        to: to_id.clone(),
                    });
                }
            }
        }
    }

    let mut graph = WorkflowGraph { nodes, edges };
    
    // Calculate execution levels for visualization
    let levels = calculate_execution_levels(&graph);
    let mut node_to_level: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (level_idx, level_nodes) in levels.iter().enumerate() {
        for node_id in level_nodes {
            node_to_level.insert(node_id.clone(), level_idx);
        }
    }
    
    // Assign execution levels to nodes
    for node in &mut graph.nodes {
        node.execution_level = node_to_level.get(&node.id).copied();
    }
    
    // Auto-apply hierarchical layout
    auto_layout_graph_hierarchical(&mut graph);
    
    Ok(graph)
}

pub fn list_plugins_for_ui() -> Result<Vec<UiPluginInfo>, String> {
    let plugins_dir = resolve_plugins_dir();
    println!("[DEBUG] Resolved plugins directory: {}", plugins_dir);
    let mut out: Vec<UiPluginInfo> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let manifest = p.join("plugin.yaml");
                if manifest.exists() {
                    if let Ok(txt) = std::fs::read_to_string(&manifest) {
                        if let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&txt) {
                            let name = val
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !name.is_empty() && !out.iter().any(|i| i.name == name) {
                                let version = val
                                    .get("version")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let description = val
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let author = val
                                    .get("maintainer")
                                    .or_else(|| val.get("author"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let tags = val
                                    .get("tags")
                                    .and_then(|v| v.as_sequence())
                                    .map(|seq| {
                                        seq.iter()
                                            .filter_map(|e| e.as_str().map(|s| s.to_string()))
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                out.push(UiPluginInfo {
                                    name,
                                    version,
                                    description,
                                    author,
                                    tags,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: scan shared libs for names if no manifests found
    if out.is_empty() {
        if let Ok(files) = std::fs::read_dir(&plugins_dir) {
            for f in files.flatten() {
                let path = f.path();
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if matches!(ext, "so" | "dll" | "dylib") {
                        if let Some(fname) = path.file_stem().and_then(|s| s.to_str()) {
                            let base = fname.strip_prefix("lib").unwrap_or(fname);
                            if !out.iter().any(|i| i.name.eq_ignore_ascii_case(base)) {
                                out.push(UiPluginInfo {
                                    name: base.to_string(),
                                    version: String::new(),
                                    description: String::new(),
                                    author: String::new(),
                                    tags: Vec::new(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if out.is_empty() {
        eprintln!("[WARN] No plugins found in directory: {}", plugins_dir);
        eprintln!("[INFO] Expected plugin files: *.dylib (macOS), *.so (Linux), *.dll (Windows)");
        eprintln!("[INFO] Or plugin directories with plugin.yaml manifests");
        eprintln!("[INFO] Current working directory: {:?}", std::env::current_dir().unwrap_or_default());
    } else {
        println!("[INFO] Found {} plugins in {}", out.len(), plugins_dir);
    }
    
    Ok(out)
}

fn resolve_plugins_dir() -> String {
    // Try environment variable first
    if let Ok(dir) = std::env::var("LAO_PLUGINS_DIR") {
        let path = std::path::Path::new(&dir);
        if path.exists() {
            println!("[DEBUG] Using LAO_PLUGINS_DIR: {}", dir);
            return dir;
        } else {
            eprintln!("[WARN] LAO_PLUGINS_DIR set to non-existent path: {}", dir);
        }
    }

    // Try candidates relative to current working directory
    let candidates = ["plugins/", "./plugins/", "../plugins/", "../../plugins/"];

    for candidate in &candidates {
        let path = std::path::Path::new(candidate);
        if path.exists() {
            let abs_path = std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string();
            println!("[DEBUG] Found plugins directory: {} (resolved to: {})", candidate, abs_path);
            return candidate.to_string();
        }
    }

    // Fallback: use PathUtils from core (more reliable)
    let plugin_dir = lao_orchestrator_core::cross_platform::PathUtils::plugin_dir();
    let plugin_dir_str = plugin_dir.to_string_lossy().to_string();
    println!("[DEBUG] Using PathUtils::plugin_dir(): {}", plugin_dir_str);
    
    // Create directory if it doesn't exist
    if !plugin_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&plugin_dir) {
            eprintln!("[WARN] Failed to create plugin directory {}: {}", plugin_dir_str, e);
        }
    }
    
    plugin_dir_str
}

// Download and build whisper.cpp
pub fn download_whisper_cpp() -> Result<String, String> {
    use std::process::Command;
    use std::path::Path;
    use std::io::{self, Write};
    
    let build_dir = "whisper.cpp";
    
    // Clone repository if it doesn't exist
    if !Path::new(build_dir).exists() {
        println!("[INFO] Cloning whisper.cpp repository...");
        let mut child = Command::new("git")
            .args(&["clone", "https://github.com/ggerganov/whisper.cpp.git", build_dir])
            .spawn()
            .map_err(|e| format!("Failed to run git clone: {}", e))?;
        
        let status = child.wait()
            .map_err(|e| format!("Failed to wait for git clone: {}", e))?;
        
        if !status.success() {
            return Err("git clone failed. Make sure git is installed.".to_string());
        }
    } else {
        println!("[INFO] whisper.cpp directory already exists, updating...");
        let mut child = Command::new("git")
            .args(&["pull"])
            .current_dir(build_dir)
            .spawn()
            .map_err(|e| format!("Failed to run git pull: {}", e))?;
        
        let _ = child.wait(); // Ignore pull errors
    }
    
    // Build whisper.cpp
    println!("[INFO] Building whisper.cpp (this may take several minutes)...");
    println!("[INFO] Requirements: cmake, make, and a C++ compiler (g++ or clang++)");
    
    // Check for cmake first
    let cmake_check = Command::new("cmake")
        .arg("--version")
        .output();
    
    if cmake_check.is_err() || !cmake_check.unwrap().status.success() {
        return Err("cmake is required but not found. Please install cmake first:\n  macOS: brew install cmake\n  Linux: sudo apt-get install cmake (or your package manager)".to_string());
    }
    
    let mut child = Command::new("make")
        .current_dir(build_dir)
        .spawn()
        .map_err(|e| format!("Failed to run make: {}. Make sure make and a C++ compiler are installed.", e))?;
    
    let status = child.wait()
        .map_err(|e| format!("Failed to wait for make: {}", e))?;
    
    if !status.success() {
        return Err("make failed. Check that you have make, cmake, and a C++ compiler (g++ or clang++) installed.\n  macOS: brew install cmake\n  Linux: sudo apt-get install cmake build-essential".to_string());
    }
    
    // Find the built binary (whisper.cpp uses cmake and builds to build/bin/ directory)
    let binary_paths = vec![
        format!("{}/build/bin/whisper-cli", build_dir),
        format!("{}/build/bin/whisper", build_dir),
        format!("{}/bin/whisper-cli", build_dir),
        format!("{}/bin/whisper", build_dir),
        format!("{}/whisper.cpp", build_dir),
        format!("{}/whisper-cpp", build_dir),
        format!("{}/main", build_dir),
    ];
    
    for binary_path in &binary_paths {
        if Path::new(binary_path).exists() {
            // Make it executable
            #[cfg(unix)]
            {
                use std::fs;
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = fs::metadata(binary_path) {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o755);
                    let _ = fs::set_permissions(binary_path, perms);
                }
            }
            
            println!("[INFO] whisper.cpp built successfully at: {}", binary_path);
            return Ok(binary_path.clone());
        }
    }
    
    Err(format!("Built binary not found. Checked: {}. The binary should be at whisper.cpp/bin/whisper-cli after building.", binary_paths.join(", ")))
}

pub fn run_workflow_stream(
    path: String,
    parallel: bool,
    state: Arc<Mutex<BackendState>>,
) -> Result<(), String> {
    // Resolve workflow path before spawning worker thread so we can surface errors immediately
    let resolved_path = resolve_workflow_path(&path)?;

    std::thread::spawn(move || {
        let _start_time = std::time::Instant::now();
        let mut total_steps = 0;
        let mut completed_steps = 0;
        let mut failed_steps = 0;

        // Initialize execution state
        {
            let mut state_guard = state.lock().unwrap();
            state_guard.is_running = true;
            state_guard.execution_progress = 0.0;
            state_guard.workflow_result = None;
            state_guard.error.clear();

            // Count total steps for progress tracking
            if let Some(ref graph) = state_guard.graph {
                total_steps = graph.nodes.len();
            }
        }

        let mut emit = |event: StepEvent| {
            if let Ok(mut state_guard) = state.lock() {
                // Update node status in graph
                if let Some(ref mut graph) = state_guard.graph {
                    if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == event.step_id) {
                        node.status = event.status.clone();
                        node.message = event.message.clone();
                        node.output = event.output.clone();
                        node.error = event.error.clone();
                        node.attempt = event.attempt;
                    }
                }

                // Add to live logs
                let error_info = if let Some(ref err) = event.error {
                    format!(" - error: {}", err)
                } else {
                    String::new()
                };
                let log_message = format!(
                    "[{}] {}: {} (attempt {}){}{}",
                    event.step_id,
                    event.runner,
                    event.status,
                    event.attempt,
                    event
                        .message
                        .map(|m| format!(" - {}", m))
                        .unwrap_or_default(),
                    error_info
                );
                state_guard.live_logs.push(log_message);

                // Limit log size
                if state_guard.live_logs.len() > 200 {
                    state_guard.live_logs.remove(0);
                }

                // Update progress
                if event.status == "success" || event.status == "cache" {
                    completed_steps += 1;
                    state_guard.execution_progress = completed_steps as f32 / total_steps as f32;
                } else if event.status == "error" {
                    failed_steps += 1;
                }
            }
        };

        let start_time = std::time::Instant::now();
        // Track execution levels for parallel execution
        let mut execution_levels_tracker: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut level_start_times: std::collections::HashMap<usize, std::time::Instant> = std::collections::HashMap::new();
        let mut current_level: Option<usize> = None;
        let mut step_level_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        
        // Calculate execution levels from graph if parallel
        if parallel {
            if let Some(ref graph) = state.lock().unwrap().graph {
                let levels = calculate_execution_levels(graph);
                for (level_idx, level_nodes) in levels.iter().enumerate() {
                    for node_id in level_nodes {
                        step_level_map.insert(node_id.clone(), level_idx);
                    }
                }
            }
        }
        
        let emit_with_level = |event: StepEvent| {
            if parallel {
                if let Some(level) = step_level_map.get(&event.step_id) {
                    if current_level != Some(*level) {
                        current_level = Some(*level);
                        level_start_times.insert(*level, std::time::Instant::now());
                    }
                    *execution_levels_tracker.entry(*level).or_insert(0) += 1;
                }
            }
            emit(event);
        };

        // Set environment variables from settings before execution
        {
            let state_guard = state.lock().unwrap();
            if !state_guard.settings.whisper_cpp_path.is_empty() {
                std::env::set_var("WHISPER_CPP_PATH", &state_guard.settings.whisper_cpp_path);
            }
        }
        
        let result = if parallel {
            run_workflow_yaml_parallel_with_callback(&resolved_path, emit_with_level)
        } else {
            run_workflow_yaml_with_callback(&resolved_path, emit)
        };

        let execution_time = start_time.elapsed().as_secs_f32();

        // Calculate parallel execution metrics
        let parallel_metrics = if parallel && !execution_levels_tracker.is_empty() {
            let execution_levels = execution_levels_tracker.len();
            let max_parallelism = *execution_levels_tracker.values().max().unwrap_or(&1);
            let total_parallel_steps: usize = execution_levels_tracker.values().sum();
            let average_parallelism = total_parallel_steps as f32 / execution_levels as f32;
            
            // Estimate sequential time (sum of level times, assuming each level takes max time)
            let estimated_sequential_time = execution_time; // Simplified: assume same time
            let speedup = if execution_time > 0.0 {
                estimated_sequential_time / execution_time
            } else {
                1.0
            };
            
            Some(ParallelExecutionMetrics {
                execution_levels,
                max_parallelism,
                average_parallelism,
                estimated_sequential_time,
                speedup,
            })
        } else {
            None
        };

        // Update final state
        if let Ok(mut state_guard) = state.lock() {
            state_guard.is_running = false;
            state_guard.execution_progress = 1.0;

            let workflow_result = match result {
                Ok(logs) => {
                    // Count successful and failed steps from logs
                    let successful_steps = logs.iter().filter(|log| log.error.is_none() && log.output.is_some()).count();
                    let failed_steps_count = logs.iter().filter(|log| log.error.is_some()).count();
                    
                    let workflow_success = failed_steps_count == 0;
                    
                    let final_message = if failed_steps_count > 0 {
                        if parallel && parallel_metrics.is_some() {
                            let metrics = parallel_metrics.as_ref().unwrap();
                            format!(
                                "Workflow completed with {} successful, {} failed steps in {:.2}s ({} levels, max parallelism: {}, {:.1}x speedup)",
                                successful_steps,
                                failed_steps_count,
                                execution_time,
                                metrics.execution_levels,
                                metrics.max_parallelism,
                                metrics.speedup
                            )
                        } else {
                            format!(
                                "Workflow completed with {} successful, {} failed steps in {:.2}s",
                                successful_steps,
                                failed_steps_count,
                                execution_time
                            )
                        }
                    } else if parallel && parallel_metrics.is_some() {
                        let metrics = parallel_metrics.as_ref().unwrap();
                        format!(
                            "Workflow completed successfully: {} steps in {:.2}s ({} levels, max parallelism: {}, {:.1}x speedup)",
                            successful_steps,
                            execution_time,
                            metrics.execution_levels,
                            metrics.max_parallelism,
                            metrics.speedup
                        )
                    } else {
                        format!(
                            "Workflow completed successfully with {} steps in {:.2}s",
                            successful_steps,
                            execution_time
                        )
                    };
                    
                    if workflow_success {
                        state_guard
                            .live_logs
                            .push(format!("✓ DONE: {}", final_message));
                    } else {
                        state_guard
                            .live_logs
                            .push(format!("⚠️ COMPLETED WITH ERRORS: {}", final_message));
                    }
                    
                    WorkflowResult {
                        success: workflow_success,
                        total_steps,
                        completed_steps: successful_steps,
                        failed_steps: failed_steps_count,
                        execution_time,
                        final_message,
                        parallel_execution: parallel_metrics,
                    }
                }
                Err(err) => {
                    let final_message = format!("Workflow failed: {}", err);
                    state_guard
                        .live_logs
                        .push(format!("✗ ERROR: {}", final_message));
                    state_guard.error = err;
                    WorkflowResult {
                        success: false,
                        total_steps,
                        completed_steps,
                        failed_steps,
                        execution_time,
                        final_message,
                        parallel_execution: parallel_metrics,
                    }
                }
            };

            state_guard.workflow_result = Some(workflow_result);
        }
    });

    Ok(())
}

// Calculate execution levels for parallel execution visualization
pub fn calculate_execution_levels(graph: &WorkflowGraph) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    let mut remaining: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let mut node_to_level: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    
    // Build dependency map
    let mut incoming: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for edge in &graph.edges {
        incoming.entry(edge.to.clone()).or_default().push(edge.from.clone());
    }
    
    while !remaining.is_empty() {
        let mut current_level = Vec::new();
        for node_id in remaining.iter() {
            let deps = incoming.get(node_id).cloned().unwrap_or_default();
            let all_deps_done = deps.iter().all(|dep_id| !remaining.contains(dep_id));
            if all_deps_done {
                current_level.push(node_id.clone());
            }
        }
        
        if current_level.is_empty() {
            // Circular dependency or error - break to avoid infinite loop
            break;
        }
        
        for node_id in &current_level {
            remaining.remove(node_id);
            node_to_level.insert(node_id.clone(), levels.len());
        }
        
        levels.push(current_level);
    }
    
    levels
}

/// Automatically layout nodes hierarchically based on execution levels
pub fn auto_layout_graph_hierarchical(graph: &mut WorkflowGraph) {
    if graph.nodes.is_empty() {
        return;
    }
    
    // Calculate execution levels
    let levels = calculate_execution_levels(graph);
    
    // Build node ID to level mapping
    let mut node_to_level: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (level_idx, level_nodes) in levels.iter().enumerate() {
        for node_id in level_nodes {
            node_to_level.insert(node_id.clone(), level_idx);
        }
    }
    
    // Update execution levels on nodes
    for node in &mut graph.nodes {
        node.execution_level = node_to_level.get(&node.id).copied();
    }
    
    // Layout parameters
    let level_height = 150.0;
    let level_spacing = 20.0;
    let start_y = 50.0;
    let horizontal_spacing = 180.0;
    let start_x = 100.0;
    
    // Position nodes by level
    for (level_idx, level_nodes) in levels.iter().enumerate() {
        if level_nodes.is_empty() {
            continue;
        }
        
        let y = start_y + (level_idx as f32 * (level_height + level_spacing));
        let node_count = level_nodes.len();
        
        // Center nodes horizontally within the level
        let total_width = (node_count as f32 - 1.0) * horizontal_spacing;
        let level_start_x = start_x + (400.0 - total_width / 2.0).max(0.0);
        
        for (node_idx, node_id) in level_nodes.iter().enumerate() {
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == *node_id) {
                node.x = level_start_x + (node_idx as f32 * horizontal_spacing);
                node.y = y;
            }
        }
    }
    
    // Handle nodes not in any level (orphans) - place them at the end
    let max_level = levels.len();
    let orphan_y = start_y + (max_level as f32 * (level_height + level_spacing));
    let mut orphan_x = start_x;
    
    for node in &mut graph.nodes {
        if node.execution_level.is_none() {
            node.x = orphan_x;
            node.y = orphan_y;
            orphan_x += horizontal_spacing;
        }
    }
}

pub fn resolve_workflows_dir() -> String {
    let candidates = [
        "workflows/",
        "./workflows/",
        "../workflows/",
        "../../workflows/",
    ];

    // First, try to find existing workflows directory
    for candidate in &candidates {
        let path = std::path::Path::new(candidate);
        if path.exists() && path.is_dir() {
            return candidate.to_string();
        }
    }

    // If none exist, create the default one
    let default_dir = "workflows/";
    let path = std::path::Path::new(default_dir);
    if let Err(e) = std::fs::create_dir_all(path) {
        eprintln!("Warning: Could not create workflows directory: {}", e);
    }
    
    default_dir.to_string()
}

pub fn list_available_workflows() -> Vec<String> {
    let workflows_dir = resolve_workflows_dir();
    let mut workflows = Vec::new();
    
    let dir_path = std::path::Path::new(&workflows_dir);
    
    // Ensure directory exists
    if !dir_path.exists() {
        if let Err(e) = std::fs::create_dir_all(dir_path) {
            eprintln!("Warning: Could not create workflows directory: {}", e);
            return workflows;
        }
    }
    
    if let Ok(entries) = std::fs::read_dir(&workflows_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "yaml" || ext == "yml" {
                        if let Some(file_name) = path.file_name() {
                            if let Some(name_str) = file_name.to_str() {
                                workflows.push(name_str.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    
    workflows.sort();
    workflows
}

fn resolve_workflow_path(path: &str) -> Result<String, String> {
    let path_str = path.trim();
    if path_str.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    let input_path = std::path::Path::new(path_str);
    
    // If path is already absolute, validate it's a file
    if input_path.is_absolute() {
        if input_path.is_dir() {
            return Err(format!("Path is a directory: {}", path_str));
        }
        if !input_path.exists() {
            return Err(format!("File not found: {}", path_str));
        }
        return Ok(path_str.to_string());
    }

    // If path already contains a directory separator, try to use it as-is first
    if path_str.contains('/') || path_str.contains('\\') {
        if input_path.exists() {
            if input_path.is_dir() {
                return Err(format!("Path is a directory: {}", path_str));
            }
            return Ok(path_str.to_string());
        }
    }

    // Otherwise, automatically resolve relative to workflows directory
    let workflows_dir = resolve_workflows_dir();
    let workflows_path = std::path::Path::new(&workflows_dir);
    
    // Ensure workflows directory exists
    if !workflows_path.exists() {
        if let Err(e) = std::fs::create_dir_all(workflows_path) {
            return Err(format!("Could not create workflows directory: {}", e));
        }
    }
    
    let resolved = workflows_path.join(path_str);
    
    // Validate resolved path
    if !resolved.exists() {
        // Try adding .yaml extension if not present
        if !path_str.ends_with(".yaml") && !path_str.ends_with(".yml") {
            let with_yaml = resolved.with_extension("yaml");
            if with_yaml.exists() && !with_yaml.is_dir() {
                return Ok(with_yaml.to_string_lossy().to_string());
            }
        }
        return Err(format!("File not found: {} (looked in: {})", path_str, resolved.display()));
    }
    
    if resolved.is_dir() {
        return Err(format!("Path is a directory: {} (resolved: {})", path_str, resolved.display()));
    }

    Ok(resolved.to_string_lossy().to_string())
}

pub fn save_workflow_yaml(graph: &WorkflowGraph, filename: &str) -> Result<(), String> {
    // Build dependency info from edges
    let mut incoming: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for e in &graph.edges {
        incoming
            .entry(e.to.clone())
            .or_default()
            .push(e.from.clone());
    }

    let workflow = lao_orchestrator_core::Workflow {
        workflow: filename.trim_end_matches(".yaml").to_string(),
        steps: graph
            .nodes
            .iter()
            .map(|node| {
                let deps = incoming.get(&node.id).cloned().unwrap_or_default();
                
                // Use primary_input if set, otherwise use first predecessor
                let input_from = node.primary_input.clone()
                    .or_else(|| deps.first().cloned())
                    .filter(|id| deps.contains(id)); // Ensure it's actually in the dependencies
                
                // All other predecessors are depends_on
                let depends_on: Vec<String> = deps
                    .iter()
                    .filter(|&dep_id| {
                        // Exclude the primary input from depends_on
                        input_from.as_ref().map(|pi| pi != dep_id).unwrap_or(true)
                    })
                    .cloned()
                    .collect();
                
                let depends_on = if depends_on.is_empty() {
                    None
                } else {
                    Some(depends_on)
                };

                lao_orchestrator_core::WorkflowStep {
                    run: node.run.clone(),
                    params: serde_yaml::Value::Null, // Could be enhanced to support parameters
                    retries: None,
                    retry_delay: None,
                    cache_key: None,
                    input_from,
                    depends_on,
                    condition: None,
                    on_success: None,
                    on_failure: None,
                }
            })
            .collect(),
    };

    let yaml_content = serde_yaml::to_string(&workflow).map_err(|e| e.to_string())?;

    // Resolve workflows directory and ensure it exists
    let workflows_dir = resolve_workflows_dir();
    std::fs::create_dir_all(&workflows_dir).map_err(|e| e.to_string())?;

    let path = std::path::Path::new(&workflows_dir).join(filename);
    std::fs::write(&path, yaml_content).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn export_workflow_yaml(graph: &WorkflowGraph) -> Result<String, String> {
    let mut yaml = String::new();
    yaml.push_str("workflow: generated_workflow\n");
    yaml.push_str("steps:\n");

    // Create a map of node incoming edges (predecessors)
    let mut incoming: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for edge in &graph.edges {
        incoming
            .entry(edge.to.clone())
            .or_default()
            .push(edge.from.clone());
    }

    // Create a map of node ID to step index for proper step naming
    // Nodes are exported in their current order, so index = step number - 1
    let mut node_to_step: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        node_to_step.insert(node.id.clone(), index);
    }

    for (_step_index, node) in graph.nodes.iter().enumerate() {
        yaml.push_str(&format!("- run: {}\n", node.run));

        // Add input_from and depends_on if this node has predecessors
        if let Some(preds) = incoming.get(&node.id) {
            if !preds.is_empty() {
                // Use primary_input if set, otherwise use first predecessor
                let input_from_id = node.primary_input.as_ref()
                    .or_else(|| preds.first())
                    .filter(|id| preds.contains(id));
                
                if let Some(input_id) = input_from_id {
                    if let Some(&pred_step_idx) = node_to_step.get(input_id) {
                        yaml.push_str(&format!("  input_from: step{}\n", pred_step_idx + 1));
                    }
                }
                
                // All other predecessors are depends_on (excluding primary input)
                let step_deps: Vec<String> = preds
                    .iter()
                    .filter(|&dep_id| {
                        // Exclude the primary input from depends_on
                        input_from_id.map(|pi| pi != dep_id).unwrap_or(true)
                    })
                    .filter_map(|dep_id| node_to_step.get(dep_id))
                    .map(|&dep_index| format!("step{}", dep_index + 1))
                    .collect();
                if !step_deps.is_empty() {
                    yaml.push_str(&format!("  depends_on: [{}]\n", step_deps.join(", ")));
                }
            }
        }

        // Only add fields that have meaningful values
        if let Some(ref input_type) = node.input_type {
            yaml.push_str(&format!("  input_type: {}\n", input_type));
        }
        if let Some(ref output_type) = node.output_type {
            yaml.push_str(&format!("  output_type: {}\n", output_type));
        }
        if node.status != "pending" {
            yaml.push_str(&format!("  status: {}\n", node.status));
        }
    }

    Ok(yaml)
}

// Handle file upload for multi-modal input
#[allow(dead_code)]
pub fn handle_file_upload(file_path: &str, original_name: &str) -> Result<UploadedFile, String> {
    let metadata = std::fs::metadata(file_path).map_err(|e| e.to_string())?;
    let size = metadata.len() as usize;

    // Determine file type based on extension
    let file_type = match std::path::Path::new(original_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .as_deref()
    {
        Some("wav") | Some("mp3") | Some("flac") | Some("m4a") => "audio",
        Some("jpg") | Some("jpeg") | Some("png") | Some("gif") | Some("bmp") => "image",
        Some("mp4") | Some("avi") | Some("mov") | Some("mkv") | Some("webm") => "video",
        Some("txt") | Some("md") | Some("json") | Some("yaml") | Some("yml") => "text",
        _ => "binary",
    };

    // Create uploads directory if it doesn't exist
    let uploads_dir = "../uploads";
    std::fs::create_dir_all(uploads_dir).map_err(|e| e.to_string())?;

    // Copy file to uploads directory with timestamp
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let new_path = format!("{}/{}_{}", uploads_dir, timestamp, original_name);
    std::fs::copy(file_path, &new_path).map_err(|e| e.to_string())?;

    Ok(UploadedFile {
        name: original_name.to_string(),
        path: new_path,
        file_type: file_type.to_string(),
        size,
        upload_time: chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    })
}

// Get supported file types for upload
#[allow(dead_code)]
pub fn get_supported_file_types() -> Vec<&'static str> {
    vec![
        "audio/*", "image/*", "video/*", "text/*", ".wav", ".mp3", ".flac", ".m4a", ".jpg",
        ".jpeg", ".png", ".gif", ".bmp", ".mp4", ".avi", ".mov", ".mkv", ".webm", ".txt", ".md",
        ".json", ".yaml", ".yml", ".pdf",
    ]
}
