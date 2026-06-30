use crate::workflow_executor::WorkflowExecutor;
use crate::workflow_types::*;

pub fn run_workflow_yaml(path: &str) -> Result<Vec<StepLog>, String> {
    WorkflowExecutor::with_defaults().run(path, |_| {})
}

// Streaming runner with callback events
pub fn run_workflow_yaml_with_callback<F>(path: &str, on_event: F) -> Result<Vec<StepLog>, String>
where
    F: FnMut(StepEvent) + Send,
{
    WorkflowExecutor::with_defaults().run(path, on_event)
}
