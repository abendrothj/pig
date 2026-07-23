# pig: Private Inference Gateway

pig is a Rust workspace for routing inference requests across private llama.cpp
workers. It exposes an OpenAI-compatible HTTP gateway and manages worker
lifecycle via systemd.

## Repository Layout

- `core/`: Shared types — `ModelRequest`, `ModelResponse`, `ModelRole`,
  `PlacementPolicy`, `Artifact`, model registry, scheduler logic.
- `worker/`: Worker HTTP API, job queue, llama-server supervision, coordinator
  server (OpenAI gateway). Published as `pig-worker`.
- `cli/`: `pig` command-line interface. Commands: worker, workers, models, route,
  jobs, coordinator.

## Build And Validate

```bash
cargo check --workspace
cargo fmt -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## CLI Usage

```bash
cargo run --bin pig -- worker serve --config pig.toml
cargo run --bin pig -- workers list
cargo run --bin pig -- models generate --role reasoning --prompt "hello"
cargo run --bin pig -- coordinator serve --config pig.toml
```

## Production Rules

- Keep routing logic in `core`; keep HTTP/process management in `worker`.
- The coordinator server routes by `pig-<role>` model alias or direct worker URL.
- Worker installs target Linux/systemd; Mac is dev-only (`worker serve`).
