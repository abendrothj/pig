//! Job lifecycle and the bounded, concurrency-limited queue that runs generation
//! requests against a `ModelBackend`.
//!
//! Lifecycle: `Queued -> Loading -> Running -> {Succeeded, Failed, Cancelled,
//! TimedOut}`. A job never regresses out of a terminal state.

use crate::backend::{
    BackendError, BackendGenerationRequest, LoadModelRequest, ModelBackend, ModelStreamEvent,
};
use lao_orchestrator_core::execution::Artifact;
use lao_orchestrator_core::model::{
    CancellationInfo, CancellationReason, FinishReason, JobId, ModelExecutionError,
    ModelExecutionMetadata, ModelId, ModelRequest, ModelResponse, ModelResponseStatus, ModelUsage,
    RequestId, ResolvedModel,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Loading,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: JobId,
    pub request_id: RequestId,
    pub status: JobStatus,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub response: Option<ModelResponse>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What the queue needs beyond the bare `ModelRequest`: which model to run and where
/// to load it from. Resolution (role -> model -> path) happens before submission,
/// either by the coordinator's scheduler or, for a direct same-worker request naming
/// a model this worker already knows about, by the caller.
#[derive(Debug, Clone)]
pub struct ResolvedGeneration {
    pub model_id: ModelId,
    pub model_path: PathBuf,
    pub context_size: Option<u32>,
    pub execution_config: serde_json::Value,
}

#[derive(Debug)]
pub enum QueueError {
    QueueFull,
}

/// The parts of `WorkerMetricsSnapshot` derivable purely from the job map in one
/// pass - `worker/server.rs`'s metrics handler combines this with the backend's own
/// "what's loaded" answer and live hardware readings for the full schema.
#[derive(Debug, Clone, Default)]
pub struct JobMetricsSnapshot {
    pub counts: lao_orchestrator_core::model::JobMetrics,
    pub currently_loading: bool,
    pub last_load_duration_ms: Option<u64>,
    pub last_prompt_tokens_per_second: Option<f64>,
    pub last_generation_tokens_per_second: Option<f64>,
}

struct RunningJob {
    cancellation: CancellationToken,
}

pub struct WorkerRuntime {
    pub worker_id: String,
    pub host_name: String,
    backend: Arc<dyn ModelBackend>,
    backend_name: String,
    jobs: Arc<RwLock<HashMap<JobId, JobRecord>>>,
    running: Arc<RwLock<HashMap<JobId, RunningJob>>>,
    concurrency: Arc<Semaphore>,
    /// Dedicated admission capacity, independent of `concurrency`. A queue permit is
    /// reserved atomically at submission time (before the request is accepted) and
    /// released exactly once, when the job either starts running or leaves the queue
    /// some other way (cancelled, worker shutting down). Deriving "queue depth" from
    /// this permit count - rather than a counter incremented/decremented around
    /// `concurrency`'s own acquire - closes a check-then-act window where two requests
    /// arriving close together could observe stale capacity and one could be rejected
    /// even though the worker could have accepted it.
    queue_slots: Arc<Semaphore>,
    max_queued: usize,
    default_timeout: Duration,
}

impl WorkerRuntime {
    pub fn new(
        worker_id: String,
        host_name: String,
        backend: Arc<dyn ModelBackend>,
        backend_name: String,
        max_concurrent_jobs: usize,
        max_queued_jobs: usize,
        default_timeout: Duration,
    ) -> Self {
        Self {
            worker_id,
            host_name,
            backend,
            backend_name,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(HashMap::new())),
            concurrency: Arc::new(Semaphore::new(max_concurrent_jobs.max(1))),
            queue_slots: Arc::new(Semaphore::new(max_queued_jobs)),
            max_queued: max_queued_jobs,
            default_timeout,
        }
    }

    pub fn backend(&self) -> &Arc<dyn ModelBackend> {
        &self.backend
    }

    pub async fn job(&self, id: &JobId) -> Option<JobRecord> {
        self.jobs.read().await.get(id).cloned()
    }

    pub async fn list_jobs(&self) -> Vec<JobRecord> {
        let mut jobs: Vec<JobRecord> = self.jobs.read().await.values().cloned().collect();
        jobs.sort_by_key(|j| j.created_at_ms);
        jobs
    }

    /// Jobs currently holding a queue permit - i.e. admitted but not yet running.
    /// A running job has already released its queue slot (see `run_job`), so this
    /// never counts jobs that are actively executing.
    pub fn queue_depth(&self) -> usize {
        self.max_queued
            .saturating_sub(self.queue_slots.available_permits())
    }

    pub async fn active_jobs(&self) -> usize {
        self.jobs
            .read()
            .await
            .values()
            .filter(|j| matches!(j.status, JobStatus::Loading | JobStatus::Running))
            .count()
    }

    /// One pass over the job map for every metric derivable from it, rather than
    /// several separate locks/iterations - this is the single place `/v1/metrics`
    /// reads from, so there's exactly one definition of "completed" etc., not a
    /// second counter that could drift from what `jobs` actually contains.
    pub async fn job_metrics(&self) -> JobMetricsSnapshot {
        let jobs = self.jobs.read().await;
        let mut counts = lao_orchestrator_core::model::JobMetrics::default();
        let mut currently_loading = false;
        let mut latest_success_completed_at = 0u64;
        let mut last_prompt_tokens_per_second = None;
        let mut last_generation_tokens_per_second = None;
        let mut latest_load_completed_at = 0u64;
        let mut last_load_duration_ms = None;

        for record in jobs.values() {
            match record.status {
                JobStatus::Loading | JobStatus::Running => counts.active += 1,
                JobStatus::Succeeded => counts.completed += 1,
                JobStatus::Failed | JobStatus::TimedOut => counts.failed += 1,
                JobStatus::Cancelled => counts.cancelled += 1,
                JobStatus::Queued => {}
            }
            if record.status == JobStatus::Loading {
                currently_loading = true;
            }

            let Some(response) = &record.response else {
                continue;
            };
            counts.cumulative_tokens_processed +=
                (response.execution.prompt_tokens + response.execution.generated_tokens) as u64;

            let Some(completed_at) = record.completed_at_ms else {
                continue;
            };
            if response.is_success() && completed_at >= latest_success_completed_at {
                latest_success_completed_at = completed_at;
                last_prompt_tokens_per_second = response.execution.prompt_tokens_per_second;
                last_generation_tokens_per_second = response.execution.generation_tokens_per_second;
            }
            if !response.execution.model_already_loaded && completed_at >= latest_load_completed_at
            {
                latest_load_completed_at = completed_at;
                last_load_duration_ms = Some(response.execution.model_load_ms);
            }
        }

        JobMetricsSnapshot {
            counts,
            currently_loading,
            last_load_duration_ms,
            last_prompt_tokens_per_second,
            last_generation_tokens_per_second,
        }
    }

    /// Submit a generation request. Returns the job id and a stream of token events
    /// immediately; the caller polls `job()` for the final `JobRecord`/`ModelResponse`.
    /// Rejects outright when the bounded queue is full rather than growing it.
    ///
    /// Admission is a single atomic `try_acquire_owned` on `queue_slots`: there is no
    /// separate check followed by a separate increment, so two calls racing for the
    /// last slot can never both succeed, and neither can observe a state where the
    /// slot looks free only because another admitted job hasn't yet been accounted
    /// for.
    pub async fn submit(
        &self,
        request: ModelRequest,
        resolved: ResolvedGeneration,
        timeout_override: Option<Duration>,
    ) -> Result<(JobId, mpsc::Receiver<ModelStreamEvent>), QueueError> {
        let queue_permit = match self.queue_slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Err(QueueError::QueueFull),
        };

        let job_id = JobId::generate();
        let record = JobRecord {
            job_id: job_id.clone(),
            request_id: request.request_id.clone(),
            status: JobStatus::Queued,
            created_at_ms: now_ms(),
            started_at_ms: None,
            completed_at_ms: None,
            response: None,
        };
        self.jobs.write().await.insert(job_id.clone(), record);

        let cancellation = CancellationToken::new();
        self.running.write().await.insert(
            job_id.clone(),
            RunningJob {
                cancellation: cancellation.clone(),
            },
        );

        let (event_tx, event_rx) = mpsc::channel(256);
        let timeout = timeout_override.unwrap_or(self.default_timeout);

        let ctx = JobContext {
            job_id: job_id.clone(),
            worker_id: self.worker_id.clone(),
            host_name: self.host_name.clone(),
            backend_name: self.backend_name.clone(),
            backend: self.backend.clone(),
            jobs: self.jobs.clone(),
            running: self.running.clone(),
            concurrency: self.concurrency.clone(),
        };

        tokio::spawn(run_job(
            ctx,
            request,
            resolved,
            cancellation,
            event_tx,
            timeout,
            queue_permit,
        ));

        Ok((job_id, event_rx))
    }

    pub async fn cancel(&self, id: &JobId) -> bool {
        if let Some(handle) = self.running.read().await.get(id) {
            handle.cancellation.cancel();
            true
        } else {
            false
        }
    }
}

