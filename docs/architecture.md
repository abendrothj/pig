# pig Architecture

## Overview

pig is an inference coordinator for local llama.cpp workers. It routes generation requests to the right worker based on hardware constraints, exposes an OpenAI-compatible gateway, and manages worker lifecycle via systemd.

## Components

- **Worker** (`pig-worker`): Supervises a `llama-server` subprocess, exposes a JSON HTTP API, manages a bounded job queue, and reports hardware capabilities and telemetry.
- **Coordinator** (`worker::coordinator`): Schedules requests across configured workers using constraint-based selection. Implemented as a synchronous `ModelInvoker` over `reqwest::blocking` — no async runtime required at the call site.
- **Coordinator server** (`worker::coordinator_server`): Persistent HTTP service exposing an OpenAI-compatible gateway (`POST /v1/chat/completions`, `GET /v1/models`) and a control plane API.
- **Core** (`pig-core`): Shared types — `ModelRequest`, `ModelResponse`, `ModelRole`, `ReasoningMode`, `PlacementPolicy`, `Artifact`, scheduler logic, model registry.
- **CLI** (`pig`): Manages workers, models, routes, jobs, and coordinator lifecycle.

## Request flow

```
pig / OpenAI client
        |
        v
Coordinator (scheduler)
        |-- hard constraints (placement policy, accelerator, locality)
        |-- scoring (load, capability match)
        v
Worker HTTP API  (/v1/generate or /v1/stream)
        |
        v
ModelBackend (llama_cpp)
        |
        v
llama-server subprocess
        |
        v
ModelResponse (output artifact + execution metadata)
```

## Scheduling

Two layers:

1. **Hard constraints** — reject workers that cannot satisfy the request: model not in the worker's known-model list, wrong accelerator type, remote worker when `PlacementPolicy::LocalOnly` is set, queue full, available memory below `minimum_available_memory_bytes`.
2. **Scoring** — among eligible workers, score by queue depth, benchmark throughput, and whether the requested model is already hot in memory (+100 bonus for a loaded model, avoiding a cold-start load).

Both inputs — `loaded_models` and `available_memory_bytes` — are populated from the worker's `/v1/capabilities` response on every snapshot probe. Workers report them from `backend.list_models()` and a TTL-cached hardware probe respectively.

`ReasoningMode` (Auto/Enabled/Disabled) is resolved in the coordinator before dispatch. `Auto` maps to `Enabled` for reasoning/coding roles and when tools are present; `Disabled` otherwise. The backend receives only the resolved value and injects the appropriate control token (`/think` or `/no_think`) for Qwen3-style models.

## Worker isolation

Each worker is an independent OS process running its own `llama-server`. pig does not split model layers across machines — it routes different jobs to different workers. Multiple workers on the same machine use different ports.

Workers can be run directly via `pig worker serve` or installed as a systemd service via `pig worker install`.
