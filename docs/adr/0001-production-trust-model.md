# ADR 0001: Production Trust Model

## Status

Accepted

## Decision

LAO keeps the trusted in-process plugin host for v1 and adds an explicit default-deny `TrustPolicy` for dangerous capabilities. Filesystem, shell, network, and subprocess-capable plugins require opt-in configuration through `lao.toml` or a compatibility escape hatch where documented.

## Consequences

- Safe text-transform workflows remain easy to run.
- Generated or third-party workflows cannot silently invoke dangerous capabilities.
- Allowed plugins still run with host process privileges; out-of-process isolation remains a future ABI milestone.
