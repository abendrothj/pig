# Parallel vs Sequential Execution in LAO

## Overview

LAO automatically determines whether workflow steps can run in parallel based on the **workflow structure** (dependencies defined by `input_from` and `depends_on`). You don't need to manually choose - the system analyzes your workflow and uses parallel execution when possible.

## How It Works

### Workflow Structure Determines Execution Mode

The workflow YAML specification defines dependencies between steps:

```yaml
workflow: "Example"
steps:
  - run: EchoPlugin
    input: "Step 1"
    # No dependencies - can start immediately
  
  - run: EchoPlugin
    input: "Step 2"
    # No dependencies - can start immediately (parallel with step 1!)
  
  - run: EchoPlugin
    input_from: step1  # Depends on step1 output
    depends_on: ["step2"]  # Also waits for step2 (but doesn't use its output)
    # Must wait for both step1 AND step2 to complete
```

### Execution Levels

LAO groups steps into **execution levels**:

- **Level 0**: Steps with no dependencies (run first, can run in parallel)
- **Level 1**: Steps that depend on Level 0 (run after Level 0 completes, can run in parallel with each other)
- **Level 2**: Steps that depend on Level 1, and so on...

**Example:**
```
Level 0: [Step A, Step B, Step C]  → Run simultaneously
Level 1: [Step D, Step E]          → Run simultaneously (after Level 0)
Level 2: [Step F]                  → Run after Level 1
```

## Sequential vs Parallel Execution

### Sequential Execution (`run_workflow_yaml_with_callback`)

- **Behavior**: Executes steps one at a time, even if they're independent
- **Order**: Respects dependencies (topological sort)
- **Use Cases**:
  - Debugging workflows (easier to trace issues)
  - Resource-constrained environments
  - When you need strict step-by-step logging
  - Testing/debugging mode

**Example Timeline:**
```
Time: 0s    1s    2s    3s    4s    5s
Step A: [====]
Step B:        [====]
Step C:             [====]
Step D:                  [====]
```

### Parallel Execution (`run_workflow_yaml_parallel_with_callback`)

- **Behavior**: Executes independent steps simultaneously within each level
- **Order**: Respects dependencies (levels execute sequentially, steps within level run in parallel)
- **Use Cases**:
  - Production workflows (faster execution)
  - Workflows with independent branches
  - Better resource utilization
  - Default mode when workflow has parallelizable steps

**Example Timeline:**
```
Time: 0s    1s    2s    3s
Level 0: [A][B][C]  (all run simultaneously)
Level 1:            [D][E]  (run simultaneously after Level 0)
Level 2:                  [F]  (runs after Level 1)
```

## During Workflow Creation

### Building Parallel Workflows in the UI

1. **Add Nodes**: Create workflow steps using the "Add Node" controls
2. **Connect Nodes**: Use the "🔗 Connect" button to create dependencies
   - Click source node → Click target node
   - Creates an edge (connection) between nodes

3. **Set Primary Input**: When a node has multiple inputs:
   - Click the node to select it
   - A popup appears showing all incoming connections
   - Choose which connection provides the **primary input** (`input_from`)
   - Other connections become **parallel dependencies** (`depends_on`)

### Visual Indicators

- **Green edges** (thicker): Primary input (`input_from`) - the main data flow
- **Purple edges** (thinner): Parallel dependencies (`depends_on`) - must complete but don't provide input
- **Purple dot** (top-left): Fan-in node (multiple inputs, can receive parallel results)
- **Blue dot** (bottom-right): Fan-out node (multiple outputs, can spawn parallel branches)
- **L# label**: Execution level indicator (shows which level the node belongs to)
- **Level bands**: Subtle background colors showing execution levels (alternating blue/purple tints)
- **Level labels**: "Level 0", "Level 1", etc. displayed on the left side (move with pan/drag)
- **Curved edges**: Bezier curves for better visual flow and reduced edge crossings
- **Auto-Layout**: Automatic hierarchical arrangement of nodes by execution level

### Example: Creating a Parallel Workflow

**Scenario**: Process three data sources, then merge results

1. **Add three initial nodes** (A, B, C) - these have no dependencies
   - They'll be in Level 0 and run in parallel automatically

2. **Add a merge node** (D) that needs all three
   - Connect A → D (this becomes primary input)
   - Connect B → D (this becomes a dependency)
   - Connect C → D (this becomes a dependency)
   - Click D and select A as primary input
   - D will be in Level 1 and wait for A, B, C to complete

