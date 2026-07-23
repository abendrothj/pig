# LAO CLI

This is the dedicated command-line interface for the LAO Orchestrator.

## Usage

```
cargo run --bin pig run workflows/test.yaml
cargo run --bin pig validate workflows/test.yaml
cargo run --bin pig plugin-list
cargo run --bin pig new-workflow myflow --output workflows/myflow.yaml
cargo run --bin pig prompt "Summarize this audio" --output workflows/audio_summary.yaml
cargo run --bin pig list-workflows
cargo run --bin pig view-workflow myflow
cargo run --bin pig delete-workflow myflow
```

## Commands
- `run <workflow.yaml> [--dry-run]`  
  Run a workflow. Use `--dry-run` to simulate execution.
- `validate <workflow.yaml>`  
  Validate workflow structure and plugin availability.
- `plugin-list`  
  List all available plugins and their IO signatures.
- `new-workflow <name> [--output <file>]`  
  Scaffold a new workflow YAML template. Optionally specify output file path.
- `prompt <prompt> [--output <file>]`  
  Generate a workflow from a prompt and save to a file (default: workflows/generated_from_prompt.yaml).
- `list-workflows`  
  List all workflow YAML files in the workflows/ directory.
- `view-workflow <name>`  
  Print the contents of a workflow YAML file from the workflows/ directory.
- `delete-workflow <name>`  
  Delete a workflow YAML file from the workflows/ directory.
- `validate-prompts [--path <json>] [--fail-fast] [--verbose]`  
  Validate prompt-to-workflow generation using the prompt library. 