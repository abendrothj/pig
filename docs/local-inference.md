# pig — Local Model Inference

pig can run local language models through a supervised `llama-server` (llama.cpp)
process, route requests across one or more registered workers based on hardware and
model requirements, and invoke a model from inside a workflow step. This document is
the setup and reference guide; architectural details live inline in the relevant
source modules (`core/model/`, `worker/`).

## Architecture

```
pig / OpenAI client
        |
        v
Coordinator (scheduler + HTTP client)
        |
        v
Worker (separate process, `pig worker serve`)
        |
        v
ModelBackend (llama_cpp | mlx)
        |
        v
llama-server / mlx_lm.server subprocess
        |
        v
ModelResponse (structured: output artifact + full execution metadata)
```

A worker is always a separate OS process from the CLI or coordinator. `core` defines `ModelInvoker`, a synchronous trait implemented by `worker::coordinator::Coordinator` using `reqwest::blocking` — calling it never requires a nested tokio runtime. All async complexity lives in `pig-worker`.

**pig does not split one model across multiple machines.** Each worker runs its own
independent `llama-server`. Scaling across machines means routing *different* jobs to
*different* workers, not distributing one model's layers between them.

## Installing llama.cpp

macOS (Homebrew):

```bash
brew install llama.cpp
llama-server --version   # confirm it's on PATH
```

