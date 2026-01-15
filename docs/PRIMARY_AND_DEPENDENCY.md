# Understanding Primary Input vs Dependencies

## The Pattern: step1 → step2 (primary) + step2 depends_on step3

This pattern combines **data flow** (`input_from`) with **execution ordering** (`depends_on`).

## Example YAML

```yaml
workflow: "Primary Input with Dependency"
steps:
  - run: ProcessData
    input: "Initial data"
    # step1: No dependencies, runs first
  
  - run: AnalyzeData
    input_from: step1  # Primary input - gets data from step1
    depends_on: ["step3"]  # Also waits for step3 to complete
    # step2: Uses step1's output, but waits for step3
  
  - run: ValidateData
    input: "Validation rules"
    # step3: Independent, runs in parallel with step1
```

## What This Means

### Execution Flow

1. **Level 0** (runs in parallel):
   - **step1** (ProcessData) - processes initial data
   - **step3** (ValidateData) - validates rules (independent)

2. **Level 1** (runs after Level 0 completes):
   - **step2** (AnalyzeData) - waits for BOTH:
     - step1 to complete (to get its output as input)
     - step3 to complete (for ordering/synchronization)

### Data Flow

- **step2 receives**: The output from step1 (via `input_from`)
- **step2 does NOT receive**: Data from step3 (step3 is just a dependency)
- **step2 waits for**: Both step1 AND step3 to finish before starting

## Visual Representation

```
Level 0:  [step1: ProcessData]  [step3: ValidateData]
              │                      │
              │ (primary input)      │ (dependency)
              │                      │
              └──────────┬───────────┘
                         │
Level 1:            [step2: AnalyzeData]
                    (uses step1 output,
                     waits for step3)
```

## In the UI

- **Green edge** (thick): step1 → step2 (primary input, data flows here)
- **Purple edge** (thin): step3 → step2 (dependency, just ordering)
- **step2** shows: Purple dot (top-left) indicating fan-in (multiple inputs)

## Use Cases

### Scenario 1: Data Processing with Validation
```yaml
steps:
  - run: LoadData      # step1: Load data file
  - run: ProcessData   # step2: Process the loaded data
    input_from: step1  # Uses loaded data
    depends_on: ["step3"]  # But wait for validation to complete
  - run: ValidateRules # step3: Validate processing rules
```

**Why**: You want to process the data (from step1), but only after validation rules are ready (step3). The validation doesn't provide data, just ensures it's done first.

### Scenario 2: Parallel Preparation + Merge
```yaml
steps:
  - run: FetchData     # step1: Get data from source A
  - run: MergeData     # step2: Merge data
    input_from: step1  # Primary data comes from step1
    depends_on: ["step3"]  # But wait for step3 to finish
  - run: FetchMetadata # step3: Get metadata from source B
```

**Why**: You need data from step1, but also need step3 (metadata) to complete before merging. The merge uses step1's data, but waits for step3's completion.

## Key Differences

| Feature | `input_from` | `depends_on` |
|---------|-------------|--------------|
| **Purpose** | Data flow | Execution ordering |
| **Provides data?** | ✅ Yes | ❌ No |
| **Visual** | Green edge (thick) | Purple edge (thin) |
| **Execution** | Must complete AND provide output | Must complete (output not used) |
| **Use case** | "I need this step's result" | "I need this step to finish first" |

## Execution Timeline

```
Time:  0s    1s    2s    3s
step1: [====] (processes data)
step3: [====] (validates rules)
step2:        [====] (analyzes, uses step1 output)
```

Both step1 and step3 run in parallel (Level 0). Step2 waits for both to complete, but only uses step1's output as its input.

## Common Patterns

### Pattern 1: Sequential Chain with Side Dependency
```yaml
steps:
  - run: StepA
  - run: StepB
    input_from: step1  # Chain continues
    depends_on: ["step3"]  # But also wait for step3
  - run: StepC  # Independent side task
```

### Pattern 2: Fan-In with Primary Input
```yaml
steps:
  - run: DataSource1  # step1
  - run: DataSource2  # step2
  - run: Merge
    input_from: step1  # Primary data from step1
    depends_on: ["step2"]  # But wait for step2 too
```

## Summary

- **`input_from: step1`** = "Give me step1's output as my input"
- **`depends_on: ["step3"]`** = "Wait for step3 to finish (but I don't need its output)"
- **Together** = "I need step1's data, but I also need step3 to complete before I start"

This pattern is useful when you need:
- Data from one step (primary input)
- Synchronization with another step (dependency)
- Both conditions met before execution
