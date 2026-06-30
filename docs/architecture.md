# LAO Architecture Overview

## Core Components

- **DAG Engine**: Executes workflows as directed acyclic graphs, handling step dependencies, retries, caching, and lifecycle hooks.
  - Supports optional parallel execution per DAG level and emits structured step events for CLI or library consumers.
- **Plugin System**: Local AI tasks are packaged as shared libraries built against the `lao_plugin_api` C ABI. Plugins are loaded dynamically from the `plugins/` directory and declare IO types and lifecycle hooks.
- **PromptDispatcherPlugin**: Uses a local LLM (Ollama) and a system prompt file to generate workflows from natural language prompts. Hot-swappable prompt at `core/prompt_dispatcher/prompt/system_prompt.txt`.
- **Prompt Library & Validation**: Prompts and expected workflows in Markdown/JSON, validated by a test harness and CLI command.
- **CLI**: Command-line interface for running, validating, scaffolding, scheduling, and inspecting workflows and plugins. Supports prompt-driven generation and validation.

## Agentic Workflow Generation
- User enters a prompt through the CLI or an embedding application.
- PromptDispatcherPlugin uses LLM + system prompt to generate YAML workflow
- Workflow is parsed, validated, and executed as a DAG

## Prompt Validation/Test Harness
- Loads prompt library
- Runs each prompt through the dispatcher
- Compares generated workflow to expected output (structure-aware)
- CLI and test harness for validation

## Data Flow

1. User defines a workflow YAML (steps, dependencies, config).
2. CLI or library consumer loads and validates the workflow.
3. DAG engine builds the execution graph.
4. Each step:
   - Checks cache (if enabled)
   - Runs plugin (with retries/lifecycle hooks)
   - Logs output, errors, and status
5. CLI or library consumer displays results and logs.

## Extensibility
- Add new plugins by implementing the `lao_plugin_api` ABI, building as a shared library, and placing the library in the `plugins/` directory or a plugin subdirectory.
- Extend CLI with new commands via Clap.