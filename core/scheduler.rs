use crate::state_manager::WorkflowStateManager;
use crate::workflow_exec::run_workflow_yaml;
use crate::workflow_state::{WorkflowSchedule, WorkflowState, WorkflowStatus};
use crate::workflow_types::StepLog;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long a `run-due` lock can exist before it is considered stale and reclaimed.
const RUN_DUE_LOCK_STALE_SECS: u64 = 3600;

/// Outcome of running each due workflow: its id paired with the run result.
pub type RunDueOutcome = Vec<(String, Result<Vec<StepLog>, String>)>;

pub struct WorkflowScheduler {
    state_manager: WorkflowStateManager,
    scheduled_workflows: HashMap<String, ScheduledWorkflow>,
    state_dir: PathBuf,
}

/// Advisory cross-process lock that prevents overlapping `run-due` invocations
/// (e.g. two cron ticks) from double-running scheduled workflows. The lock is a file
/// in the state directory created atomically; it is removed on drop.
pub struct RunDueLock {
    path: PathBuf,
}

impl RunDueLock {
    pub fn acquire(state_dir: &Path) -> Result<Option<Self>, String> {
        use std::fs::OpenOptions;
        use std::io::ErrorKind;

        let path = state_dir.join(".run-due.lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "pid={}", std::process::id());
                Ok(Some(Self { path }))
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(|modified| {
                        modified
                            .elapsed()
                            .map(|age| age > Duration::from_secs(RUN_DUE_LOCK_STALE_SECS))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if stale {
                    tracing::warn!("Reclaiming stale run-due lock at {}", path.display());
                    let _ = fs::remove_file(&path);
                    return Self::acquire(state_dir);
                }
                Ok(None)
            }
            Err(e) => Err(format!("failed to acquire run-due lock: {}", e)),
        }
    }
}

impl Drop for RunDueLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledWorkflow {
    pub workflow_path: String,
    pub schedule: WorkflowSchedule,
    pub last_run: Option<SystemTime>,
    pub next_run: SystemTime,
}

impl WorkflowScheduler {
    pub fn new(state_dir: &str) -> std::io::Result<Self> {
        let state_manager = WorkflowStateManager::new(state_dir)?;
        let mut scheduler = Self {
            state_manager,
            scheduled_workflows: HashMap::new(),
            state_dir: PathBuf::from(state_dir),
        };
        scheduler.reload_scheduled_workflows();
        Ok(scheduler)
    }

    pub fn schedule_workflow(
        &mut self,
        workflow_id: String,
        workflow_path: String,
        schedule: WorkflowSchedule,
    ) -> Result<(), String> {
        let next_run = self.calculate_next_run(&schedule)?;

        let mut schedule = schedule.clone();
        schedule.next_run = Some(next_run);

        let scheduled = ScheduledWorkflow {
            workflow_path: workflow_path.clone(),
            schedule: schedule.clone(),
            last_run: None,
            next_run,
        };

        self.scheduled_workflows
            .insert(workflow_id.clone(), scheduled);

        // Create a scheduled workflow state
        let mut state = WorkflowState::new(workflow_id, "Scheduled Workflow".to_string(), 0);
        state.status = WorkflowStatus::Scheduled;
        state.workflow_path = Some(workflow_path);
        state.schedule = Some(schedule);

        self.state_manager
            .save_state(&state)
            .map_err(|e| format!("Failed to save scheduled workflow state: {}", e))?;

        Ok(())
    }

    fn reload_scheduled_workflows(&mut self) {
        let states: Vec<_> = self
            .state_manager
            .list_scheduled_workflows()
            .into_iter()
            .cloned()
            .collect();

        for state in states {
            let Some(workflow_path) = state.workflow_path.clone() else {
                continue;
            };
            let Some(schedule) = state.schedule.clone() else {
                continue;
            };
            if !schedule.enabled {
                continue;
            }

            let next_run = schedule.next_run.unwrap_or_else(|| {
                self.calculate_next_run(&schedule)
                    .unwrap_or(SystemTime::now())
            });
            self.scheduled_workflows.insert(
                state.workflow_id,
                ScheduledWorkflow {
                    workflow_path,
                    schedule,
                    last_run: state.completed_at,
                    next_run,
                },
            );
        }
    }

