# pig Architecture

## Overview

pig is a hardware-aware inference scheduler and coordinator for local model workers.
It routes generation requests across workers based on real runtime measurements —
load state, throughput, TTFT, memory — and exposes an OpenAI-compatible gateway.
pig does not decompose tasks, orchestrate agents, or understand domains. It owns
compute; the caller owns intelligence.

## Scope

**Pig is compute, not intelligence.**

| Pig's job | Not pig's job |
|---|---|
| Which `(worker, model)` handles this request fastest? | What should the request say? |
| Load model X on worker Y | Decompose a task into subtasks |
| Reject requests that exceed a worker's context window | Manage conversation history |
| Report VRAM, throughput, and TTFT | Choose between local and cloud inference |
| Route tool-calling requests to capable models | Build or orchestrate agents |

External APIs (OpenAI, Anthropic, Groq) are the caller's concern. If an agent needs
to fall back to a cloud model when local workers are busy, that logic belongs in the
agent layer — your orchestrator, your application code. Pig routes to your hardware
and nothing else. This boundary is intentional: pig is closer to a cluster scheduler
than an AI framework.

## Components

- **Worker** (`pig-worker`): Supervises an inference subprocess (`llama-server` or
  `mlx_lm.server`), exposes a JSON HTTP API, manages a bounded job queue, and
  reports hardware capabilities and telemetry.
- **Coordinator** (`worker::coordinator`): Schedules requests across configured
  workers using constraint-based selection. Implemented as a synchronous
  `ModelInvoker` over `reqwest::blocking` — no async runtime required at the call
  site.
- **Coordinator server** (`worker::coordinator_server`): Persistent HTTP service
  exposing an OpenAI-compatible gateway (`POST /v1/chat/completions`,
  `GET /v1/models`, `POST /v1/pipeline`) and a control plane API.
- **Core** (`pig-core`): Shared types — `ModelRequest`, `ModelResponse`,
  `ModelRole`, `ReasoningMode`, `PlacementPolicy`, scheduler logic, model registry,
  benchmark store.
- **CLI** (`pig`): Manages workers, models, routes, jobs, and coordinator lifecycle.

## Scheduling primitive: ModelInstance

The scheduler's unit of capacity is a **ModelInstance** — a `(worker, model)` pair
with known runtime state:

```
Hardware
   ↓
Worker
   ↓
ModelInstance  ←  load state, context window, accelerators, benchmark data
   ↓
Scheduler decision
   ↓
Inference execution
```

Two workers running the same model are distinct instances. One may be hot, one cold;
one may have benchmark data, one may not. `GET /v1/instances` surfaces the full
view. Benchmark data is always absent rather than penalised — a model with no
history is not disadvantaged against benchmarked peers.

## Request flow

```
pig / OpenAI client
        |
        v
Coordinator (scheduler)
        |-- hard constraints (placement policy, accelerator, locality,
        |                     context window, memory, tool_calling, reasoning)
        |-- scoring (load state, queue depth, throughput, TTFT)
        v
Worker HTTP API  (/v1/generate or /v1/stream)
        |
        v
ModelBackend (llama_cpp | mlx)
        |
        v
llama-server / mlx_lm.server subprocess
        |
        v
ModelResponse (output artifact + full execution metadata)
```

## Scheduling

Two phases, always in order:

1. **Hard constraints** — reject `(worker, model)` placements that cannot satisfy
   the request. Each rejection carries an explicit reason. All constraints are
   evaluated (not short-circuited) so the routing explanation is complete.
   Constraints include: model not known to worker, wrong accelerator type, remote
   worker when `PlacementPolicy::LocalOnly`, queue full, available memory below
   floor, context window too small, `tool_calling` or `reasoning` capability absent.

2. **Scoring** — among eligible placements, score by:
   - +100 if the model is already loaded (avoids cold-start)
   - Queue depth (inverse — fewer queued jobs scores higher)
   - Measured generation throughput from benchmark records
   - TTFT (lower p50_ttft_ms scores higher — capped contribution)
   - Worker priority setting
   - Accelerator preference

Both `loaded_models` and `available_memory_bytes` are populated live from each
worker's `/v1/capabilities` response on every snapshot probe.

## Capability routing

Capabilities are facts about a model, declared in `pig.toml`:

- `tool_calling: bool` — model supports structured tool calls. `llama_cpp` defaults
  true; `mlx` defaults false. Per-entry override wins.
- `reasoning: bool` — model supports extended chain-of-thought (Qwen3 `/think`,
  DeepSeek R1, etc.). Not declared by default; opt in per model.

Requests set matching requirements (`requirements.reasoning = true`); the scheduler
hard-rejects any model that doesn't satisfy them.

`ReasoningMode` (Auto/Enabled/Disabled) is resolved in the coordinator before
dispatch. `Auto` maps to `Enabled` for reasoning/coding roles and when tools are
present; `Disabled` otherwise. The backend receives only the resolved value and
injects the appropriate control token (`/think` or `/no_think`) for Qwen3-style
models.

## Backends

| Backend | Runtime | Model format | Accelerator |
|---|---|---|---|
| `llama_cpp` | `llama-server` subprocess | GGUF | CUDA, Metal, Vulkan, CPU |
| `mlx` | `mlx_lm.server` subprocess | HuggingFace directory | Metal (Apple Silicon only) |
| `fake` | in-process | — | — (CI / tests) |

Each worker runs exactly one backend. Multiple workers on the same machine use
different ports. pig does not split model layers across machines.

## Auto-benchmark

`pig models load` triggers a background benchmark immediately after a successful
load. The benchmark runs 3 passes and stores the median generation t/s, prompt t/s,
and p50 TTFT to `.pig_benchmarks/<model-id>.jsonl`. The scheduler uses this data on
the next request — no explicit `pig models benchmark` call is needed.

## Worker isolation

Each worker is an independent OS process. pig routes different jobs to different
workers; it never distributes one model's layers between machines. Workers can be
run directly via `pig worker serve` or installed as a systemd service via
`pig worker install`.
