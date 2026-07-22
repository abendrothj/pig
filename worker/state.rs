use crate::config::WorkerConfig;
use crate::hardware::HardwareInfo;
use crate::job::WorkerRuntime;
use lao_orchestrator_core::model::ModelRegistry;
use std::sync::Arc;
use std::time::Instant;

pub struct AppState {
    pub config: WorkerConfig,
    pub runtime: Arc<WorkerRuntime>,
    pub registry: ModelRegistry,
    pub hardware: HardwareInfo,
    pub started_at: Instant,
    pub auth_token: Option<String>,
    pub backend_name: String,
}
