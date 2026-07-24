# pig

```
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣤⣤⣶⣶⣶⣶⣦⣤⣄⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⢀⡶⢻⡦⢀⣠⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡟⢀⣴⣾⡿⠀⣠⠀⠀
⠀⠠⣬⣷⣾⣡⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⣌⣋⣉⣄⠘⠋⠀⠀
⠀⠀⠀⠀⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⣿⣿⡄⠀⠀⠀
⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣾⣿⣷⣶⡄⠀
⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀
⠀⠀⠀⠀⠸⣿⣿⣿⠛⠛⠛⠛⠛⠛⠛⠛⠻⠿⣿⣿⡿⠛⠛⠛⠋⠉⠉⠀⠀⠀
⠀⠀⠀⠀⠀⢻⣿⣿⠀⠀⢸⣿⡇⠀⠀⠀⠀⠀⢻⣿⠃⠸⣿⡇⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠈⠿⠇⠀⠀⠀⠻⠇⠀⠀⠀⠀⠀⠈⠿⠀⠀⠻⠿⠀⠀⠀⠀⠀⠀

  ██████╗ ██╗ ██████╗
  ██╔══██╗██║██╔════╝
  ██████╔╝██║██║  ███╗
  ██╔═══╝ ██║██║   ██║
  ██║     ██║╚██████╔╝
  ╚═╝     ╚═╝ ╚═════╝

  Private Inference Gateway
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Pig owns compute. You own intelligence.
```

![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)
![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-orange?logo=rust)

pig is a hardware-aware inference router for your own GPU machines. It schedules
generation requests across your workers, manages model loading, tracks throughput and
latency, and exposes an OpenAI-compatible API at the coordinator.

pig does not orchestrate agents, manage prompts, decompose tasks, or proxy external
APIs. Those belong in the caller. pig's only job is: *given this request, which
`(worker, model)` pair completes it fastest?*

Think Kubernetes for your GPUs, not LangChain.

## Architecture

```
your agent / OpenAI client
         │
         ▼
Coordinator (scheduler)         ← hard constraints + scoring
         │
         ├── Worker A  [CUDA]   llama.cpp
         ├── Worker B  [Metal]  mlx_lm
         └── Worker C  [CUDA]   llama.cpp
```

The coordinator applies hard constraints (context window, tool support, reasoning
capability, accelerator type, placement policy) then scores eligible placements by
load state, queue depth, throughput, and TTFT. The winning `(worker, model)` pair
gets the request. See [docs/architecture.md](docs/architecture.md) for internals.

## Install

**From a release tarball** (recommended):

```bash
tar xzf pig-0.5.0-aarch64-apple-darwin.tar.gz
cp pig-0.5.0-aarch64-apple-darwin/bin/pig /usr/local/bin/
```

**From source:**

```bash
cargo build --release
# binary at target/release/pig
```

## Quick start

```bash
# Start a worker (supervises llama-server or mlx_lm.server)
pig worker serve --config pig.toml

# Check registered workers
pig workers list

# Route a generation request to the best available worker
pig models generate --role reasoning --prompt "Explain backpressure in one paragraph."

# Start the OpenAI-compatible coordinator
pig coordinator serve --config pig.toml
```

See [docs/cli.md](docs/cli.md) for all commands and
[docs/local-inference.md](docs/local-inference.md) for worker and model configuration.

## Configuration

Copy `pig.toml.example` to `pig.toml` and fill in your worker URLs and model paths.

| Variable | Meaning |
|---|---|
| `PIG_CONFIG` | Path to config file (default: `./pig.toml`) |
| `RUST_LOG` | Logging filter (default: `warn`) |

## License

Apache-2.0 — see [LICENSE](LICENSE).
