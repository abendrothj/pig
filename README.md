# pig — Private Inference Gateway

![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)
![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-orange?logo=rust)

pig is an OpenAI-compatible inference gateway that routes generation requests
across private llama.cpp workers over LAN. Runs entirely on local hardware —
no cloud, no external APIs.

## What it does

- Routes requests to the right worker based on hardware constraints and model role.
- Supervises `llama-server` subprocesses, manages job queues, and reports telemetry.
- Exposes an OpenAI-compatible `/v1/chat/completions` endpoint at the coordinator.
- Installs workers as systemd services on Linux.

```
pig coordinator serve   →   routes to   →   pig worker serve (llama.cpp)
    (OpenAI gateway)                         (macOS/Metal or Linux/CUDA)
```

See [docs/architecture.md](docs/architecture.md) for internals.

## Build

```bash
cargo build --release
```

## Quick start

```bash
# Start a worker (manages llama-server, exposes HTTP API)
pig worker serve --config pig.toml

# List registered workers and their loaded models
pig workers list

# Send a generation request to the best available worker
pig models generate --role reasoning --prompt "Explain backpressure in one paragraph."

# Route via the OpenAI-compatible coordinator
pig coordinator serve --config pig.toml
```

See [docs/cli.md](docs/cli.md) for all commands and [docs/local-inference.md](docs/local-inference.md)
for worker/model configuration.

## Configuration

Copy `pig.toml.example` to `pig.toml` and fill in your worker URLs and model paths.

| Variable | Meaning |
|---|---|
| `PIG_CONFIG` | Path to config file (default: `./pig.toml`) |
| `RUST_LOG` | Logging filter (default: `warn`) |

## License

Apache-2.0 — see [LICENSE](LICENSE).
