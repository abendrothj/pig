# LAO Workflows

## Workflow YAML Format
A workflow is a list of steps, each specifying a plugin to run, its inputs, and optional config.

```yaml
workflow: "Summarize Meeting"
steps:
  - run: Whisper
    input: "meeting.wav"
    retry_count: 3
    retry_delay: 1000
    cache_key: "whisper_meeting"
  - run: Summarizer
    input_from: Whisper
    cache_key: "summary_meeting"
  - run: Tagger
    input_from: Summarizer
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
    - run: RustRefactor
      input: "main.rs"
    - run: CommentGenerator
      input_from: RustRefactor
  ```

## Parallel Execution

LAO supports parallel execution of independent workflow steps, significantly improving performance for workflows with multiple independent branches.

### How It Works

1. **Execution Levels**: Steps are automatically grouped into execution levels based on dependencies
2. **Level-Based Execution**: Steps at the same level run concurrently; levels execute sequentially
3. **Primary Input vs Dependencies**: 
   - `input_from`: The primary input source (green edge in UI)
   - `depends_on`: Parallel dependencies that must complete first (purple edges in UI)

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

### Visual Indicators in UI

- **Green edges**: Primary input (`input_from`) - curved for better visualization
- **Purple edges**: Parallel dependencies (`depends_on`) - curved for better visualization
- **Purple dot** (top-left): Fan-in node (multiple inputs)
- **Blue dot** (bottom-right): Fan-out node (multiple outputs)
- **L# label**: Execution level indicator
- **Level bands**: Subtle background colors showing execution levels (alternating blue/purple tints)
- **Level labels**: "Level 0", "Level 1", etc. displayed on the left side of each execution level
- **Hierarchical layout**: Nodes automatically arranged vertically by execution level, horizontally within levels

### Auto-Layout Feature

LAO includes an **Auto-Layout** feature that automatically arranges workflow nodes hierarchically:

- **Automatic on load**: When you load a workflow, nodes are automatically arranged by execution level
- **Manual trigger**: Click the **"📐 Auto-Layout"** button to rearrange nodes at any time
- **Hierarchical structure**: 
  - Nodes are positioned vertically by execution level (top to bottom)
  - Nodes at the same level are aligned horizontally (side by side)
  - Proper spacing ensures clear visualization of parallel execution
- **Pan and drag**: Level labels and bands move with the graph when panning/dragging
- **Dynamic height**: Graph area expands to show all execution levels

This makes complex workflows with multiple parallel branches much easier to understand and navigate!

### Running Parallel Workflows

The UI automatically detects if your workflow has parallelizable steps and uses the appropriate execution mode:

- **Single "Run" Button**: Automatically detects parallelism
  - Shows "▶️ Run (Parallel)" in purple when parallel execution will be used
  - Shows "▶️ Run" in blue for sequential execution
  - Tooltip explains the execution mode

- **Debug Mode**: Optional checkbox "🐛 Debug" forces sequential execution
  - Use for debugging, testing, or step-by-step execution
  - Overrides automatic parallel detection

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

- **Conditional/Branching Steps**: if/else, loops, parameterized flows ✅
- **Parallel Execution**: Level-based concurrent execution ✅
- **Parameter Injection**: Securely pass secrets, user data, etc.
- **Multi-modal Input**: Files, voice, etc.

## Contributing Workflows
- Add new prompt/workflow pairs to the prompt library for validation and LLM tuning
- See `workflows/parallel_example.yaml`, `parallel_fan_in.yaml`, and `parallel_fan_out.yaml` for parallel workflow examples 