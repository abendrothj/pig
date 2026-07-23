# Coordinator Control Plane API

The coordinator exposes a control plane alongside its OpenAI-compatible gateway.
All endpoints require the bearer token when `--auth-token-env` is set.

## Health

```
GET /v1/health
```

```json
{
  "status": "ok",
  "uptime_seconds": 3612,
  "workers_total": 2,
  "workers_healthy": 2
}
```

`status` is `"ok"` when all workers are healthy, `"degraded"` when some are, and
`"unavailable"` when none are. Does not return a non-2xx status — callers should
read the `status` field.

## Workers

```
GET /v1/workers
```

Returns a `WorkerSnapshot` array — the same structs the scheduler uses. Each entry
includes `worker_id`, `healthy`, `backend`, `loaded_models`, `known_models`,
`available_memory_bytes`, `queue_depth`, `active_jobs`, `benchmarks`, and `locality`.
Useful for observability and debugging routing decisions without running the CLI.

## Model instances

```
GET /v1/instances
```

Returns a `ModelInstance` array — the coordinator's view of all available inference
capacity across every configured worker. Each entry combines the worker snapshot data
with registry metadata:

```json
[
  {
    "instance_id": "m4-worker/qwen3-8b-mlx",
    "worker_id": "m4-worker",
    "model_id": "qwen3-8b-mlx",
    "backend": "mlx",
    "loaded": true,
    "context_tokens": 32768,
    "accelerators": ["metal", "cpu"],
    "benchmark": {
      "generation_tokens_per_second": 120.4,
      "prompt_tokens_per_second": 800.2,
      "p50_ttft_ms": 210
    }
  },
  {
    "instance_id": "linux-worker/qwen3-14b-q4",
    "worker_id": "linux-worker",
    "model_id": "qwen3-14b-q4",
    "backend": "llama_cpp",
    "loaded": false,
    "context_tokens": 32768,
    "accelerators": ["cuda", "cpu"],
    "benchmark": null
  }
]
```

Results are sorted: loaded instances first, then by `generation_tokens_per_second`
descending. This is the best single API call to understand what pig has available and
how fast each option is.

Two instances of the same model on different workers are distinct resources — one may
be hot, one cold; one may have benchmark data, one may not.

## Route explanation

```
POST /v1/route
Content-Type: application/json

{ "role": "coding", "messages": [...] }
```

Returns a `RoutingExplanation`:

```json
{
  "selected": {
    "worker_id": "linux-worker",
    "model_id": "qwen-coder-7b-q6",
    "score": 142.5,
    "used_cpu_fallback": false
  },
  "rejected": [
    { "worker_id": "mac-worker", "reason": "model 'qwen-coder-7b-q6' is not known to this worker" }
  ],
  "all_candidates": [...]
}
```

Accepts the same body as `POST /v1/generate`. Does not invoke any model — pure
scheduling dry-run.

## Direct generation

```
POST /v1/generate
Content-Type: application/json
```

Accepts a `ModelRequest` (the internal type, not the OpenAI wrapper) and returns a
`ModelResponse`. Useful when you want full scheduling control without the OpenAI
translation layer.

## Jobs

```
GET  /v1/jobs?worker=<worker-id>
GET  /v1/jobs/<job-id>?worker=<worker-id>
POST /v1/jobs/<job-id>/cancel?worker=<worker-id>
```

Proxied directly to the target worker's job API. Returns the worker's response
unchanged. `worker` is the `id` field from the `[[workers]]` config block.
