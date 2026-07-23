# lao CLI

## Usage

```
lao <COMMAND> [OPTIONS]
```

## Worker

```bash
lao worker serve [--config lao.toml]    # start the worker daemon (foreground)
lao worker install [--config lao.toml]  # install as a systemd service (Linux)
lao worker uninstall [--purge-user]
lao worker start | stop | restart | status
lao worker logs [--follow] [--lines N]
```

## Workers

Inspect configured workers from the coordinator side.

```bash
lao workers list [--json]
lao workers inspect <worker-id> [--json]
lao workers health [--json]                   # non-zero exit if any worker is unhealthy
lao workers metrics [<worker-id>] [--json]    # omit worker-id for aggregate
```

## Models

```bash
lao models list [--json]
lao models inspect <model-id> [--json]
lao models discover --directory <path>        # scan for GGUF files, does not write config
lao models load <model-id> [--worker <id>]
lao models unload <model-id> [--worker <id>]
lao models generate --prompt "..." \
    [--role <role> | --model <id>] \
    [--system "..."] [--max-tokens N] [--temperature F] \
    [--stream] [--json] [--force-worker <id>] [--force-cpu]
lao models benchmark <model-id> [--worker <id>] [--json]
```

`--stream` prints tokens as they arrive. Without it, `models generate` waits for the full response. `--json` emits the complete structured `ModelResponse`.

## Route

```bash
lao route explain [--role <role> | --model <id>] [--json]
```

Shows which worker and model would be selected for a request, and why — including which workers were rejected and the reason.

## Jobs

```bash
lao jobs list --worker <id> [--json]
lao jobs inspect <job-id> --worker <id> [--json]
lao jobs cancel <job-id> --worker <id>
```

## Coordinator

```bash
lao coordinator serve \
    [--config lao.toml] \
    [--bind 0.0.0.0:3001] \
    [--auth-token-env VAR]
```

Starts the coordinator as a persistent HTTP service exposing the OpenAI-compatible API at `/v1/chat/completions` and `/v1/models`. See `docs/openai-compatibility.md`.

## Profile selection

```bash
lao --profile remote models list    # use [profiles.remote] from lao.toml
```

## Logging

```bash
RUST_LOG=info lao workers health
RUST_LOG=debug lao models generate --role reasoning --prompt "hello"
```
