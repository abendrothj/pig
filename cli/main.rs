mod coordinator_commands;
mod model_commands;
mod profiles;
mod worker_lifecycle;

use clap::{Parser, Subcommand};
use lao_orchestrator_core::{
    code_intelligence::{
        CachingProvider, CodeIntelligenceProvider, CodebaseMemoryCliProvider, GraphOperation,
    },
    cross_platform::PathUtils,
    load_workflow_yaml,
    plugins::PluginRegistry,
    scheduler::WorkflowScheduler,
    trust::{CapabilityClass, TrustPolicy},
    workflow_state::WorkflowSchedule,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct PromptPair {
    prompt: String,
    workflow: String,
}

fn normalize_yaml(yaml: &str) -> serde_yaml::Value {
    serde_yaml::from_str(yaml).unwrap_or(serde_yaml::Value::Null)
}

fn strip_code_fences(s: &str) -> String {
    s.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[derive(Parser)]
#[command(name = "lao")]
#[command(about = "Local AI Orchestrator CLI", long_about = None)]
struct Cli {
    /// Coordinator profile from [profiles.<name>] in lao.toml. Omit to preserve
    /// the existing embedded coordinator behavior.
    #[arg(long, global = true)]
    profile: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a workflow YAML file
    Run {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Validate a workflow YAML file (type & plugin availability)
    Validate { path: String },
    /// List available plugins
    PluginList,
    /// Scaffold a new workflow YAML template
    NewWorkflow {
        name: String,
        #[arg(long, help = "Output file path (default: workflows/<name>.yaml)")]
        output: Option<String>,
    },
    /// Generate and run a workflow from a prompt
    Prompt {
        prompt: String,
        #[arg(
            long,
            help = "Output file path (default: workflows/generated_from_prompt.yaml)"
        )]
        output: Option<String>,
    },
    /// Validate prompt-to-workflow generation using the prompt library
    ValidatePrompts {
        #[arg(
            long,
            default_value = "core/prompt_dispatcher/prompt/prompt_library.json"
        )]
        path: String,
        #[arg(long)]
        fail_fast: bool,
        #[arg(long)]
        verbose: bool,
    },
    /// List all saved workflows in the workflows/ directory
    ListWorkflows,
    /// View a workflow YAML file by name (from workflows/ directory)
    ViewWorkflow { name: String },
    /// Delete a workflow YAML file by name (from workflows/ directory)
    DeleteWorkflow { name: String },
    /// Explain a plugin's capabilities, schemas, and usage examples
    ExplainPlugin { name: String },
    /// Schedule a workflow to run at specified intervals
    Schedule {
        workflow_path: String,
        #[arg(
            long,
            help = "Cron-like expression (e.g., 'interval:60' for every 60 minutes)"
        )]
        cron: String,
        #[arg(long, help = "Maximum number of times to run (optional)")]
        max_runs: Option<u32>,
    },
    /// Unschedule a workflow
    Unschedule { workflow_id: String },
    /// List all scheduled workflows
    ListScheduled,
    /// Run all scheduled workflows that are currently due
    RunDue,
    /// Show workflow execution history and state
    Status {
        #[arg(help = "Workflow ID (optional - shows all if not specified)")]
        workflow_id: Option<String>,
    },
    /// Clean up old workflow states
    Cleanup {
        #[arg(
            long,
            default_value = "168",
            help = "Remove states older than this many hours"
        )]
        max_age_hours: u64,
    },
    /// List configured code intelligence providers
    ProviderList,
    /// Check code intelligence provider availability
    ProviderHealth,
    /// Run an allowlisted read-only code-graph query against the configured provider
    CodeGraphQuery {
        #[arg(
            help = "search_graph | search_code | trace_path | get_code_snippet | get_architecture | query_graph | index_status"
        )]
        operation: String,
        #[arg(
            default_value = "{}",
            help = "JSON arguments, e.g. '{\"project\":\"myrepo\"}'"
        )]
        args: String,
        #[arg(long, help = "Emit machine-readable JSON output")]
        json: bool,
    },
    /// Worker daemon control
    Worker {
        #[command(subcommand)]
        action: WorkerAction,
    },
    /// Inspect configured workers
    Workers {
        #[command(subcommand)]
        action: WorkersAction,
    },
    /// Model registry and generation
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Routing
    Route {
        #[command(subcommand)]
        action: RouteAction,
    },
    /// Job management
    Jobs {
        #[command(subcommand)]
        action: JobsAction,
    },
    /// Coordinator service control
    Coordinator {
        #[command(subcommand)]
        action: CoordinatorAction,
    },
}