    pub fn unschedule_workflow(&mut self, workflow_id: &str) -> Result<(), String> {
        self.scheduled_workflows.remove(workflow_id);
        self.state_manager
            .delete_state(workflow_id)
            .map_err(|e| format!("Failed to delete workflow state: {}", e))?;
        Ok(())
    }

    pub fn get_due_workflows(&self) -> Vec<String> {
        let now = SystemTime::now();
        self.scheduled_workflows
            .iter()
            .filter(|(_, scheduled)| scheduled.next_run <= now && scheduled.schedule.enabled)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn update_workflow_run(&mut self, workflow_id: &str) -> Result<(), String> {
        // Extract schedule info to avoid borrowing conflicts
        let (schedule, max_runs) = {
            if let Some(scheduled) = self.scheduled_workflows.get(workflow_id) {
                (scheduled.schedule.clone(), scheduled.schedule.max_runs)
            } else {
                return Ok(());
            }
        };

        // Calculate next run time
        let next_run = self.calculate_next_run(&schedule)?;

        // Update the scheduled workflow
        if let Some(scheduled) = self.scheduled_workflows.get_mut(workflow_id) {
            scheduled.last_run = Some(SystemTime::now());
            scheduled.next_run = next_run;
            scheduled.schedule.run_count += 1;
            scheduled.schedule.next_run = Some(next_run);

            // Check if max runs reached
            if let Some(max_runs) = max_runs {
                if scheduled.schedule.run_count >= max_runs {
                    scheduled.schedule.enabled = false;
                }
            }

            if let Ok(Some(mut state)) = self.state_manager.load_state(workflow_id) {
                state.schedule = Some(scheduled.schedule.clone());
                state.completed_at = scheduled.last_run;
                self.state_manager
                    .save_state(&state)
                    .map_err(|e| format!("Failed to persist workflow run update: {}", e))?;
            }
        }
        Ok(())
    }

    /// Run due workflows under an advisory lock so overlapping invocations cannot
    /// double-run schedules. Returns `Err` if another `run-due` holds the lock.
    pub fn run_due_workflows_guarded(&mut self) -> Result<RunDueOutcome, String> {
        match RunDueLock::acquire(&self.state_dir)? {
            Some(_lock) => Ok(self.run_due_workflows()),
            None => Err("another run-due invocation is in progress".to_string()),
        }
    }

    pub fn run_due_workflows(&mut self) -> RunDueOutcome {
        let due = self.get_due_workflows();
        let mut results = Vec::new();

        for workflow_id in due {
            let Some(workflow_path) = self
                .scheduled_workflows
                .get(&workflow_id)
                .map(|scheduled| scheduled.workflow_path.clone())
            else {
                continue;
            };
            let result = run_workflow_yaml(&workflow_path);
            let update_result = self.update_workflow_run(&workflow_id);
            let combined = match (result, update_result) {
                (Ok(logs), Ok(())) => Ok(logs),
                (Err(run_err), Ok(())) => Err(run_err),
                (Ok(_), Err(update_err)) => Err(update_err),
                (Err(run_err), Err(update_err)) => Err(format!("{}; {}", run_err, update_err)),
            };
            results.push((workflow_id, combined));
        }

        results
    }

    pub fn list_scheduled_workflows(&self) -> Vec<(String, &ScheduledWorkflow)> {
        self.scheduled_workflows
            .iter()
            .map(|(id, scheduled)| (id.clone(), scheduled))
            .collect()
    }

    fn calculate_next_run(&self, schedule: &WorkflowSchedule) -> Result<SystemTime, String> {
        if let Some(cron_expr) = &schedule.cron_expression {
            // For now, implement simple interval parsing
            // In a full implementation, you'd use a cron parsing library
            self.parse_simple_cron(cron_expr)
        } else {
            // Default to 1 hour from now
            Ok(SystemTime::now() + Duration::from_secs(3600))
        }
    }

    fn parse_simple_cron(&self, cron_expr: &str) -> Result<SystemTime, String> {
        // Simple cron parser for common patterns
        // Format: "interval:minutes" or "daily:HH:MM" or "weekly:day:HH:MM"
        let parts: Vec<&str> = cron_expr.split(':').collect();

        match parts.as_slice() {
            ["interval", minutes_str] => {
                let minutes: u64 = minutes_str
                    .parse()
                    .map_err(|_| format!("Invalid interval minutes: {}", minutes_str))?;
                Ok(SystemTime::now() + Duration::from_secs(minutes * 60))
            }
            ["daily", hour_str, minute_str] => {
                let _hour: u32 = hour_str
                    .parse()
                    .map_err(|_| format!("Invalid hour: {}", hour_str))?;
                let _minute: u32 = minute_str
                    .parse()
                    .map_err(|_| format!("Invalid minute: {}", minute_str))?;
                // Simplified: schedule for next day at same time
                Ok(SystemTime::now() + Duration::from_secs(24 * 3600))
            }
            ["weekly", _day, _hour, _minute] => {
                // Simplified: schedule for next week
                Ok(SystemTime::now() + Duration::from_secs(7 * 24 * 3600))
            }
            _ => Err(format!("Invalid cron expression format: {}", cron_expr)),
        }
    }

    pub fn cleanup_old_states(&mut self, max_age_hours: u64) -> std::io::Result<usize> {
        self.state_manager.cleanup_old_states(max_age_hours)
    }

    pub fn get_workflow_history(
        &self,
        workflow_id: &str,
    ) -> std::io::Result<Option<WorkflowState>> {
        self.state_manager.load_state(workflow_id)
    }

    pub fn list_workflow_states(&self) -> Vec<&WorkflowState> {
        self.state_manager.list_states()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_state::WorkflowSchedule;

    fn temp_scheduler() -> (tempfile::TempDir, WorkflowScheduler) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let scheduler = WorkflowScheduler::new(dir.path().to_str().unwrap()).unwrap();
        (dir, scheduler)
    }

    fn make_schedule(cron: &str, enabled: bool) -> WorkflowSchedule {
        WorkflowSchedule {
            cron_expression: Some(cron.to_string()),
            next_run: None,
            enabled,
            max_runs: None,
            run_count: 0,
        }
    }

    #[test]
    fn run_due_lock_is_exclusive_then_released() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // First acquisition succeeds.
        let lock = RunDueLock::acquire(dir.path()).unwrap();
        assert!(lock.is_some(), "first lock acquisition should succeed");

        // Second acquisition is blocked while the first is held.
        let blocked = RunDueLock::acquire(dir.path()).unwrap();
        assert!(blocked.is_none(), "second acquisition should be blocked");

        // Releasing the first lock allows re-acquisition.
        drop(lock);
        drop(blocked);
        let reacquired = RunDueLock::acquire(dir.path()).unwrap();
        assert!(
            reacquired.is_some(),
            "lock should be re-acquirable after release"
        );
    }

