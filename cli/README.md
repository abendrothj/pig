# pig CLI

Command-line interface for the pig Private Inference Gateway.

## Usage

```bash
cargo run --bin pig -- worker serve --config pig.toml
cargo run --bin pig -- workers list
cargo run --bin pig -- models generate --role reasoning --prompt "hello"
cargo run --bin pig -- coordinator serve --config pig.toml
```

See [docs/cli.md](../docs/cli.md) for all commands.