#[derive(Subcommand)]
enum WorkerAction {
    /// Start the worker daemon (blocks until shutdown)
    Serve {
        #[arg(long, help = "Path to lao.toml (default: ./lao.toml)")]
        config: Option<String>,
    },
    /// Install the worker as a systemd service (Linux only, requires root)
    Install {
        #[arg(long, help = "Path to lao.toml (default: ./lao.toml)")]
        config: Option<String>,
    },
    /// Remove the systemd service, its config/token files, and the ACL grant
    Uninstall {
        #[arg(long, help = "Also remove the dedicated lao-worker service account")]
        purge_user: bool,
    },
    /// Start the installed systemd service
    Start,
    /// Stop the installed systemd service
    Stop,
    /// Restart the installed systemd service
    Restart,
    /// Show systemd + backend health status for the installed service
    Status,
    /// Show journald logs for the installed service
    Logs {
        #[arg(long, help = "Follow the log (like tail -f)")]
        follow: bool,
        #[arg(long, help = "Number of recent lines to show")]
        lines: Option<u32>,
    },
}

#[derive(Subcommand)]
enum WorkersAction {
    /// List configured workers and their health
    List {
        #[arg(long)]
        json: bool,
    },
    /// Inspect one worker's full capability snapshot
    Inspect {
        worker_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Health-check all configured workers (non-zero exit if any is unhealthy)
    Health {
        #[arg(long)]
        json: bool,
    },
    /// Show telemetry for one worker, or an aggregate across all configured workers
    /// if no worker id is given
    Metrics {
        worker_id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ModelsAction {
    /// List models declared in [models.entries]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Inspect one model entry
    Inspect {
        model_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Scan a directory for GGUF files (does not modify configuration)
    Discover {
        #[arg(long)]
        directory: String,
    },
    /// Load a model on a worker
    Load {
        model_id: String,
        #[arg(long)]
        worker: Option<String>,
    },
    /// Unload a model from a worker
    Unload {
        model_id: String,
        #[arg(long)]
        worker: Option<String>,
    },
    /// Run a direct generation request
    Generate {
        #[arg(long, help = "Model role, e.g. reasoning, coding")]
        role: Option<String>,
        #[arg(long, help = "Direct model id/alias, overrides --role resolution")]
        model: Option<String>,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        system: Option<String>,
        #[arg(long)]
        max_tokens: Option<u32>,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long, help = "Emit the full structured ModelResponse as JSON")]
        json: bool,
        #[arg(long, help = "Stream tokens as they are generated (interactive use)")]
        stream: bool,
        #[arg(long)]
        force_worker: Option<String>,
        #[arg(long)]
        force_cpu: bool,
    },
    /// Run a short benchmark prompt against a model and record the result
    Benchmark {
        model_id: String,
        #[arg(long)]
        worker: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RouteAction {
    /// Show which worker/model would be selected for a request, and why
    Explain {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum JobsAction {
    /// List jobs known to a worker
    List {
        #[arg(long)]
        worker: String,
        #[arg(long)]
        json: bool,
    },
    /// Inspect one job
    Inspect {
        job_id: String,
        #[arg(long)]
        worker: String,
        #[arg(long)]
        json: bool,
    },
    /// Cancel a running or queued job
    Cancel {
        job_id: String,
        #[arg(long)]
        worker: String,
    },
}

#[derive(Subcommand)]
enum CoordinatorAction {
    /// Start the coordinator as a persistent HTTP service (homelab mode)
    Serve {
        #[arg(long, help = "Path to lao.toml (default: ./lao.toml)")]
        config: Option<String>,
        #[arg(long, default_value = "0.0.0.0:3001", help = "Address to bind to")]
        bind: Option<String>,
        #[arg(long, help = "Env var name holding the bearer auth token")]
        auth_token_env: Option<String>,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let profile = model_commands::resolve_profile(cli.profile.as_deref());
    match cli.command {
        Commands::Run { path, dry_run } => {
            if dry_run {
                match load_workflow_yaml(&path) {
                    Ok(workflow) => {
                        let plugin_dir = PathUtils::plugin_dir();
                        let plugin_registry = PluginRegistry::dynamic_registry(
                            plugin_dir.to_str().unwrap_or("plugins"),
                        );
                        println!("[DRY RUN] Workflow: {}", workflow.workflow);
                        for (i, step) in workflow.steps.iter().enumerate() {
                            let plugin = plugin_registry.plugins.get(&step.run);
                            println!("Step {}: {}", i + 1, step.run);
                            match plugin {
                                Some(_p) => {
                                    println!("  [OK] Plugin '{}' loaded.", step.run);
                                }
                                None => {
                                    println!("  [ERROR] Plugin '{}' not found!", step.run);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[DRY RUN] Failed to load workflow: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                let mut executor = lao_orchestrator_core::WorkflowExecutor::with_defaults();
                if let Some(invoker) = model_commands::build_model_invoker() {
                    executor = executor.with_model_invoker(invoker);
                }
                match executor.run(&path, |_event| {}) {
                    Ok(results) => {
                        let has_errors = results.iter().any(|step| step.error.is_some());
                        if has_errors {
                            println!("Workflow completed with step errors:");
                        } else {
                            println!("Workflow executed successfully. Step outputs:");
                        }
                        for (i, output) in results.iter().enumerate() {
                            println!("Step {}: {:?}", i + 1, output);
                        }
                        if has_errors {
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("Workflow execution failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Validate { path } => match load_workflow_yaml(&path) {
            Ok(workflow) => {
                let plugin_dir = PathUtils::plugin_dir();
                let plugin_registry =
                    PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
                let dag = match lao_orchestrator_core::build_dag(&workflow.steps) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[ERROR] Failed to build DAG: {}", e);
                        std::process::exit(1);
                    }
                };
                let errors = lao_orchestrator_core::validate_workflow_types(&dag, &plugin_registry);
                let trust_result = TrustPolicy::load_default().validate_workflow(&workflow);
                if errors.is_empty() && trust_result.is_ok() {
                    println!("Validation passed: all steps and plugins available.");
                } else {
                    for (step, msg) in errors {
                        println!("Step {}: {}", step, msg);
                    }
                    if let Err(e) = trust_result {
                        println!("Trust policy: {}", e);
                    }
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Failed to load workflow: {}", e);
                std::process::exit(1);
            }
        },
        Commands::PluginList => {
            let plugin_dir = PathUtils::plugin_dir();
            let plugin_registry =
                PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
            println!("Available plugins:");
            for name in plugin_registry.plugins.keys() {
                println!("- {}", name);
            }
        }
        Commands::NewWorkflow { name, output } => {
            let path = output.unwrap_or_else(|| format!("workflows/{}.yaml", name));
            let template = format!(
                "workflow: \"{}\"\nsteps:\n  - run: WhisperPlugin\n    input: audio.wav\n    retries: 2\n    retry_delay: 1000\n    cache_key: \"whisper_{}\"\n  - run: SummarizerPlugin\n    input_from: step1\n    cache_key: \"summary_{}\"\n",
                name, name, name
            );
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!(
                        "[ERROR] Failed to create directory {}: {}",
                        parent.display(),
                        e
                    );
                    std::process::exit(1);
                }
            }
            if let Err(e) = std::fs::write(&path, template) {
                eprintln!("[ERROR] Failed to write workflow file {}: {}", path, e);
                std::process::exit(1);
            }
            println!("Scaffolded new workflow at {}", path);
        }
        Commands::Prompt { prompt, output } => {
            // Use the PromptDispatcherPlugin to generate a workflow YAML
            let plugin_dir = PathUtils::plugin_dir();
            let registry =
                PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
            let dispatcher = match registry.plugins.get("PromptDispatcherPlugin") {
                Some(d) => d,
                None => {
                    eprintln!("PromptDispatcherPlugin not found");
                    std::process::exit(1);
                }
            };
            let yaml = match dispatcher.run_with_text(&prompt) {
                result if result.is_success() => result.output.unwrap_or_default(),
                result => {
                    eprintln!("PromptDispatcherPlugin failed: {}", result.display_error());
                    std::process::exit(1);
                }
            };
            println!("Generated workflow:\n{}", yaml);
            let clean_yaml = strip_code_fences(&yaml);
            match serde_yaml::from_str::<lao_orchestrator_core::Workflow>(&clean_yaml) {
                Ok(workflow) => {
                    if let Err(e) = TrustPolicy::load_default().validate_workflow(&workflow) {
                        eprintln!(
                            "[TRUST WARNING] Generated workflow is not trusted by default: {}",
                            e
                        );
                    }
                    let out_path = output
                        .unwrap_or_else(|| "workflows/generated_from_prompt.yaml".to_string());
                    if let Some(parent) = std::path::Path::new(&out_path).parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            eprintln!(
                                "[ERROR] Failed to create directory {}: {}",
                                parent.display(),
                                e
                            );
                            std::process::exit(1);
                        }
                    }
                    if let Err(e) = std::fs::write(&out_path, &clean_yaml) {
                        eprintln!("[ERROR] Failed to write workflow file {}: {}", out_path, e);
                        std::process::exit(1);
                    }
                    println!("Workflow saved to {}", out_path);
                }
                Err(e) => {
                    eprintln!("Failed to parse generated workflow YAML: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::ValidatePrompts {
            path,
            fail_fast,
            verbose,
        } => {
            // Load prompt pairs from the prompt library JSON
            let prompt_pairs: Vec<PromptPair> = {
                let data = match std::fs::read_to_string(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("Failed to read prompt library: {}", e);
                        std::process::exit(1);
                    }
                };
                match serde_json::from_str(&data) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to parse prompt library JSON: {}", e);
                        std::process::exit(1);
                    }
                }
            };
            let plugin_dir = PathUtils::plugin_dir();
            let registry =
                PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
            let dispatcher = match registry.plugins.get("PromptDispatcherPlugin") {
                Some(d) => d,
                None => {
                    eprintln!("PromptDispatcherPlugin not found");
                    std::process::exit(1);
                }
            };
            let mut failures = 0;
            for (i, pair) in prompt_pairs.iter().enumerate() {
                let generated = match dispatcher.run_with_text(&pair.prompt) {
                    result if result.is_success() => result.output.unwrap_or_default(),
                    result => {
                        eprintln!("Prompt {} failed: {}", i + 1, result.display_error());
                        failures += 1;
                        continue;
                    }
                };
                let expected = normalize_yaml(&pair.workflow);
                let actual = normalize_yaml(&generated);
                let pass = expected == actual;
                if !pass {
                    failures += 1;
                    println!(
                        "[FAIL] Prompt {}: {}\nExpected:\n{}\nActual:\n{}\n",
                        i + 1,
                        pair.prompt,
                        pair.workflow,
                        generated
                    );
                    if fail_fast {
                        println!("Fail-fast enabled. Stopping at first failure.");
                        std::process::exit(1);
                    }
                } else if verbose {
                    println!("[PASS] Prompt {}: {}", i + 1, pair.prompt);
                }
            }
            if failures == 0 {
                println!("All prompts passed validation!");
            } else {
                println!("{} prompts failed validation.", failures);
                std::process::exit(1);
            }
        }
        Commands::ListWorkflows => {
            let dir = std::path::Path::new("workflows");
            match std::fs::read_dir(dir) {
                Ok(entries) => {
                    println!("Available workflows:");
                    let mut found = false;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(ext) = path.extension() {
                            if ext == "yaml" || ext == "yml" {
                                match path.file_name() {
                                    Some(name) => println!("- {}", name.to_string_lossy()),
                                    None => println!("- [unknown file name]"),
                                }
                                found = true;
                            }
                        }
                    }
                    if !found {
                        println!("[INFO] No workflow YAML files found in workflows/ directory.");
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to read workflows directory: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::ViewWorkflow { name } => {
            let path = format!("workflows/{}.yaml", name);
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    println!("Workflow {}:\n{}", name, contents);
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to read workflow file {}: {}", path, e);
                    std::process::exit(1);
                }
            }
        }
        Commands::DeleteWorkflow { name } => {
            let path = format!("workflows/{}.yaml", name);
            match std::fs::remove_file(&path) {
                Ok(_) => {
                    println!("Deleted workflow file {}", path);
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to delete workflow file {}: {}", path, e);
                    std::process::exit(1);
                }
            }
        }
        Commands::ExplainPlugin { name } => {
            use std::fs;
            use std::path::Path;
            let plugin_dir = format!("plugins/{}Plugin", name);
            let yaml_path = Path::new(&plugin_dir).join("plugin.yaml");
            let yaml_str = match fs::read_to_string(&yaml_path) {
                Ok(s) => s,
                Err(_) => {
                    eprintln!(
                        "[ERROR] plugin.yaml not found for plugin '{}'. Looked in {}",
                        name,
                        yaml_path.display()
                    );
                    std::process::exit(1);
                }
            };
            let manifest: serde_yaml::Value = match serde_yaml::from_str(&yaml_str) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[ERROR] Failed to parse plugin.yaml: {}", e);
                    std::process::exit(1);
                }
            };
            println!("\n--- Plugin: {} ---", name);
            if let Some(desc) = manifest.get("description").and_then(|v| v.as_str()) {
                println!("Description: {}", desc);
            }
            if let Some(tags) = manifest.get("tags") {
                println!("Tags: {:?}", tags);
            }
            if let Some(input) = manifest.get("input") {
                println!(
                    "Input Schema: {}",
                    serde_yaml::to_string(input).unwrap_or_default().trim()
                );
            }
            if let Some(output) = manifest.get("output") {
                println!(
                    "Output Schema: {}",
                    serde_yaml::to_string(output).unwrap_or_default().trim()
                );
            }
            if let Some(examples) = manifest.get("example_prompts") {
                println!("Example Prompts:");
                if let Some(arr) = examples.as_sequence() {
                    for ex in arr {
                        println!("  - {:?}", ex);
                    }
                } else {
                    println!("  - {:?}", examples);
                }
            }
        }
        Commands::Schedule {
            workflow_path,
            cron,
            max_runs,
        } => {
            let workflow_id = format!(
                "scheduled_{}",
                &uuid::Uuid::new_v4().to_string().replace("-", "")[..8]
            );

            // Validate workflow exists
            if !std::path::Path::new(&workflow_path).exists() {
                eprintln!("[ERROR] Workflow file not found: {}", workflow_path);
                std::process::exit(1);
            }

            let schedule = WorkflowSchedule {
                cron_expression: Some(cron.clone()),
                next_run: None,
                enabled: true,
                max_runs,
                run_count: 0,
            };

            let mut scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            match scheduler.schedule_workflow(workflow_id.clone(), workflow_path.clone(), schedule)
            {
                Ok(_) => {
                    println!(
                        "✓ Scheduled workflow '{}' with ID: {}",
                        workflow_path, workflow_id
                    );
                    println!("  Cron expression: {}", cron);
                    if let Some(max) = max_runs {
                        println!("  Max runs: {}", max);
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to schedule workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Unschedule { workflow_id } => {
            let mut scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            match scheduler.unschedule_workflow(&workflow_id) {
                Ok(_) => println!("✓ Unscheduled workflow: {}", workflow_id),
                Err(e) => {
                    eprintln!("[ERROR] Failed to unschedule workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::ListScheduled => {
            let scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            let scheduled = scheduler.list_scheduled_workflows();
            if scheduled.is_empty() {
                println!("No scheduled workflows found.");
            } else {
                println!("Scheduled workflows:");
                for (id, workflow) in scheduled {
                    println!("  {} - {}", id, workflow.workflow_path);
                    println!("    Schedule: {:?}", workflow.schedule.cron_expression);
                    println!("    Next run: {:?}", workflow.next_run);
                    println!("    Enabled: {}", workflow.schedule.enabled);
                    if let Some(last) = workflow.last_run {
                        println!("    Last run: {:?}", last);
                    }
                    println!("    Run count: {}", workflow.schedule.run_count);
                    println!();
                }
            }
        }
        Commands::RunDue => {
            let mut scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            let results = match scheduler.run_due_workflows_guarded() {
                Ok(results) => results,
                Err(e) => {
                    eprintln!("[ERROR] {}", e);
                    std::process::exit(1);
                }
            };
            if results.is_empty() {
                println!("No scheduled workflows are due.");
            } else {
                let mut failures = 0;
                for (id, result) in results {
                    match result {
                        Ok(logs) => println!("✓ Ran {} ({} steps)", id, logs.len()),
                        Err(e) => {
                            failures += 1;
                            eprintln!("[ERROR] {} failed: {}", id, e);
                        }
                    }
                }
                if failures > 0 {
                    std::process::exit(1);
                }
            }
        }
        Commands::Status { workflow_id } => {
            let scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            if let Some(id) = workflow_id {
                match scheduler.get_workflow_history(&id) {
                    Ok(Some(state)) => {
                        println!("Workflow: {} ({})", state.workflow_name, state.workflow_id);
                        println!("Status: {:?}", state.status);
                        println!("Created: {:?}", state.created_at);
                        if let Some(started) = state.started_at {
                            println!("Started: {:?}", started);
                        }
                        if let Some(completed) = state.completed_at {
                            println!("Completed: {:?}", completed);
                        }
                        println!(
                            "Progress: {}/{} steps",
                            state.current_step, state.total_steps
                        );
                        if let Some(error) = &state.error_message {
                            println!("Error: {}", error);
                        }
                    }
                    Ok(None) => println!("Workflow not found: {}", id),
                    Err(e) => eprintln!("[ERROR] Failed to load workflow state: {}", e),
                }
            } else {
                let states = scheduler.list_workflow_states();
                if states.is_empty() {
                    println!("No workflow states found.");
                } else {
                    println!("All workflow states:");
                    for state in states {
                        println!(
                            "  {} - {} ({:?})",
                            state.workflow_id, state.workflow_name, state.status
                        );
                    }
                }
            }
        }
        Commands::Cleanup { max_age_hours } => {
            let mut scheduler = match WorkflowScheduler::new("workflow_states") {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to initialize scheduler: {}", e);
                    std::process::exit(1);
                }
            };

            match scheduler.cleanup_old_states(max_age_hours) {
                Ok(count) => println!("✓ Cleaned up {} old workflow states", count),
                Err(e) => eprintln!("[ERROR] Failed to cleanup states: {}", e),
            }
        }
        Commands::ProviderList => {
            let repo_root =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            println!("Configured code intelligence providers:");
            match CodebaseMemoryCliProvider::discover(repo_root) {
                Ok(provider) => {
                    let meta = provider.metadata();
                    println!(
                        "- {} ({})",
                        meta.name,
                        meta.version.as_deref().unwrap_or("unknown version")
                    );
                }
                Err(e) => {
                    println!("- codebase-memory-mcp (not available: {})", e);
                }
            }
        }
        Commands::ProviderHealth => {
            let repo_root =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let provider = match CodebaseMemoryCliProvider::discover(repo_root) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[ERROR] {}", e);
                    std::process::exit(1);
                }
            };
            match provider.health() {
                Ok(health) => {
                    println!(
                        "codebase-memory-mcp: {}",
                        if health.available {
                            "available"
                        } else {
                            "unavailable"
                        }
                    );
                    println!("  {}", health.detail);
                    if !health.available {
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] health check failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::CodeGraphQuery {
            operation,
            args,
            json,
        } => {
            let Some(op) = GraphOperation::from_tool_name(&operation) else {
                eprintln!(
                    "[ERROR] unsupported code-graph operation '{}'; supported: {}",
                    operation,
                    GraphOperation::ALL
                        .iter()
                        .map(|o| o.tool_name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(1);
            };

            if !TrustPolicy::load_default().allows_class(CapabilityClass::CodeGraphRead) {
                eprintln!(
                    "[ERROR] code-graph access denied: set trust.allow_code_graph_read in lao.toml"
                );
                std::process::exit(1);
            }

            let parsed_args: serde_json::Value = match serde_json::from_str(&args) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[ERROR] failed to parse --args as JSON: {}", e);
                    std::process::exit(1);
                }
            };

            let repo_root =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let cache_dir = repo_root
                .join(std::env::var("LAO_CACHE_DIR").unwrap_or_else(|_| "cache".to_string()))
                .join("code_graph");

            let provider = match CodebaseMemoryCliProvider::discover(repo_root.clone()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[ERROR] {}", e);
                    std::process::exit(1);
                }
            };
            let provider = CachingProvider::new(provider, repo_root, cache_dir);

            match provider.query(op, parsed_args) {
                Ok(artifact) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&artifact).unwrap_or_default()
                        );
                    } else {
                        println!(
                            "Provider: {} ({})",
                            artifact.provider,
                            artifact
                                .provider_version
                                .as_deref()
                                .unwrap_or("unknown version")
                        );
                        println!("Repo: {}", artifact.repo_root.display());
                        println!(
                            "Revision: {}",
                            artifact.git_revision.as_deref().unwrap_or("unknown")
                        );
                        println!("Dirty: {}", artifact.dirty);
                        println!("Operation: {}", artifact.operation);
                        println!(
                            "Result:\n{}",
                            serde_json::to_string_pretty(&artifact.payload).unwrap_or_default()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Worker { action } => match action {
            WorkerAction::Serve { config } => model_commands::worker_serve(config),
            WorkerAction::Install { config } => worker_lifecycle::worker_install(config),
            WorkerAction::Uninstall { purge_user } => {
                worker_lifecycle::worker_uninstall(purge_user)
            }
            WorkerAction::Start => worker_lifecycle::worker_start(),
            WorkerAction::Stop => worker_lifecycle::worker_stop(),
            WorkerAction::Restart => worker_lifecycle::worker_restart(),
            WorkerAction::Status => worker_lifecycle::worker_status(),
            WorkerAction::Logs { follow, lines } => worker_lifecycle::worker_logs(follow, lines),
        },
        Commands::Workers { action } => match action {
            WorkersAction::List { json } => model_commands::workers_list(json, &profile),
            WorkersAction::Inspect { worker_id, json } => {
                model_commands::workers_inspect(worker_id, json, &profile)
            }
            WorkersAction::Health { json } => model_commands::workers_health(json, &profile),
            WorkersAction::Metrics { worker_id, json } => {
                model_commands::workers_metrics(worker_id, json, &profile)
            }
        },
        Commands::Models { action } => match action {
            ModelsAction::List { json } => model_commands::models_list(json),
            ModelsAction::Inspect { model_id, json } => {
                model_commands::models_inspect(model_id, json)
            }
            ModelsAction::Discover { directory } => model_commands::models_discover(directory),
            ModelsAction::Load { model_id, worker } => {
                model_commands::models_load(model_id, worker)
            }
            ModelsAction::Unload { model_id, worker } => {
                model_commands::models_unload(model_id, worker)
            }
            ModelsAction::Generate {
                role,
                model,
                prompt,
                system,
                max_tokens,
                temperature,
                json,
                stream,
                force_worker,
                force_cpu,
            } => model_commands::models_generate(
                role,
                model,
                prompt,
                system,
                max_tokens,
                temperature,
                json,
                stream,
                force_worker,
                force_cpu,
            ),
            ModelsAction::Benchmark {
                model_id,
                worker,
                json,
            } => model_commands::models_benchmark(model_id, worker, json),
        },
        Commands::Route { action } => match action {
            RouteAction::Explain { role, model, json } => {
                model_commands::route_explain(role, model, json, &profile)
            }
        },
        Commands::Jobs { action } => match action {
            JobsAction::List { worker, json } => model_commands::jobs_list(worker, json, &profile),
            JobsAction::Inspect {
                job_id,
                worker,
                json,
            } => model_commands::jobs_inspect(job_id, worker, json, &profile),
            JobsAction::Cancel { job_id, worker } => {
                model_commands::jobs_cancel(job_id, worker, &profile)
            }
        },
        Commands::Coordinator { action } => match action {
            CoordinatorAction::Serve {
                config,
                bind,
                auth_token_env,
            } => coordinator_commands::coordinator_serve(config, bind, auth_token_env),
        },
    }
}
