# LAO Workflows

## Workflow YAML Format
A workflow is a list of steps, each specifying a plugin to run, its inputs, and optional config.

```yaml
workflow: "Summarize Meeting"
steps:
  - run: WhisperPlugin
    input: "meeting.wav"
    retries: 3
    retry_delay: 1000
    cache_key: "whisper_meeting"
  - run: SummarizerPlugin
    input_from: step1
    cache_key: "summary_meeting"
```

## Prompt-Generated Workflows
- Use the CLI or UI to generate workflows from natural language prompts
- Example:
  ```bash
  lao prompt "Refactor this Rust file and add comments"
  ```
  Output:
  ```yaml
  workflow: "Rust Refactor"
  steps:
    - run: EchoPlugin
      input: "main.rs"
    - run: SummarizerPlugin
      input_from: step1
  ```

## Parallel Execution

LAO supports parallel execution of independent workflow steps, significantly improving performance for workflows with multiple independent branches.

### How It Works

1. **Execution Levels**: Steps are automatically grouped into execution levels based on dependencies
2. **Level-Based Execution**: Steps at the same level run concurrently; levels execute sequentially
3. **Primary Input vs Dependencies**:
   - `input_from`: The primary input source for the step
   - `depends_on`: Additional dependencies that must complete first

### Example: Parallel Processing

```yaml
workflow: "Parallel Processing Example"
steps:
  # Level 0: Three independent steps run in parallel
  - run: EchoPlugin
    input: "Process A"
  
  - run: EchoPlugin
    input: "Process B"
  
  - run: EchoPlugin
    input: "Process C"

  # Level 1: Merge step waits for all three, uses step1 as primary input
  - run: EchoPlugin
    input_from: step1  # Primary input
    depends_on: ["step2", "step3"]  # Parallel dependencies
```

### Running Parallel Workflows

Use `run_workflow_yaml_parallel_with_callback` from the core crate when you need
level-based parallel execution and structured step events.

### Performance Metrics

When running in parallel mode, LAO tracks:
- **Execution levels**: Number of dependency levels
- **Max parallelism**: Maximum concurrent steps
- **Average parallelism**: Average concurrent steps
- **Speedup**: Time saved vs sequential execution

### Best Practices

1. **Use parallel execution** when steps are independent or can run concurrently
2. **Use sequential execution** for debugging or when strict ordering is required
3. **Set primary input** correctly: the step that provides the main data flow
4. **Use `depends_on`** for steps that must complete but don't provide input

## Advanced Features

- **Conditional/Branching Steps**: output/status/error-based conditions and loops ✅
- **Parallel Execution**: Level-based concurrent execution ✅
- **Parameter Injection**: Securely pass secrets, user data, etc.

## Contributing Workflows
- Add new prompt/workflow pairs to the prompt library for validation and LLM tuning
- See `workflows/test_parallel.yaml` for a parallel workflow example.