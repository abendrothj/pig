use crate::config::WorkerConfig;
use crate::hardware::HardwareInfo;
use crate::job::WorkerRuntime;
use lao_orchestrator_core::model::ModelRegistry;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct AppState {
    pub config: WorkerConfig,
    pub runtime: Arc<WorkerRuntime>,
    pub registry: ModelRegistry,
    pub hardware: HardwareInfo,
    pub started_at: Instant,
    pub auth_token: Option<String>,
    pub backend_name: String,
    /// Short-TTL cache for the live (re-probed) hardware reading used by
    /// `/v1/metrics` - `hardware` above is the one-time startup snapshot used by
    /// `/v1/capabilities`; this is refreshed periodically instead of shelling out to
    /// `nvidia-smi`/reading `/proc` on every metrics request. `None` until the first
    /// metrics request.
    pub hardware_cache: Mutex<Option<(Instant, HardwareInfo)>>,
}
