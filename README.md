# LAO — Local AI Workflow Orchestrator

![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)
![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-orange?logo=rust)

LAO is a headless engine and CLI for chaining local plugins into DAG-based
workflows. Plugins are dynamically loaded shared libraries built against a small,
stable C ABI; workflows are plain YAML. Everything runs locally and offline.

## What it does

- Loads plugins at runtime by `dlopen`-ing shared libraries (`.so`/`.dylib`/`.dll`)
  that export a known C vtable.
- Parses a YAML workflow into a DAG, validates it (plugin availability + I/O type
  compatibility), topologically sorts it, and executes it.
- Supports parallel execution by dependency level, per-step retries with backoff,
  on-disk output caching, output-based conditions, and `for_each` loops.

```
YAML workflow → build_dag → topo_sort → PluginRegistry (dlopen) → run each step
```

See [docs/architecture.md](docs/architecture.md) for internals.

## Build

```bash
# Engine + CLI
cargo build --release

# Plugins (built in-place; the registry discovers them from each
# plugin's target/release/ or from the plugins/ directory)
bash scripts/build-plugins.sh
```

## Quick start

```bash
# List the plugins the registry can load
./target/release/lao-cli plugin-list

# Validate, then run a workflow
./target/release/lao-cli validate workflows/test_loop.yaml
./target/release/lao-cli run workflows/test_loop.yaml
```

A workflow is a list of steps. Steps are ordered by their data/dependency edges;
a step references an upstream step's output by its positional id (`step1`, `step2`, …):

```yaml
workflow: "Audio Transcription & Summary"
steps:
  - run: WhisperPlugin          # step1
    input: "meeting.wav"
    cache_key: "whisper_transcription"
    retries: 2
    retry_delay: 1000

  - run: SummarizerPlugin       # step2
    input_from: step1           # receives step1's output as its input
    cache_key: "summary"
```

See [docs/workflows.md](docs/workflows.md) for the full schema (conditions, loops,
parallel levels, caching) and [docs/cli.md](docs/cli.md) for all commands.

## Bundled plugins

| Plugin | Purpose |
|---|---|
| `EchoPlugin` | Returns its input; reference implementation of the ABI |
| `WhisperPlugin` | Transcribes audio by shelling out to a local `whisper.cpp` build |
| `SummarizerPlugin` | Summarizes text via a local HTTP inference endpoint |
| `PromptDispatcherPlugin` | Generates a workflow YAML from a natural-language prompt |
| `FileReadPlugin` | Reads a file from disk and returns its contents |
| `FolderMapPlugin` | Recursively lists files under a directory |
| `JsonExtractPlugin` | Extracts a value from JSON via a `$.a.b[0]` selector |
| `RegexExtractPlugin` | Returns regex matches from text (first input line is the pattern) |
| `ShellCommandPlugin` | Runs a trusted shell command (gated by `LAO_ALLOW_SHELL=1`) |
| `MarkdownReportPlugin` | Formats text into a Markdown report, optionally writing it to disk |

Writing your own plugin is a matter of implementing the C vtable — see
`lao_plugin_api` and the bundled plugins for examples.

## Configuration

| Variable | Meaning |
|---|---|
| `LAO_PLUGINS_DIR` | Directory the registry scans for plugins (default: `plugins/`) |
| `LAO_CACHE_DIR` | Directory for cached step outputs (default: `cache/`) |
| `WHISPER_CPP_PATH` | Path to the `whisper.cpp` build used by `WhisperPlugin` |
| `LAO_ALLOW_SHELL` | Set to `1` to allow `ShellCommandPlugin` to execute commands |

## Notes & limitations

- **Plugins run in-process with full host privileges.** They are loaded via
  `dlopen` with no sandbox — only load plugins you trust.
- **Step success is determined by convention:** a step is considered failed if its
  output is empty or begins with `error:`. The ABI has no separate status channel.
- **`schedule`/`unschedule`/`list-scheduled` persist schedule metadata only.** There
  is no background daemon; runs are triggered manually with `run`.

## License

MIT — see [LICENSE](LICENSE).
