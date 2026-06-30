use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowState {
    pub workflow_id: String,
    pub workflow_name: String,
    #[serde(default)]
    pub workflow_path: Option<String>,
    pub status: WorkflowStatus,
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub completed_at: Option<SystemTime>,
    pub current_step: usize,
    pub total_steps: usize,
    pub step_results: Vec<StepResult>,
    pub outputs: HashMap<String, String>,
    pub error_message: Option<String>,
    pub retry_count: u32,
    pub schedule: Option<WorkflowSchedule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Scheduled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepResult {
    pub step_id: String,
    pub plugin_name: String,
    pub status: StepStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub duration_ms: Option<u64>,
    pub retry_count: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum StepStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowSchedule {
    pub cron_expression: Option<String>,
    pub next_run: Option<SystemTime>,
    pub enabled: bool,
    pub max_runs: Option<u32>,
    pub run_count: u32,
}

impl WorkflowState {
    pub fn new(workflow_id: String, workflow_name: String, total_steps: usize) -> Self {
        Self {
            workflow_id,
            workflow_name,
            workflow_path: None,
            status: WorkflowStatus::Pending,
            created_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
            current_step: 0,
            total_steps,
            step_results: Vec::new(),
            outputs: HashMap::new(),
            error_message: None,
            retry_count: 0,
            schedule: None,
        }
    }

    pub fn start(&mut self) {
        self.status = WorkflowStatus::Running;
        self.started_at = Some(SystemTime::now());
    }

    pub fn complete(&mut self) {
        self.status = WorkflowStatus::Completed;
        self.completed_at = Some(SystemTime::now());
    }

    pub fn fail(&mut self, error: String) {
        self.status = WorkflowStatus::Failed;
        self.completed_at = Some(SystemTime::now());
        self.error_message = Some(error);
    }

    pub fn add_step_result(&mut self, result: StepResult) {
        self.step_results.push(result);
        self.current_step = self.step_results.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_state_new() {
        let state = WorkflowState::new("wf-1".to_string(), "Test".to_string(), 3);
        assert_eq!(state.workflow_id, "wf-1");
        assert_eq!(state.workflow_name, "Test");
        assert_eq!(state.total_steps, 3);
        assert_eq!(state.current_step, 0);
        assert!(matches!(state.status, WorkflowStatus::Pending));
        assert!(state.started_at.is_none());
        assert!(state.completed_at.is_none());
        assert!(state.error_message.is_none());
    }

    #[test]
    fn test_workflow_state_start() {
        let mut state = WorkflowState::new("wf-1".to_string(), "Test".to_string(), 3);
        state.start();
        assert!(matches!(state.status, WorkflowStatus::Running));
        assert!(state.started_at.is_some());
    }

    #[test]
    fn test_workflow_state_complete() {
        let mut state = WorkflowState::new("wf-1".to_string(), "Test".to_string(), 3);
        state.start();
        state.complete();
        assert!(matches!(state.status, WorkflowStatus::Completed));
        assert!(state.completed_at.is_some());
    }

    #[test]
    fn test_workflow_state_fail() {
        let mut state = WorkflowState::new("wf-1".to_string(), "Test".to_string(), 3);
        state.start();
        state.fail("something broke".to_string());
        assert!(matches!(state.status, WorkflowStatus::Failed));
        assert!(state.completed_at.is_some());
        assert_eq!(state.error_message.as_deref(), Some("something broke"));
    }

    #[test]
    fn test_add_step_result() {
        let mut state = WorkflowState::new("wf-1".to_string(), "Test".to_string(), 3);
        state.start();

        let result = StepResult {
            step_id: "step-1".to_string(),
            plugin_name: "EchoPlugin".to_string(),
            status: StepStatus::Success,
            output: Some("hello".to_string()),
            error: None,
            started_at: SystemTime::now(),
            completed_at: Some(SystemTime::now()),
            duration_ms: Some(42),
            retry_count: 0,
        };

        state.add_step_result(result);
        assert_eq!(state.current_step, 1);
        assert_eq!(state.step_results.len(), 1);
        assert_eq!(state.step_results[0].step_id, "step-1");
    }

    #[test]
    fn test_workflow_state_serialization_roundtrip() {
        let mut state = WorkflowState::new("wf-rt".to_string(), "Roundtrip".to_string(), 2);
        state.start();
        state.outputs.insert("key".to_string(), "value".to_string());

        let json = serde_json::to_string(&state).expect("serialize");
        let deserialized: WorkflowState = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.workflow_id, "wf-rt");
        assert_eq!(deserialized.workflow_name, "Roundtrip");
        assert_eq!(deserialized.total_steps, 2);
        assert_eq!(deserialized.outputs.get("key").unwrap(), "value");
    }

    #[test]
    fn test_workflow_schedule_serialization() {
        let schedule = WorkflowSchedule {
            cron_expression: Some("interval:30".to_string()),
            next_run: Some(SystemTime::now()),
            enabled: true,
            max_runs: Some(10),
            run_count: 3,
        };

        let json = serde_json::to_string(&schedule).expect("serialize");
        let deserialized: WorkflowSchedule = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.cron_expression.as_deref(), Some("interval:30"));
        assert!(deserialized.enabled);
        assert_eq!(deserialized.max_runs, Some(10));
        assert_eq!(deserialized.run_count, 3);
    }
}