struct JobContext {
    job_id: JobId,
    worker_id: String,
    host_name: String,
    backend_name: String,
    backend: Arc<dyn ModelBackend>,
    jobs: Arc<RwLock<HashMap<JobId, JobRecord>>>,
    running: Arc<RwLock<HashMap<JobId, RunningJob>>>,
    concurrency: Arc<Semaphore>,
}

async fn set_status(jobs: &RwLock<HashMap<JobId, JobRecord>>, id: &JobId, status: JobStatus) {
    if let Some(record) = jobs.write().await.get_mut(id) {
        record.status = status;
        if status == JobStatus::Loading && record.started_at_ms.is_none() {
            record.started_at_ms = Some(now_ms());
        }
    }
}

async fn finish(ctx: &JobContext, status: JobStatus, response: ModelResponse) {
    if let Some(record) = ctx.jobs.write().await.get_mut(&ctx.job_id) {
        record.status = status;
        record.completed_at_ms = Some(now_ms());
        record.response = Some(response);
    }
    ctx.running.write().await.remove(&ctx.job_id);
}

fn base_metadata(
    ctx: &JobContext,
    resolved: &ResolvedGeneration,
    queue_wait_ms: u64,
) -> ModelExecutionMetadata {
    ModelExecutionMetadata {
        worker_id: ctx.worker_id.clone().into(),
        host_name: ctx.host_name.clone(),
        backend: ctx.backend_name.clone(),
        backend_version: None,
        model_id: resolved.model_id.clone(),
        model_identity: resolved.model_path.to_string_lossy().into_owned(),
        accelerator: None,
        cpu_threads: None,
        gpu_layers: None,
        context_tokens: resolved.context_size,
        batch_size: None,
        queue_wait_ms,
        model_load_ms: 0,
        prompt_eval_ms: 0,
        generation_ms: 0,
        total_ms: 0,
        prompt_tokens: 0,
        generated_tokens: 0,
        prompt_tokens_per_second: None,
        generation_tokens_per_second: None,
        model_already_loaded: false,
        cancellation: None,
        peak_memory_bytes: None,
        peak_vram_bytes: None,
    }
}

