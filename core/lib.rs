// --- LAO Workflow Engine ---

// Platform & infrastructure modules
pub mod apple_silicon;
pub mod core_scheduler;
pub mod cross_platform;
pub mod macos_integrations;
pub mod mps_shaders;
pub mod plugin_dev_tools;
pub mod plugin_manager;
pub mod plugins;
pub mod power_management;
pub mod scheduler;
pub mod state_manager;
pub mod unified_memory;
pub mod workflow_state;

// Workflow engine modules
pub mod workflow_types;
pub mod workflow_dag;
pub mod workflow_helpers;
pub mod workflow_exec;
pub mod workflow_parallel;

// Re-export public API to preserve backward compatibility
pub use workflow_types::*;
pub use workflow_dag::*;
pub use workflow_exec::*;
pub use workflow_parallel::run_workflow_yaml_parallel_with_callback;
pub use workflow_helpers::{evaluate_condition, should_execute_step};