    #[test]
    fn run_due_guarded_blocks_when_locked() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let mut scheduler = WorkflowScheduler::new(dir.path().to_str().unwrap()).unwrap();
        let held = RunDueLock::acquire(dir.path()).unwrap();
        assert!(held.is_some());

        let result = scheduler.run_due_workflows_guarded();
        assert!(result.is_err(), "guarded run should fail while lock held");
    }

    #[test]
    fn test_schedule_and_list_workflow() {
        let (_dir, mut scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:30", true);
        scheduler
            .schedule_workflow("wf-1".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        let listed = scheduler.list_scheduled_workflows();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "wf-1");
        assert_eq!(listed[0].1.workflow_path, "test.yaml");
    }

    #[test]
    fn test_unschedule_workflow() {
        let (_dir, mut scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:10", true);
        scheduler
            .schedule_workflow("wf-1".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();
        assert_eq!(scheduler.list_scheduled_workflows().len(), 1);

        scheduler.unschedule_workflow("wf-1").unwrap();
        assert_eq!(scheduler.list_scheduled_workflows().len(), 0);
    }

    #[test]
    fn test_parse_simple_cron_interval() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:60", true);
        let next = scheduler.calculate_next_run(&schedule);
        assert!(next.is_ok());

        // Should be roughly 60 minutes from now
        let next = next.unwrap();
        let diff = next.duration_since(SystemTime::now()).unwrap_or_default();
        assert!(diff.as_secs() >= 3550 && diff.as_secs() <= 3650);
    }

    #[test]
    fn test_parse_simple_cron_daily() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = make_schedule("daily:10:30", true);
        let next = scheduler.calculate_next_run(&schedule);
        assert!(next.is_ok());

        // Should be roughly 24 hours from now
        let next = next.unwrap();
        let diff = next.duration_since(SystemTime::now()).unwrap_or_default();
        assert!(diff.as_secs() >= 86000 && diff.as_secs() <= 86800);
    }

    #[test]
    fn test_parse_simple_cron_weekly() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = make_schedule("weekly:Mon:09:00", true);
        let next = scheduler.calculate_next_run(&schedule);
        assert!(next.is_ok());
    }

