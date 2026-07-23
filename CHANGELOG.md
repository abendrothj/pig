# Changelog

All notable changes to pig will be documented here.

This project follows semantic versioning.

## [0.5.0] — 2026-07-23

### Added
- **`ModelInstance` abstraction**: coordinator now exposes `GET /v1/instances` listing
  all known models across all workers as first-class inference resources, sorted by
  load state then benchmark throughput. Each instance carries `instance_id`, `loaded`,
  `context_tokens`, `accelerators`, and `benchmark` — making the scheduler's actual
  capacity view visible to API callers. Two workers running the same model are distinct
  instances.
- **Model-level `tool_calling` capability** on `ModelEntry`: set `tool_calling = true`
  or `false` in `[models.entries.*]` to override the backend's blanket default. The
  coordinator checks the model registry before the worker snapshot, so an MLX-served
  model that actually supports tools can be marked as such without changing the backend.
- **`reasoning` capability routing**: `reasoning = true` on a model entry marks it as
  supporting extended chain-of-thought (Qwen3 `/think`, DeepSeek R1, etc.). Requests
  with `requirements.reasoning = true` are hard-rejected for any model not tagged this
  way — the scheduler never silently falls back to a non-reasoning model.
- **Async auto-benchmark**: `pig models load` prints "Auto-benchmark running in
  background." and returns immediately; the benchmark runs in a detached thread and
  writes the record to `.pig_benchmarks/` without blocking the user.
- **MLX backend** (`backend = "mlx"`) for Apple Silicon: supervises `mlx_lm.server`
  (from the `mlx-lm` Python package), accepts HuggingFace model directories instead
  of GGUF files, reports `Metal` accelerator, and is selectable via
  `[worker.runtime.mlx] enabled = true`. Generation throughput advantage over
  llama.cpp varies by model size; larger models (7B+) typically benefit more.
- **Session affinity in pipelines**: `POST /v1/pipeline` accepts `"session_affinity": true`
  to pin all steps after the first to the worker that served step 0, keeping the
  computation on the same warm machine without exposing worker addresses to callers.
- **Auto-benchmark on load**: `pig models load` now automatically runs a short
  benchmark after a successful load, so the scheduler always has fresh TTFT and
  throughput data without any explicit `pig models benchmark` call.
- **TTFT scoring in the scheduler**: `BenchmarkSummary` gains `p50_ttft_ms`; the
  scheduler scores candidates with lower time-to-first-token higher, so workers
  with better latency are preferred when benchmark data is available.
- **Routing response headers**: `POST /v1/generate` includes `X-Pig-Worker-Id` and
  `X-Pig-Model-Id` headers on every response, so callers can inspect routing
  decisions without a separate `POST /v1/route` call.
- `execution_config` per model in `pig.toml`: backend-specific parameters
  (`gpu_layers`, `flash_attention`, `parallel`, `cache_type_k/v` for llama_cpp;
  `trust_remote_code`, `seed` for mlx) flow through to the subprocess at load time.
- `parallel` and `cache_type_k`/`cache_type_v` in `LlamaCppExecutionConfig` (KV
  cache quantization and concurrent request slots / continuous batching).
- `POST /v1/pipeline` on the coordinator: sequential multi-step inference with
  per-step `role`, `requirements`, and `inject_previous` context threading.
- `BenchmarkRecord` gains `p50_ttft_ms`, `p95_ttft_ms`, and
  `pipeline_acceptance_rate` fields (all optional, `#[serde(default)]` — existing
  records deserialize without changes).
- `GET /v1/models` now returns actual model IDs from the registry in addition to
  the stable role aliases (`pig-coding`, `pig-reasoning`, `pig-verification`).
- Release workflow builds `aarch64-apple-darwin` (Apple Silicon) in addition to
  `x86_64-apple-darwin` and `x86_64-unknown-linux-gnu`.
- CI now runs on pushes to `main` as well as pull requests.

### Fixed
- `loaded_models` and `available_memory_bytes` were always empty in coordinator
  worker snapshots. Workers now expose both fields in `/v1/capabilities` and the
  coordinator extracts them on every probe. The +100 hot-model scheduling bonus
  and the `minimum_available_memory_bytes` hard constraint now have real data.
- `minimum_available_memory_bytes` in `ModelRequirements` was never enforced by
  `check_hard_constraints()` — the field existed but was never read.
- `workers health` made 4N HTTP requests (two full snapshot passes) instead of 2N.
- Coordinator sync and async snapshot builders were ~150-line duplicates with
  diverging field handling; collapsed into shared `parse_snapshot()` helper.
- Clippy warning: `.clone()` on `Copy` type `Option<AcceleratorKind>` in
  `llama_cpp.rs`.
- `pig.toml.example` profile block used wrong field name (`url` instead of
  `coordinator_url`) and was missing `mode = "remote"`.
- CLI `--help` reported "Local AI Orchestrator" instead of "Private Inference Gateway".

### Removed
- `LocalWithFallback` profile variant (dead code; behaviorally identical to `Remote`).
- LAO-era dead code from `cross_platform.rs`: `PathUtils`, `EnvUtils`, plugin/config/
  cache directory helpers.
- Unused `Artifact` variants: `Integer`, `Float`, `Boolean`, `File`, `FileSet`,
  `CommandResult`. Only `Null`, `Text`, and `Json` are produced or consumed.

## [0.1.0] — 2026-07-22

Initial release as pig (Private Inference Gateway).

- OpenAI-compatible coordinator gateway (`pig coordinator serve`)
- Worker management: serve, install/uninstall as systemd service, start/stop/status/logs
- Model registry: list, inspect, load/unload, generate, benchmark
- Constraint-based scheduling across llama.cpp workers (hard constraints + scoring)
- Worker telemetry and metrics API
- Remote coordinator profiles via `pig.toml`
- SSE streaming from workers proxied through coordinator
- Tool call support (structured, not stringified)
- `ReasoningMode` auto-resolution for Qwen3-style `/think` / `/no_think` tokens