Linux: build from source (https://github.com/ggml-org/llama.cpp) with CUDA support
(`-DGGML_CUDA=ON`) if you have an NVIDIA GPU, or install a prebuilt release. pig invokes
`llama-server` directly — it does not build or vendor llama.cpp itself.

pig detects the installed build's actual capabilities rather than assuming a fixed flag
set: it parses `llama-server --help` once per worker process and only passes flags the
installed binary actually advertises, and calls `llama-server --list-devices` to detect
CUDA/Metal/Vulkan availability. Different llama.cpp builds are handled automatically —
there's nothing to configure for this.

## Installing MLX (Apple Silicon only)

```bash
pip install mlx-lm
mlx_lm.server --help   # confirm it's on PATH
```

MLX models are HuggingFace-format directories, not GGUF files. Download one:

```bash
pip install huggingface-hub
huggingface-cli download mlx-community/Qwen3-8B-4bit --local-dir ~/models/Qwen3-8B-4bit
```

Then configure a worker to use the MLX backend:

```toml
[worker]
id = "m4-worker"
bind = "127.0.0.1:9847"

[worker.auth]
enabled = false

[worker.runtime.llama_cpp]
enabled = false

[worker.runtime.mlx]
enabled = true
server_executable = "mlx_lm.server"

[models.entries."qwen3-8b-mlx"]
format = "mlx"
path = "/Users/you/models/Qwen3-8B-4bit"
backend = "mlx"
context_tokens = 32768
roles = ["reasoning"]

[models.roles.reasoning]
candidates = ["qwen3-8b-mlx"]
```

MLX handles quantization and Metal acceleration automatically — there are no `gpu_layers` or `flash_attention` knobs. The `execution_config` for MLX accepts only `trust_remote_code` (boolean, default false) and `seed` (integer).

pig detects that the MLX backend is running and reports `Metal` as the accelerator in worker snapshots and routing explanations, so the scheduler scores MLX workers the same way as llama.cpp workers with Metal.

## Adding a GGUF model

```bash
pig models discover --directory ~/models
```

This only scans and prints — it never writes configuration for you. Add the model to
`pig.toml` yourself:

```toml
[models.entries."qwen3-0.6b"]
format = "gguf"
path = "/Users/you/models/Qwen3-0.6B-Q4_K_M.gguf"
backend = "llama_cpp"
context_tokens = 2048
roles = ["reasoning"]

# Optional: backend execution parameters passed to llama-server at load time.
# All fields are optional; unsupported flags are silently skipped.
[models.entries."qwen3-0.6b".execution_config]
gpu_layers = -1          # -1 = all layers on GPU
flash_attention = true   # significant throughput gain on CUDA and Metal
parallel = 2             # concurrent request slots (continuous batching)
cache_type_k = "q8_0"   # KV cache quantization: halves VRAM, negligible quality loss
cache_type_v = "q8_0"
```

**Model IDs containing a period must be quoted** in the TOML table header
(`[models.entries."qwen3-0.6b"]`, not `[models.entries.qwen3-0.6b]`) — otherwise TOML
parses the period as a nested-table separator and the entry silently splits into the
wrong keys. This is standard TOML syntax, not a pig-specific quirk, but it's an easy
mistake with quantization-style names.

`tool_calling` overrides the backend's default capability at the model level. `llama_cpp`
defaults to `true`; `mlx` defaults to `false`. Set it explicitly when a model's actual
support differs from its backend's default:

```toml
[models.entries."qwen3-8b-q4"]
# ...
tool_calling = true    # explicit: this model supports tool calls via llama_cpp
```

Roles let a workflow ask for "reasoning" or "coding" without naming a specific model:

```toml
[models.roles.reasoning]
candidates = ["qwen3-14b-q4", "qwen3-8b-q6"]   # tried in this order, all else equal

[models.roles.coding]
candidates = ["qwen-coder-7b-q6", "qwen3-8b-q6"]
```

A worker refuses to start bound to a non-loopback address unless `worker.auth.enabled = true`. There is no way to expose a worker to the public internet through default configuration.

## Running one local worker

```toml
[worker]
id = "macbook-worker"
bind = "127.0.0.1:9847"
max_concurrent_jobs = 1
max_queued_jobs = 16
shutdown_grace_seconds = 20

[worker.auth]
enabled = false

[worker.runtime.llama_cpp]
enabled = true
server_executable = "llama-server"   # or an absolute path
startup_timeout_seconds = 60
request_timeout_seconds = 600

[models.entries."qwen3-0.6b"]
format = "gguf"
path = "/Users/you/models/Qwen3-0.6B-Q4_K_M.gguf"
backend = "llama_cpp"
context_tokens = 2048
roles = ["reasoning"]

[models.roles.reasoning]
candidates = ["qwen3-0.6b"]
```

```bash
pig worker serve --config pig.toml
```

The worker starts immediately (health becomes `ok` as soon as the `llama-server`
executable itself is confirmed runnable) — no model is loaded until the first request
or an explicit `pig models load`. Stop it with Ctrl-C or `SIGTERM`; it waits up to
`shutdown_grace_seconds` for in-flight jobs to finish, then exits, and the supervised
`llama-server` child is always terminated on exit (`kill_on_drop`), never orphaned.

## Connecting two machines

On the coordinator side (wherever you run `pig` or workflows from), add the second
machine to the same `pig.toml`:

```toml
[[workers]]
id = "macbook-worker"
url = "http://127.0.0.1:9847"

[[workers]]
id = "linux-worker"
url = "http://100.x.y.z:9847"        # e.g. a Tailscale/VPN address
auth_token_env = "PIG_LINUX_WORKER_TOKEN"
```

On the Linux worker itself, require auth since its bind address is non-loopback:

```toml
[worker]
id = "linux-worker"
bind = "0.0.0.0:9847"

[worker.auth]
enabled = true
token_env = "PIG_LINUX_WORKER_TOKEN"
```

```bash
export PIG_LINUX_WORKER_TOKEN=$(openssl rand -hex 32)
pig worker serve --config pig.toml
```

The coordinator polls each configured worker's `/v1/health` and `/v1/capabilities`
before every routing decision. If one worker is unreachable it gets a rejection reason
in the routing explanation rather than being silently dropped — the coordinator keeps
operating and routes to whichever configured worker is actually eligible. HTTP (not
HTTPS) is fine over loopback or a trusted private network (VPN/Tailscale); pig does not
implement public worker discovery and workers are never exposed to the internet by
default configuration.

## Platform support

| Platform | Backend | Status |
|---|---|---|
| macOS (Apple Silicon) | llama.cpp + Metal | Tested — Metal autodetected, no configuration needed |
| macOS (Apple Silicon) | MLX | Tested — 1.5–2× faster than llama.cpp on M-series; requires `mlx-lm` Python package |
| Linux x86\_64 + NVIDIA GPU | llama.cpp + CUDA | Tested — build llama.cpp with `-DGGML_CUDA=ON`; pig detects CUDA via `nvidia-smi` and `--list-devices` |
| Linux x86\_64 (CPU only) | llama.cpp | Tested — set `allow_cpu_fallback = true` when GPU VRAM is insufficient |
| Windows | llama.cpp | Compiles; systemd lifecycle commands (`worker install/start/stop`) do not apply |
| AMD ROCm / Vulkan | llama.cpp | llama.cpp supports these; pig's hardware probe does not enumerate them yet |

**Not supported in v0.5:**
- Distributed single-model inference across machines (no tensor or pipeline parallelism)
- Embeddings and reranking (both backends return 501 — `/v1/embed` and `/v1/rerank` are not implemented)
- Automatic model downloads (all model files must be present on the worker's local filesystem)


## CLI reference

```bash
pig worker serve [--config pig.toml]

pig workers list [--json]
pig workers inspect <worker-id> [--json]
pig workers health [--json]          # non-zero exit if any worker is unhealthy

pig models list [--json]
pig models inspect <model-id> [--json]
pig models discover --directory <path>
pig models load <model-id> [--worker <id>]
pig models unload <model-id> [--worker <id>]
pig models generate --prompt "..." [--role reasoning | --model <id>] \
    [--system "..."] [--max-tokens N] [--temperature F] \
    [--stream] [--json] [--force-worker <id>] [--force-cpu]
pig models benchmark <model-id> [--worker <id>] [--json]

pig route explain [--role reasoning | --model <id>] [--json]

pig jobs list --worker <id> [--json]
pig jobs inspect <job-id> --worker <id> [--json]
pig jobs cancel <job-id> --worker <id>

pig coordinator serve [--config pig.toml] [--bind 0.0.0.0:3001] [--auth-token-env VAR]
```

The coordinator exposes `GET /v1/models`, `POST /v1/chat/completions`, and
`POST /v1/pipeline`. See [openai-compatibility.md](openai-compatibility.md) for
the full HTTP API reference, including the pipeline endpoint's per-step
`role`, `requirements`, and `inject_previous` context-threading semantics.

`--stream` prints tokens as they arrive (interactive use); without it, `models
generate` waits for the full response and prints the final text (or the complete
structured `ModelResponse` with `--json`, for scripting). Every subcommand's non-zero
exit code reflects failure — `workers health`, for instance, exits 1 if any configured
worker is unhealthy.

### End-to-end walkthrough

```bash
# 1. Configure (see "Running one local worker" above) and start the worker
pig worker serve --config pig.toml &

# 2. Confirm it's up
pig workers health

# 3. Direct generation
pig models generate --role reasoning --prompt "Say hello in one word."

# 4. See why a request would route where it does
pig route explain --role reasoning

# 5. Benchmark the model
pig models benchmark qwen3-0.6b
```

## Benchmarking

`pig models benchmark <model-id>` runs a short fixed prompt and records the result
(load time, prompt/generation tokens-per-second, total latency, worker, backend,
timestamp) as a JSON line under `.pig_benchmarks/<model-id>.jsonl` (gitignored — this is
local runtime state, not something to commit). Records aren't currently pruned or
deduplicated by model file hash/backend version — treat older entries in that file as
historical, not automatically-invalidated, and compare timestamps yourself when a model
file, backend, or worker hardware changes.

## Cancellation and timeouts

- A job's lifecycle is `Queued -> Loading -> Running -> {Succeeded, Failed, Cancelled,
  TimedOut}`.
- `pig jobs cancel <job-id> --worker <id>` (or `POST /v1/jobs/{id}/cancel`)
  cooperatively cancels a queued or running job via a `CancellationToken`; the
  `llama-server` HTTP connection is dropped, which stops it from producing further
  output for that request.
- `requirements.maximum_execution_ms` on a `ModelRequest` (or the worker's configured
  `request_timeout_seconds` when unset) bounds how long a single generation may run;
  on expiry the job is marked `TimedOut` with a structured
  `ModelExecutionError::Timeout`.
- The worker's bounded queue (`max_queued_jobs`) rejects new submissions with HTTP 429
  once full, rather than growing without bound.

## Troubleshooting

- **"no [[workers]] configured"** — `pig models generate`/`route explain`/`jobs *`
  all require at least one `[[workers]]` entry in the resolved `pig.toml`.
- **`route explain` says "no candidate model for the requested role"** — check
  `[models.roles.<role>]` lists at least one candidate, and that candidate has a
  matching `[models.entries.<id>]` (remember to quote IDs containing a period).
- **`route explain` says "model 'X' is not known to this worker"** — the *worker's own*
  copy of `pig.toml` needs the same `[models.entries.*]`; workers don't hot-reload
  config, so restart the worker after editing it.
- **A "thinking" model (e.g. Qwen3) returns empty output at a low `max_tokens`** — the
  model spent its whole budget on chain-of-thought before reaching a final answer;
  raise `max_tokens` or use a smaller/non-reasoning model for short responses. pig
  captures both `content` and `reasoning_content` deltas so you'll see the reasoning
  text rather than nothing, but a very low budget can still cut it off mid-thought.
- **Worker fails to start on a non-loopback bind** — set `worker.auth.enabled = true`
  and `worker.auth.token_env`; this is enforced, not optional.

## Performance tuning

`LlamaCppExecutionConfig` (set per model, e.g. via a future `[models.entries.*]`
execution table, or passed directly to `/v1/models/load`'s `execution_config`) exposes
`context_size`, `cpu_threads`, `cpu_threads_batch`, `gpu_layers`, `batch_size`,
`micro_batch_size`, `flash_attention`, `mmap`, `mlock`. Only flags the installed
`llama-server` build actually advertises in `--help` are ever passed — unsupported
flags are silently skipped rather than causing a startup failure, so upgrading
llama.cpp never requires touching this configuration.

## Current limitations

- No distributed execution of one model across machines, no EXO integration, no
  automatic model downloads (all explicit non-goals for v0.5).
- Embeddings and reranking are not implemented by either backend; both report the
  capability as unsupported rather than pretending to work.
- Only one server process is supervised at a time per worker (one `llama-server` or
  one `mlx_lm.server`) — loading a different model stops the previous one. Run
  multiple workers (even on the same machine, on different ports) for true concurrent
  multi-model serving.
- The MLX backend does not support tool calling; `supports_tools` is reported as
  false and the scheduler will route tool-calling requests to llama_cpp workers.
- Benchmark records are not automatically invalidated by model/backend/hardware
  changes; compare timestamps yourself. `pig models load` runs an auto-benchmark
  on every successful load — benchmark data is fresh without any explicit invocation.
- Hardware discovery is real and tested on macOS; the Linux path
  (`/proc/cpuinfo`, `/proc/meminfo`) is implemented but was not validated against a
  real Linux machine in this development session — see the project's commit history
  for the exact state.