    #[test]
    fn test_parse_invalid_cron() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = make_schedule("garbage", true);
        let result = scheduler.calculate_next_run(&schedule);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_interval_value() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:abc", true);
        let result = scheduler.calculate_next_run(&schedule);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_cron_defaults_to_one_hour() {
        let (_dir, scheduler) = temp_scheduler();

        let schedule = WorkflowSchedule {
            cron_expression: None,
            next_run: None,
            enabled: true,
            max_runs: None,
            run_count: 0,
        };
        let next = scheduler.calculate_next_run(&schedule).unwrap();
        let diff = next.duration_since(SystemTime::now()).unwrap_or_default();
        assert!(diff.as_secs() >= 3550 && diff.as_secs() <= 3650);
    }

    #[test]
    fn test_get_due_workflows() {
        let (_dir, mut scheduler) = temp_scheduler();

        // Schedule with very short interval (should be immediately due after scheduling)
        // Actually, calculate_next_run returns future time. So we manipulate directly.
        let schedule = make_schedule("interval:1", true);
        scheduler
            .schedule_workflow("wf-due".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        // The next_run is 1 minute in the future, so nothing due yet
        let due = scheduler.get_due_workflows();
        assert_eq!(due.len(), 0);

        // Manually set next_run to past
        if let Some(sw) = scheduler.scheduled_workflows.get_mut("wf-due") {
            sw.next_run = SystemTime::now() - Duration::from_secs(60);
        }

        let due = scheduler.get_due_workflows();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], "wf-due");
    }

    #[test]
    fn test_disabled_workflows_not_due() {
        let (_dir, mut scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:1", false); // disabled
        scheduler
            .schedule_workflow("wf-off".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        // Force past time
        if let Some(sw) = scheduler.scheduled_workflows.get_mut("wf-off") {
            sw.next_run = SystemTime::now() - Duration::from_secs(60);
        }

        let due = scheduler.get_due_workflows();
        assert_eq!(due.len(), 0);
    }

    #[test]
    fn test_update_workflow_run_increments_count() {
        let (_dir, mut scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:10", true);
        scheduler
            .schedule_workflow("wf-1".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        scheduler.update_workflow_run("wf-1").unwrap();

        let sw = &scheduler.scheduled_workflows["wf-1"];
        assert_eq!(sw.schedule.run_count, 1);
        assert!(sw.last_run.is_some());
    }

    #[test]
    fn test_max_runs_disables_workflow() {
        let (_dir, mut scheduler) = temp_scheduler();

        let mut schedule = make_schedule("interval:10", true);
        schedule.max_runs = Some(2);
        scheduler
            .schedule_workflow("wf-max".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        scheduler.update_workflow_run("wf-max").unwrap();
        assert!(scheduler.scheduled_workflows["wf-max"].schedule.enabled);

        scheduler.update_workflow_run("wf-max").unwrap();
        assert!(!scheduler.scheduled_workflows["wf-max"].schedule.enabled);
    }

    #[test]
    fn test_workflow_state_persisted() {
        let (_dir, mut scheduler) = temp_scheduler();

        let schedule = make_schedule("interval:30", true);
        scheduler
            .schedule_workflow("wf-persist".to_string(), "test.yaml".to_string(), schedule)
            .unwrap();

        let history = scheduler.get_workflow_history("wf-persist").unwrap();
        assert!(history.is_some());
        let state = history.unwrap();
        assert!(matches!(state.status, WorkflowStatus::Scheduled));
    }

    #[test]
    fn test_scheduled_workflows_reload_from_disk() {
        let dir = tempfile::tempdir().expect("create temp dir");
        {
            let mut scheduler = WorkflowScheduler::new(dir.path().to_str().unwrap()).unwrap();
            let schedule = make_schedule("interval:30", true);
            scheduler
                .schedule_workflow("wf-reload".to_string(), "test.yaml".to_string(), schedule)
                .unwrap();
        }

        let scheduler = WorkflowScheduler::new(dir.path().to_str().unwrap()).unwrap();
        let listed = scheduler.list_scheduled_workflows();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "wf-reload");
        assert_eq!(listed[0].1.workflow_path, "test.yaml");
    }
}
