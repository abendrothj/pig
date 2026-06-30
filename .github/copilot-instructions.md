# LAO: Local AI Workflow Orchestrator

LAO is a lean Rust workspace for running local AI workflows through a dynamic
plugin ABI. The current project is headless: a core library, CLI, plugin API,
bundled plugins, docs, scripts, and workflow examples.

## Repository Layout

- `core/`: DAG engine, workflow execution, scheduling state, caching, prompt validation, and plugin loading.
- `cli/`: `lao-cli` command-line interface for running, validating, scaffolding, and inspecting workflows.
- `lao_plugin_api/`: Stable C ABI shared by the host and plugin crates.
- `plugins/`: Bundled plugins including `EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`, `PromptDispatcherPlugin`, `FileReadPlugin`, `FolderMapPlugin`, `JsonExtractPlugin`, `RegexExtractPlugin`, `ShellCommandPlugin`, and `MarkdownReportPlugin`.
- `workflows/`: YAML workflow examples.
- `docs/`: Architecture, CLI, and workflow documentation.

## Build And Validate

```bash
cargo check --workspace
cargo fmt -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
bash scripts/build-plugins.sh
```

Use longer timeouts for `cargo test --workspace`, release builds, and sanitizer
runs. They may take several minutes in CI.

## CLI Usage

```bash
cargo run --bin lao-cli -- plugin-list
cargo run --bin lao-cli -- validate workflows/test_loop.yaml
cargo run --bin lao-cli -- run workflows/test_loop.yaml
cargo run --bin lao-cli -- new-workflow demo --output workflows/demo.yaml
cargo run --bin lao-cli -- validate-prompts
```

Workflow YAML should use full plugin names, for example `EchoPlugin` rather than
`Echo`. Step dependencies use generated step ids such as `step1` and `step2`.

## Plugin Notes

Plugins are shared libraries loaded from `plugins/` or from a plugin
subdirectory's `target/release/`. Each plugin should export the expected
`plugin_vtable` symbol and include a `plugin.yaml` manifest for CLI inspection.

Use `lao_plugin_api` and `plugins/EchoPlugin` as the reference implementation
when adding new plugins.

## Production Rules

- Keep workflow schema fields executable or reject them explicitly.
- Route plugin execution through the core plugin host, not raw vtable calls.
- Respect `TrustPolicy`; dangerous filesystem, shell, network, and subprocess plugins must be explicitly allowed.
- `lao_plugin_api` is an external compatibility contract. ABI layout or ownership changes require documented versioning impact.