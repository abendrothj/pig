mod coordinator_commands;
mod model_commands;
mod profiles;
mod worker_lifecycle;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pig")]
#[command(about = "Local AI Orchestrator CLI", long_about = None)]
struct Cli {
    /// Coordinator profile from [profiles.<name>] in pig.toml. Omit to preserve
    /// the existing embedded coordinator behavior.
    #[arg(long, global = true)]
    profile: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
        #[arg(long, help = "Path to pig.toml (default: ./pig.toml)")]
        config: Option<String>,
    },
    /// Install the worker as a systemd service (Linux only, requires root)
    Install {
        #[arg(long, help = "Path to pig.toml (default: ./pig.toml)")]
        config: Option<String>,
    },
    /// Remove the systemd service, its config/token files, and the ACL grant
    Uninstall {
        #[arg(long, help = "Also remove the dedicated pig-worker service account")]
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
        #[arg(long, help = "Path to pig.toml (default: ./pig.toml)")]
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
