# ADR 0005: Out-of-Process Plugin Host

## Status

Proposed (design only — not implemented in ABI v1)

## Context

LAO v1 runs plugins in-process via `libloading` and the ABI v1 vtable. Trust policy reduces blast radius but cannot isolate memory corruption, malicious native code, or privilege escalation within the host process.

## Decision

Introduce an out-of-process plugin host in a future ABI v2 milestone while keeping ABI v1 in-process plugins supported during migration.

### Process protocol (draft)

1. Host spawns a `lao-plugin-host` worker per plugin invocation (or long-lived pool per plugin type).
2. Request envelope (JSON over stdin or Unix socket):
   - `abi_version`, `plugin_name`, `plugin_path`, `input` (text or structured blob ref), `timeout_ms`, `trust_token`
3. Response envelope:
   - `status`: `success | validation_failed | runtime_error | timeout`
   - `output`: optional text
   - `error`: optional message
4. Worker loads the dylib in an isolated process, calls v1 `run`, returns structured JSON (no `error:` prefix convention at the host boundary).

### Timeouts and cancellation

- Default step timeout: 5 minutes (configurable per workflow step).
- Host sends SIGTERM; escalate to SIGKILL after grace period.
- Partial output from timed-out runs is discarded.

### Trust migration

- In-process plugins continue to honor `lao.toml` trust policy.
- Out-of-process workers inherit a reduced capability set derived from the same policy (filesystem roots, network endpoints).
- `allow_plugins` grants both in-process and OOP execution for named plugins.

### Rollout

1. Ship structured host-side `PluginRunResult` (done in v0.2.x).
2. Document ABI v2 structured results in `docs/plugin-api.md`.
3. Implement OOP host behind `LAO_PLUGIN_MODE=process` feature flag.
4. Deprecate in-process loading for untrusted third-party plugins in a major release.

## Consequences

- Higher per-step latency and packaging complexity.
- Stronger isolation story for external contributors and LLM-generated workflows.
- ABI v1 plugins remain usable during transition without breaking existing builds.
