# Contributing to pig

Thanks for helping improve pig.

## Development Setup

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt -- --check
```

Use `RUST_LOG=info` or `RUST_LOG=debug` when debugging routing or worker behavior.

## Expectations

- Keep routing logic in `core`; HTTP and process management belongs in `worker`.
- Add or update tests for behavior changes, especially scheduling, job queuing, and coordinator routing.
- Do not commit `pig.toml` (gitignored) or local benchmark data.

## Pull Requests

- Explain the user-facing behavior change and why it is safe.
- Include test output for the relevant gates.
- Call out any config schema or API compatibility impact.
