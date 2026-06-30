# LAO CLI Documentation

## Usage
```
lao <COMMAND> [OPTIONS]
```

## Commands
- `run <workflow.yaml> [--dry-run]`  
  Run a workflow. Use `--dry-run` to validate plugin availability without executing steps.
- `validate <workflow.yaml>`  
  Validate workflow structure, types, and plugin availability.
- `plugin-list`  
  List all dynamically loaded plugins.
- `new-workflow <name> [--output <file>]`  
  Scaffold a starter workflow YAML file.
- `prompt <prompt> [--output <file>]`  
  Generate a workflow from a natural language prompt using `PromptDispatcherPlugin`.
- `validate-prompts [--path <json>] [--fail-fast] [--verbose]`  
  Validate prompt-to-workflow generation using the prompt library.
- `list-workflows`, `view-workflow <name>`, `delete-workflow <name>`  
  Manage workflow YAML files under `workflows/`.
- `explain-plugin <name>`  
  Show manifest details and examples for a bundled plugin.
- `schedule`, `unschedule`, `list-scheduled`, `run-due`, `status`, `cleanup`  
  Manage persisted workflow schedule and execution state metadata. `run-due`
  manually executes due enabled schedules; LAO does not run a background daemon.

## Logging

Set `RUST_LOG` to control CLI/core diagnostics:

```bash
RUST_LOG=info lao run workflows/test_loop.yaml
RUST_LOG=debug lao plugin-list
```

## Execution Modes

### Sequential Execution
- Steps execute one at a time, even if they're independent
- Use for debugging, testing, or when strict ordering is required
- Use the sequential workflow runner APIs when embedding the core crate.

### Parallel Execution
- Automatically detects and executes independent steps concurrently
- Steps are grouped into execution levels based on dependencies
- Steps within the same level run in parallel; levels execute sequentially
- Available through `run_workflow_yaml_parallel_with_callback` in the core crate.

## Examples
```
lao run workflows/test_loop.yaml
lao run workflows/test_loop.yaml --dry-run
lao validate workflows/test_loop.yaml
lao plugin-list
lao prompt "Summarize this audio and tag action items"
lao validate-prompts --path core/prompt_dispatcher/prompt/prompt_library.json --verbose
``` 