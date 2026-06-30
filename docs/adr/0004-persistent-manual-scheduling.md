# ADR 0004: Persistent Manual Scheduling

## Status

Accepted

## Decision

Scheduling is part of the v1 production scope, but LAO does not run a background daemon. Schedules persist to workflow state, reload on startup, and are executed manually through `run-due`.

## Consequences

- `schedule`, `list-scheduled`, `unschedule`, `status`, and `run-due` share persisted state.
- Operators can integrate `run-due` with cron, launchd, systemd timers, or other schedulers.
- A future daemon can build on the same persisted state model.
