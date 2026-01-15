# LAO CLI Documentation

## Usage
```
lao <COMMAND> [OPTIONS]
```

## Commands
- `run <workflow.yaml> [--dry-run]`  
  Run a workflow. Use `--dry-run` to simulate execution and show expected IO types.
- `validate <workflow.yaml>`  
  Validate workflow structure, types, and plugin availability.
- `plugin-list`  
  List all available plugins, their IO signatures, and descriptions.
- `prompt <prompt>`  
  Generate and run a workflow from a natural language prompt using the local LLM.
- `validate-prompts [--path <json>] [--fail-fast] [--verbose]`  
  Validate prompt-to-workflow generation using the prompt library.
- (Planned) `explain plugin <name>`  
  Show detailed info and examples for a plugin.

## Execution Modes

### Sequential Execution
- Steps execute one at a time, even if they're independent
- Use for debugging, testing, or when strict ordering is required
- Enabled via `--debug` flag or Debug mode in UI

### Parallel Execution (Default)
- Automatically detects and executes independent steps concurrently
- Steps are grouped into execution levels based on dependencies
- Steps within the same level run in parallel; levels execute sequentially
- Significantly faster for workflows with independent branches
- Enabled automatically when workflow has parallelizable steps

### UI Execution
- The desktop UI automatically detects parallelism and uses the appropriate execution mode
- Single "Run" button changes to "Run (Parallel)" when parallel execution will be used
- Debug mode checkbox forces sequential execution for debugging
- Real-time event streaming shows step progress and execution levels

## Examples
```
lao run workflows/test.yaml
lao run workflows/test.yaml --dry-run
lao validate workflows/test.yaml
lao plugin-list
lao prompt "Summarize this audio and tag action items"
lao validate-prompts --path core/prompt_dispatcher/prompt/prompt_library.json --verbose
``` 