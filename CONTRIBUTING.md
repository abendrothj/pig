# Contributing to LAO

Thanks for helping improve LAO. This project treats workflow execution and plugin loading as production infrastructure, so changes should be small, tested, and explicit about trust boundaries.

## Development Setup

```bash
cargo build --workspace
bash scripts/build-plugins.sh
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt -- --check
```

Use `RUST_LOG=info` or `RUST_LOG=debug` when debugging CLI/plugin loading behavior.

## Expectations

- Keep public workflow schema fields honest. Do not add YAML fields until the engine executes or rejects them intentionally.
- Route plugin calls through the core plugin host; do not call vtables directly from CLI or tests unless testing the ABI boundary.
- Add or update tests for behavior changes, especially execution order, caching, retries, trust policy, and scheduler persistence.
- Do not commit runtime output from `cache/`, `workflow_states/`, or local plugin build artifacts.

## Pull Requests

- Explain the user-facing behavior change and why it is safe.
- Include test output for the relevant gates.
- Call out any plugin ABI, trust policy, or workflow schema compatibility impact.