3. **Result**: 
   - A, B, C run simultaneously (Level 0)
   - D runs after all three complete (Level 1)
   - The system automatically detects this and uses parallel execution

## UI Behavior

### Single "Run" Button

The UI now has a **single "▶️ Run" button** that:

- **Automatically detects** if your workflow has parallelizable steps
- **Shows "(Parallel)"** in the button text if parallel execution will be used
- **Changes color**:
  - Purple = Parallel execution will be used
  - Blue = Sequential execution (no parallelism detected)
  - Gray = Disabled (workflow running or no workflow selected)

### Debug Mode Toggle

- **🐛 Debug checkbox**: Forces sequential execution even when parallel is possible
- **Use when**: Debugging, testing, or when you need step-by-step execution
- **Visual feedback**: Button shows "Debug (Sequential)" when enabled

## When to Use Each Mode

### Use Parallel Execution (Default)

✅ **Always use parallel execution** unless you have a specific reason not to:
- Faster execution for independent steps
- Better resource utilization
- The workflow structure already defines what can run in parallel
- Production workflows

### Use Sequential Execution (Debug Mode)

✅ **Use sequential execution** when:
- Debugging workflow issues (easier to trace)
- Testing individual steps
- Resource-constrained environments
- You need strict step-by-step logging
- Troubleshooting plugin interactions

## Technical Details

### How Dependencies Work

1. **`input_from`**: 
   - The primary input source
   - The step's input parameter comes from this step's output
   - Creates a **data dependency** (green edge)

2. **`depends_on`**: 
   - Additional dependencies that must complete first
   - Don't provide input data, just ensure execution order
   - Creates **parallel dependencies** (purple edges)

### Execution Algorithm

**Sequential:**
```rust
for step in topological_order {
    execute(step);  // One at a time
    wait_for_completion();
}
```

**Parallel:**
```rust
for level in execution_levels {
    for step in level {
        spawn_thread(execute(step));  // All in level run simultaneously
    }
    wait_for_all_threads();  // Wait for level to complete
}
```

## Examples

### Example 1: Simple Sequential Chain

```yaml
steps:
  - run: StepA
  - run: StepB
    input_from: step1  # Must wait for StepA
  - run: StepC
    input_from: step2  # Must wait for StepB
```

**Execution**: Sequential (no parallelism possible)
- StepA → StepB → StepC (one after another)

### Example 2: Parallel Branches

```yaml
steps:
  - run: StepA
  - run: StepB  # Independent of StepA
  - run: StepC  # Independent of StepA and StepB
  - run: StepD
    input_from: step1
    depends_on: ["step2", "step3"]  # Waits for all three
```

**Execution**: Parallel
- Level 0: StepA, StepB, StepC run simultaneously
- Level 1: StepD runs after all three complete

### Example 3: Fan-Out Pattern

```yaml
steps:
  - run: ProcessData
  - run: AnalyzeA
    input_from: step1
  - run: AnalyzeB
    input_from: step1
  - run: AnalyzeC
    input_from: step1
```

**Execution**: Parallel
- Level 0: ProcessData runs first
- Level 1: AnalyzeA, AnalyzeB, AnalyzeC run simultaneously (all use step1 output)

## Best Practices

1. **Design workflows with parallelism in mind**:
   - Group independent operations
   - Use `depends_on` for ordering without data flow
   - Use `input_from` for data flow

2. **Let the system decide**:
   - Don't manually choose sequential/parallel
   - The workflow structure determines execution mode
   - Use Debug mode only when needed

3. **Visual feedback**:
   - Check execution levels (L# labels) in the graph
   - Use Auto-Layout button to arrange nodes hierarchically for better visualization
   - Green edges show data flow
   - Purple edges show parallel dependencies

4. **Testing**:
   - Test workflows in Debug mode first
   - Switch to parallel execution for production
   - Monitor execution metrics to see parallelism achieved

## Summary

- **Workflow structure determines execution mode** - dependencies (`input_from`, `depends_on`) define what can run in parallel
- **Single "Run" button** - automatically detects and uses parallel execution when possible
- **Debug mode** - optional toggle for sequential execution when debugging
- **Visual indicators** - graph shows execution levels and dependency types
- **Best practice** - design workflows with parallelism in mind, let the system handle execution

The key insight: **You define the workflow structure, LAO determines the optimal execution strategy.**
