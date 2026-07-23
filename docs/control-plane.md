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
