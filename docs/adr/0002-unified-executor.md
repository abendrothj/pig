# ADR 0002: Unified Workflow Execution Path

## Status

Accepted

## Decision

Public workflow execution entry points delegate to the loop-capable callback runner. This removes divergent cache, retry, condition, and loop behavior between serial, callback, and CLI execution.

## Consequences

- `lao run` exercises the same execution semantics as the core parallel runner.
- Future serial execution should be implemented as an execution option around the shared step lifecycle, not as a second copy of step logic.
- Cache and event behavior should be changed in one place.
