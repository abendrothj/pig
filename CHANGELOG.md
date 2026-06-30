# Changelog

All notable changes to LAO will be documented here.

This project follows semantic versioning for the CLI/core crates and treats `lao_plugin_api` as an external compatibility contract. Breaking ABI or workflow schema changes require a major version bump once the first stable release is cut.

## Unreleased

## 0.3.0

### Plugin ABI v2 (structured results)

- `lao_plugin_api` now advertises ABI version 2 and exposes a structured result
  channel (`PluginResult` + `run_structured`/`free_result`) appended to the vtable.
- The host negotiates the ABI per plugin and remains backward compatible: v1 plugins
  still load and run through the legacy text channel (`LAO_PLUGIN_ABI_MIN_SUPPORTED = 1`).
- Vtable fields are read via raw, version-gated pointer access so a shorter v1 vtable
  can never be over-read.
- `EchoPlugin` implements native v2 status codes; the other bundled plugins migrate via
  the new `lao_structured_adapter!` macro.

### Execution engine

- Extracted a shared `StepExecutor` kernel that owns the full per-step lifecycle
  (conditions, parameter wiring, loops, caching, retries, trust gating, events, logging).
- Serial and parallel orchestration now route through the same kernel via
  `run_workflow_with_options`, so `ExecutionOptions { parallel, record_state, state_dir }`
  is honored for real (no more "serial not implemented" fallback).

### Trust

- Trust enforcement is now manifest-driven: a plugin is gated by the capability classes
  declared in its manifest, reconciled against the policy before any step runs — not by a
  hardcoded plugin-name list.

### Scheduler

- `run-due` now takes an advisory lock file in the state directory so overlapping cron
  ticks cannot double-run scheduled workflows; stale locks are reclaimed automatically.

### Prior hardening (carried from 0.2.x work)

- Workflow schema rejects unsupported control-flow and modality fields.
- CLI/core execution routes through the loop-capable workflow runner.
- Core plugin calls validate inputs and reclaim host-owned FFI input memory.
- Trust policy denies dangerous plugin capabilities by default.
