# Changelog

All notable changes to pig will be documented here.

This project follows semantic versioning.

## [0.5.0] — 2026-07-23

### Added
- `POST /v1/pipeline` on the coordinator: sequential multi-step inference with
  per-step `role`, `requirements`, and `inject_previous` context threading. Each
  step runs the full scheduler independently, so steps can land on different workers.
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