fn resolved_model(
    ctx: &JobContext,
    resolved: &ResolvedGeneration,
    request: &ModelRequest,
) -> ResolvedModel {
    ResolvedModel {
        model_id: resolved.model_id.clone(),
        role: Some(request.role.clone()),
        backend: ctx.backend_name.clone(),
        identity: resolved.model_path.to_string_lossy().into_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_job(
    ctx: JobContext,
    request: ModelRequest,
    resolved: ResolvedGeneration,
    cancellation: CancellationToken,
    event_tx: mpsc::Sender<ModelStreamEvent>,
    timeout: Duration,
    queue_permit: OwnedSemaphorePermit,
) {
    let queue_start = Instant::now();

    let permit = tokio::select! {
        p = ctx.concurrency.clone().acquire_owned() => {
            match p {
                Ok(p) => p,
                // Semaphore closed: worker is shutting down. `queue_permit` is
                // dropped here, releasing the queue slot.
                Err(_) => return,
            }
        }
        _ = cancellation.cancelled() => {
            // `queue_permit` is dropped at the end of this arm, releasing the queue
            // slot for a job that never got to run.
            let queue_wait_ms = queue_start.elapsed().as_millis() as u64;
            let mut metadata = base_metadata(&ctx, &resolved, queue_wait_ms);
            metadata.total_ms = queue_wait_ms;
            metadata.cancellation = Some(CancellationInfo { reason: CancellationReason::UserRequested, at_ms: now_ms() });
            let response = ModelResponse {
                request_id: request.request_id.clone(),
                status: ModelResponseStatus::Cancelled,
                output: Artifact::Null,
                finish_reason: FinishReason::Cancelled,
                model: resolved_model(&ctx, &resolved, &request),
                execution: metadata,
                usage: ModelUsage::default(),
                error: Some(ModelExecutionError::Cancelled),
            };
            finish(&ctx, JobStatus::Cancelled, response).await;
            return;
        }
    };
    // The job is now actually running: release the queue slot so a new request can
    // be admitted into it. This is the one explicit drop; every other exit path
    // above returns without it, letting the compiler-guaranteed destructor release
    // the slot instead (also covers task abort and panics during the select).
    drop(queue_permit);
    let queue_wait_ms = queue_start.elapsed().as_millis() as u64;

    set_status(&ctx.jobs, &ctx.job_id, JobStatus::Loading).await;

    let backend = ctx.backend.clone();
    let resolved_for_task = resolved.clone();
    let request_for_task = request.clone();
    let jobs_for_status = ctx.jobs.clone();
    let job_id_for_status = ctx.job_id.clone();
    let event_tx_for_task = event_tx.clone();
    let cancellation_for_task = cancellation.clone();

    let work = async move {
        let load_start = Instant::now();
        let loaded = backend
            .load_model(LoadModelRequest {
                model_id: resolved_for_task.model_id.clone(),
                path: resolved_for_task.model_path.clone(),
                context_size: resolved_for_task.context_size,
                execution_config: resolved_for_task.execution_config.clone(),
            })
            .await?;
        let model_load_ms = load_start.elapsed().as_millis() as u64;

        set_status(&jobs_for_status, &job_id_for_status, JobStatus::Running).await;

        let gen_start = Instant::now();
        let generation = backend
            .generate(
                BackendGenerationRequest {
                    request_id: request_for_task.request_id.clone(),
                    model_id: resolved_for_task.model_id.clone(),
                    messages: request_for_task.messages.clone(),
                    parameters: request_for_task.parameters.clone(),
                },
                event_tx_for_task,
                cancellation_for_task,
            )
            .await?;
        let generation_ms = gen_start.elapsed().as_millis() as u64;

        Ok::<_, BackendError>((loaded, model_load_ms, generation, generation_ms))
    };

    let outcome = tokio::time::timeout(timeout, work).await;
    drop(permit);
    let total_ms = queue_start.elapsed().as_millis() as u64;
    let _ = event_tx.send(ModelStreamEvent::Done).await;

    let mut metadata = base_metadata(&ctx, &resolved, queue_wait_ms);
    metadata.total_ms = total_ms;

    let (status, response) = match outcome {
        Err(_elapsed) => {
            cancellation.cancel();
            metadata.cancellation = Some(CancellationInfo {
                reason: CancellationReason::Timeout,
                at_ms: now_ms(),
            });
            (
                JobStatus::TimedOut,
                ModelResponse {
                    request_id: request.request_id.clone(),
                    status: ModelResponseStatus::TimedOut,
                    output: Artifact::Null,
                    finish_reason: FinishReason::TimedOut,
                    model: resolved_model(&ctx, &resolved, &request),
                    execution: metadata,
                    usage: ModelUsage::default(),
                    error: Some(ModelExecutionError::Timeout {
                        after_ms: timeout.as_millis() as u64,
                    }),
                },
            )
        }
        Ok(Err(BackendError::Cancelled)) => {
            metadata.cancellation = Some(CancellationInfo {
                reason: CancellationReason::UserRequested,
                at_ms: now_ms(),
            });
            (
                JobStatus::Cancelled,
                ModelResponse {
                    request_id: request.request_id.clone(),
                    status: ModelResponseStatus::Cancelled,
                    output: Artifact::Null,
                    finish_reason: FinishReason::Cancelled,
                    model: resolved_model(&ctx, &resolved, &request),
                    execution: metadata,
                    usage: ModelUsage::default(),
                    error: Some(ModelExecutionError::Cancelled),
                },
            )
        }
        Ok(Err(e)) => (
            JobStatus::Failed,
            ModelResponse {
                request_id: request.request_id.clone(),
                status: ModelResponseStatus::Failed,
                output: Artifact::Null,
                finish_reason: FinishReason::Error,
                model: resolved_model(&ctx, &resolved, &request),
                execution: metadata,
                usage: ModelUsage::default(),
                error: Some(ModelExecutionError::BackendError {
                    message: e.to_string(),
                }),
            },
        ),
        Ok(Ok((loaded, model_load_ms, generation, generation_ms))) => {
            metadata.model_load_ms = model_load_ms;
            metadata.generation_ms = generation_ms;
            metadata.prompt_eval_ms = generation.prompt_ms;
            metadata.prompt_tokens = generation.prompt_tokens;
            metadata.generated_tokens = generation.completion_tokens;
            metadata.prompt_tokens_per_second = generation.prompt_tokens_per_second;
            metadata.generation_tokens_per_second = generation.generation_tokens_per_second;
            metadata.model_already_loaded = loaded.already_loaded;
            metadata.accelerator = loaded.accelerator;
            metadata.cpu_threads = loaded.cpu_threads;
            metadata.gpu_layers = loaded.gpu_layers;
            metadata.batch_size = loaded.batch_size;
            metadata.context_tokens = loaded.context_tokens.or(metadata.context_tokens);

            let usage = ModelUsage {
                prompt_tokens: generation.prompt_tokens,
                completion_tokens: generation.completion_tokens,
                total_tokens: generation.prompt_tokens + generation.completion_tokens,
            };
            let output = match request.parameters.response_format {
                Some(lao_orchestrator_core::model::ResponseFormat::Json) => {
                    serde_json::from_str::<serde_json::Value>(&generation.content)
                        .map(Artifact::Json)
                        .unwrap_or_else(|_| Artifact::Text(generation.content.clone()))
                }
                _ => Artifact::Text(generation.content.clone()),
            };

            (
                JobStatus::Succeeded,
                ModelResponse {
                    request_id: request.request_id.clone(),
                    status: ModelResponseStatus::Success,
                    output,
                    finish_reason: generation.finish_reason,
                    model: resolved_model(&ctx, &resolved, &request),
                    execution: metadata,
                    usage,
                    error: None,
                },
            )
        }
    };

    finish(&ctx, status, response).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeBackend;
    use lao_orchestrator_core::model::{GenerationParameters, ModelMessage, ModelRole};

    fn runtime(max_concurrent: usize, max_queued: usize) -> WorkerRuntime {
        WorkerRuntime::new(
            "test-worker".to_string(),
            "test-host".to_string(),
            Arc::new(FakeBackend::new()),
            "fake".to_string(),
            max_concurrent,
            max_queued,
            Duration::from_secs(5),
        )
    }

    fn request(prompt: &str) -> ModelRequest {
        ModelRequest {
            request_id: RequestId::generate(),
            role: ModelRole::Reasoning,
            model: None,
            messages: vec![ModelMessage::user(prompt)],
            parameters: GenerationParameters {
                max_tokens: Some(4),
                ..Default::default()
            },
            requirements: Default::default(),
            inputs: vec![],
            metadata: Default::default(),
        }
    }

    fn resolved(model_id: &str) -> ResolvedGeneration {
        ResolvedGeneration {
            model_id: ModelId::from(model_id),
            model_path: PathBuf::from("/models/fake.gguf"),
            context_size: Some(4096),
            execution_config: serde_json::Value::Null,
        }
    }

    async fn wait_for_terminal(rt: &WorkerRuntime, id: &JobId) -> JobRecord {
        for _ in 0..200 {
            if let Some(record) = rt.job(id).await {
                if matches!(
                    record.status,
                    JobStatus::Succeeded
                        | JobStatus::Failed
                        | JobStatus::Cancelled
                        | JobStatus::TimedOut
                ) {
                    return record;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job {} did not reach a terminal state in time", id);
    }

    /// Poll `active_jobs()` until it matches `expected`, bounded so a regression
    /// hangs the test instead of the whole suite.
    async fn wait_for_active_jobs(rt: &WorkerRuntime, expected: usize) {
        for _ in 0..500 {
            if rt.active_jobs().await == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!(
            "active_jobs never reached {} within timeout (last observed {})",
            expected,
            rt.active_jobs().await
        );
    }

    #[tokio::test]
    async fn successful_generation_produces_a_response_with_metadata() {
        let rt = runtime(1, 4);
        let (id, _events) = rt
            .submit(request("one two three"), resolved("m1"), None)
            .await
            .ok()
            .unwrap();
        let record = wait_for_terminal(&rt, &id).await;
        assert_eq!(record.status, JobStatus::Succeeded);
        let response = record.response.unwrap();
        assert!(response.is_success());
        assert_eq!(response.execution.worker_id.0, "test-worker");
        assert_eq!(response.execution.model_id.0, "m1");
        assert!(response.execution.generated_tokens > 0);
    }

    #[tokio::test]
    async fn cancel_marks_job_cancelled() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(50),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            4,
            Duration::from_secs(5),
        );
        let (id, _events) = rt
            .submit(request("one two three four five"), resolved("m1"), None)
            .await
            .ok()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(rt.cancel(&id).await);
        let record = wait_for_terminal(&rt, &id).await;
        assert_eq!(record.status, JobStatus::Cancelled);
        assert_eq!(
            record.response.unwrap().error,
            Some(ModelExecutionError::Cancelled)
        );
    }

    #[tokio::test]
    async fn timeout_marks_job_timed_out() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(50),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            4,
            Duration::from_secs(5),
        );
        let mut req = request("one two three four five");
        req.parameters.max_tokens = Some(100);
        let (id, _events) = rt
            .submit(req, resolved("m1"), Some(Duration::from_millis(30)))
            .await
            .ok()
            .unwrap();
        let record = wait_for_terminal(&rt, &id).await;
        assert_eq!(record.status, JobStatus::TimedOut);
        assert!(matches!(
            record.response.unwrap().error,
            Some(ModelExecutionError::Timeout { .. })
        ));
    }

    #[tokio::test]
    async fn backend_failure_surfaces_as_failed_job() {
        let rt = runtime(1, 4);
        let (id, _events) = rt
            .submit(request("hi"), resolved("fail-to-generate"), None)
            .await
            .ok()
            .unwrap();
        let record = wait_for_terminal(&rt, &id).await;
        assert_eq!(record.status, JobStatus::Failed);
        assert!(matches!(
            record.response.unwrap().error,
            Some(ModelExecutionError::BackendError { .. })
        ));
    }

    #[tokio::test]
    async fn queue_rejects_submissions_once_full() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(200),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1, // only one concurrent job, so the rest pile up in the queue
            1, // and the queue only holds one
            Duration::from_secs(5),
        );
        let mut req = request("one two three");
        req.parameters.max_tokens = Some(20);
        let first = rt.submit(req.clone(), resolved("m1"), None).await;
        assert!(first.is_ok());
        let second = rt.submit(req, resolved("m1"), None).await;
        assert!(matches!(second, Err(QueueError::QueueFull)));
    }

    #[tokio::test]
    async fn queue_capacity_zero_rejects_waiting_jobs() {
        // A queue permit is reserved unconditionally at admission time, before it's
        // known whether an execution slot happens to be free - so zero queue
        // capacity rejects every submission outright, even one that could have run
        // immediately. This preserves the pre-existing max_queued=0 semantics under
        // the new atomic-permit implementation.
        let rt = runtime(1, 0);
        let result = rt.submit(request("hi"), resolved("m1"), None).await;
        assert!(matches!(result, Err(QueueError::QueueFull)));
    }

    #[tokio::test]
    async fn running_job_does_not_consume_queue_slot() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(200),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            1,
            Duration::from_secs(5),
        );
        let mut req = request("one two three");
        req.parameters.max_tokens = Some(20);
        let (_id, _events) = rt.submit(req.clone(), resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;
        assert_eq!(
            rt.queue_depth(),
            0,
            "a running job must not occupy a queue slot"
        );
        // The single queue slot must still be fully available for a new submission.
        let second = rt.submit(req, resolved("m1"), None).await;
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn queued_job_releases_slot_when_started() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(20),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            1,
            Duration::from_secs(5),
        );
        let mut req = request("one two three");
        req.parameters.max_tokens = Some(3);
        let (first_id, _e1) = rt.submit(req.clone(), resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;
        let (second_id, _e2) = rt.submit(req, resolved("m1"), None).await.unwrap();
        assert_eq!(rt.queue_depth(), 1, "second job should be sitting queued");

        wait_for_terminal(&rt, &first_id).await;
        wait_for_terminal(&rt, &second_id).await;
        assert_eq!(
            rt.queue_depth(),
            0,
            "queue slot must be released once the queued job started running"
        );
    }

    #[tokio::test]
    async fn cancelled_queued_job_releases_slot() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(300),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            1,
            Duration::from_secs(5),
        );
        let mut blocker = request("one two three");
        blocker.parameters.max_tokens = Some(50); // long-running: keeps the execution slot busy
        rt.submit(blocker, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;

        let (second_id, _events) = rt
            .submit(request("hi"), resolved("m1"), None)
            .await
            .unwrap();
        assert_eq!(rt.queue_depth(), 1);

        assert!(rt.cancel(&second_id).await);
        let record = wait_for_terminal(&rt, &second_id).await;
        assert_eq!(record.status, JobStatus::Cancelled);
        assert_eq!(
            rt.queue_depth(),
            0,
            "cancelling a queued job must release its queue slot"
        );
    }

    #[tokio::test]
    async fn concurrent_admission_never_exceeds_capacity() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(300),
        ));
        let max_concurrent = 1;
        let max_queued = 3;
        let rt = Arc::new(WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            max_concurrent,
            max_queued,
            Duration::from_secs(5),
        ));

        // Occupy the only execution slot with a long-running job so nothing admitted
        // during the burst below can transition out of "queued" mid-test - isolates
        // the admission race from execution-slot timing entirely.
        let mut blocker = request("one two three");
        blocker.parameters.max_tokens = Some(50);
        rt.submit(blocker, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;

        let n = max_queued + 5;
        let barrier = Arc::new(tokio::sync::Barrier::new(n));
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let rt = rt.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                rt.submit(request("hi"), resolved("m1"), None).await.is_ok()
            }));
        }
        let mut accepted = 0;
        for h in handles {
            if h.await.unwrap() {
                accepted += 1;
            }
        }
        assert_eq!(
            accepted, max_queued,
            "admission under concurrent contention must accept exactly the queue capacity, never more"
        );
    }

    #[tokio::test]
    async fn exactly_capacity_requests_are_accepted() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(300),
        ));
        let max_queued = 4;
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            max_queued,
            Duration::from_secs(5),
        );
        let mut blocker = request("one two three");
        blocker.parameters.max_tokens = Some(50);
        rt.submit(blocker, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;

        for i in 0..max_queued {
            let result = rt.submit(request("hi"), resolved("m1"), None).await;
            assert!(
                result.is_ok(),
                "submission {} within capacity should be accepted",
                i
            );
        }
    }

    #[tokio::test]
    async fn capacity_plus_one_returns_429() {
        // job.rs deals in QueueError::QueueFull; the HTTP layer (worker/server.rs)
        // maps this 1:1 onto a 429 response - see queue_overflow_returns_429 in
        // worker/tests/http_server_test.rs for the end-to-end version.
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(300),
        ));
        let max_queued = 2;
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            max_queued,
            Duration::from_secs(5),
        );
        let mut blocker = request("one two three");
        blocker.parameters.max_tokens = Some(50);
        rt.submit(blocker, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;

        for _ in 0..max_queued {
            assert!(rt.submit(request("hi"), resolved("m1"), None).await.is_ok());
        }
        let overflow = rt.submit(request("hi"), resolved("m1"), None).await;
        assert!(matches!(overflow, Err(QueueError::QueueFull)));
    }

    #[tokio::test]
    async fn repeated_contention_test_has_no_spurious_429() {
        // Exactly `max_queued` concurrent submissions released together via a
        // barrier (not a sleep) must all succeed - total demand never exceeds
        // supply, so a spurious rejection here can only mean the admission check
        // is racy. Repeated many times to build real confidence, the same way the
        // original queue_overflow_returns_429 flakiness only showed up under
        // repetition.
        for iteration in 0..40 {
            let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
                Duration::from_millis(50),
            ));
            let max_queued = 4;
            let rt = Arc::new(WorkerRuntime::new(
                "w".to_string(),
                "h".to_string(),
                backend,
                "fake".to_string(),
                1,
                max_queued,
                Duration::from_secs(5),
            ));
            let barrier = Arc::new(tokio::sync::Barrier::new(max_queued));
            let mut handles = Vec::with_capacity(max_queued);
            for _ in 0..max_queued {
                let rt = rt.clone();
                let barrier = barrier.clone();
                handles.push(tokio::spawn(async move {
                    barrier.wait().await;
                    rt.submit(request("hi"), resolved("m1"), None).await.is_ok()
                }));
            }
            for h in handles {
                assert!(
                    h.await.unwrap(),
                    "iteration {}: exactly-capacity concurrent submissions must never be spuriously rejected",
                    iteration
                );
            }
        }
    }

    #[tokio::test]
    async fn unknown_job_id_cancel_returns_false() {
        let rt = runtime(1, 4);
        assert!(!rt.cancel(&JobId::generate()).await);
    }

    #[tokio::test]
    async fn job_metrics_active_count_reflects_a_running_job() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(200),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            4,
            Duration::from_secs(5),
        );
        let before = rt.job_metrics().await;
        assert_eq!(before.counts.active, 0);

        let mut req = request("one two three");
        req.parameters.max_tokens = Some(20);
        rt.submit(req, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;

        let during = rt.job_metrics().await;
        assert_eq!(during.counts.active, 1);
        assert_eq!(during.counts.completed, 0);
    }

    #[tokio::test]
    async fn job_metrics_counts_completed_failed_and_cancelled_separately() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(200),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            4, // enough concurrency that these three don't queue behind each other
            4,
            Duration::from_secs(5),
        );

        let (ok_id, _e1) = rt
            .submit(request("one two three"), resolved("m1"), None)
            .await
            .unwrap();
        wait_for_terminal(&rt, &ok_id).await;

        let (fail_id, _e2) = rt
            .submit(request("hi"), resolved("fail-to-generate"), None)
            .await
            .unwrap();
        wait_for_terminal(&rt, &fail_id).await;

        let mut cancel_req = request("one two three four five");
        cancel_req.parameters.max_tokens = Some(50);
        let (cancel_id, _e3) = rt.submit(cancel_req, resolved("m1"), None).await.unwrap();
        wait_for_active_jobs(&rt, 1).await;
        assert!(rt.cancel(&cancel_id).await);
        wait_for_terminal(&rt, &cancel_id).await;

        let metrics = rt.job_metrics().await;
        assert_eq!(metrics.counts.active, 0);
        assert_eq!(metrics.counts.completed, 1, "exactly the successful job");
        assert_eq!(metrics.counts.failed, 1, "exactly the fail-to-generate job");
        assert_eq!(metrics.counts.cancelled, 1, "exactly the cancelled job");
    }

    #[tokio::test]
    async fn job_metrics_throughput_does_not_regress_after_a_later_failure() {
        let backend = Arc::new(crate::backend::fake::FakeBackend::with_token_delay(
            Duration::from_millis(20),
        ));
        let rt = WorkerRuntime::new(
            "w".to_string(),
            "h".to_string(),
            backend,
            "fake".to_string(),
            1,
            4,
            Duration::from_secs(5),
        );

        let (ok_id, _e1) = rt
            .submit(request("one two three"), resolved("m1"), None)
            .await
            .unwrap();
        wait_for_terminal(&rt, &ok_id).await;

        let after_success = rt.job_metrics().await;
        assert!(after_success.last_prompt_tokens_per_second.is_some());
        assert!(after_success.last_generation_tokens_per_second.is_some());
        assert!(
            after_success.last_load_duration_ms.is_some(),
            "the first job for a model incurs a real load (FakeBackend sleeps 5ms), so this must be Some"
        );

        let (fail_id, _e2) = rt
            .submit(request("hi"), resolved("fail-to-generate"), None)
            .await
            .unwrap();
        wait_for_terminal(&rt, &fail_id).await;

        let after_failure = rt.job_metrics().await;
        assert_eq!(
            after_failure.last_prompt_tokens_per_second,
            after_success.last_prompt_tokens_per_second,
            "a later failed job must not blank out the last real throughput reading"
        );
        assert_eq!(
            after_failure.last_generation_tokens_per_second,
            after_success.last_generation_tokens_per_second,
        );
    }
}
