# pig CLI

## Usage

```
pig <COMMAND> [OPTIONS]
```

## Worker

```bash
pig worker serve [--config pig.toml]    # start the worker daemon (foreground)
pig worker install [--config pig.toml]  # install as a systemd service (Linux)
pig worker uninstall [--purge-user]
pig worker start | stop | restart | status
pig worker logs [--follow] [--lines N]
```

## Workers

Inspect configured workers from the coordinator side.

```bash
pig workers list [--json]
pig workers inspect <worker-id> [--json]
pig workers health [--json]                   # non-zero exit if any worker is unhealthy
pig workers metrics [<worker-id>] [--json]    # omit worker-id for aggregate
```

## Models

```bash
pig models list [--json]
pig models inspect <model-id> [--json]
pig models discover --directory <path>        # scan for GGUF files, does not write config
pig models load <model-id> [--worker <id>]
pig models unload <model-id> [--worker <id>]
pig models generate --prompt "..." \
    [--role <role> | --model <id>] \
    [--system "..."] [--max-tokens N] [--temperature F] \
    [--stream] [--json] [--force-worker <id>] [--force-cpu]
pig models benchmark <model-id> [--worker <id>] [--json]
```

`--stream` prints tokens as they arrive. Without it, `models generate` waits for the full response. `--json` emits the complete structured `ModelResponse`.

## Route

```bash
pig route explain [--role <role> | --model <id>] [--json]
```

Shows which worker and model would be selected for a request, and why — including which workers were rejected and the reason.

## Jobs

```bash
pig jobs list --worker <id> [--json]
pig jobs inspect <job-id> --worker <id> [--json]
pig jobs cancel <job-id> --worker <id>
```

## Coordinator

```bash
pig coordinator serve \
    [--config pig.toml] \
    [--bind 0.0.0.0:3001] \
    [--auth-token-env VAR]
```

Starts the coordinator as a persistent HTTP service exposing `GET /v1/models`, `POST /v1/chat/completions`, and `POST /v1/pipeline`. See `docs/openai-compatibility.md`.

## Profile selection

```bash
pig --profile remote models list    # use [profiles.remote] from pig.toml
```

## Logging

```bash
RUST_LOG=info pig workers health
RUST_LOG=debug pig models generate --role reasoning --prompt "hello"
```
