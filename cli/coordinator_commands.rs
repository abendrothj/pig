//! Handlers for `lao coordinator` — run the coordinator as a persistent HTTP service.
//! Reads the same lao.toml that the rest of the CLI uses.

use lao_orchestrator_core::model::ModelRegistry;
use lao_worker::coordinator::{Coordinator, WorkersConfig};
use lao_worker::coordinator_server::CoordinatorServerState;
use std::sync::Arc;

fn config_text(config: Option<&str>) -> String {
    let candidates: Vec<&str> = if let Some(p) = config {
        vec![p]
    } else if let Ok(p) = std::env::var("LAO_CONFIG") {
        return std::fs::read_to_string(p).unwrap_or_default();
    } else {
        vec!["lao.toml", "config/lao.toml"]
    };
    candidates
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn coordinator_serve(
    config: Option<String>,
    bind: Option<String>,
    auth_token_env: Option<String>,
) {
    let text = config_text(config.as_deref());

    let workers = WorkersConfig::from_toml_str(&text)
        .map(|c| c.workers)
        .unwrap_or_default();
    if workers.is_empty() {
        eprintln!("[ERROR] no [[workers]] configured in lao.toml (or LAO_CONFIG)");
        std::process::exit(1);
    }

    let registry = ModelRegistry::from_toml_str(&text).unwrap_or_default();
    let coordinator = Arc::new(Coordinator::new(workers, registry));

    let auth_token = auth_token_env.and_then(|var| {
        std::env::var(&var).ok().or_else(|| {
            eprintln!(
                "[WARN] --auth-token-env '{}' is not set; coordinator will be unauthenticated",
                var
            );
            None
        })
    });

    let bind_addr = bind.unwrap_or_else(|| "0.0.0.0:3001".to_string());

    let state = Arc::new(CoordinatorServerState::new(coordinator, auth_token));

    println!("Starting coordinator on {}", bind_addr);
    let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
    rt.block_on(async move {
        if let Err(e) = lao_worker::coordinator_server::serve(state, &bind_addr).await {
            eprintln!("[ERROR] coordinator server failed: {}", e);
            std::process::exit(1);
        }
    });
}
