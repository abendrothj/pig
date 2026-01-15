# LAO UI Tests

This directory contains comprehensive tests for the LAO UI application.

## Test Structure

### Backend Tests (`backend_tests.rs`)
Tests for backend state management, workflow loading, and execution level calculation:
- `test_backend_state_default` - Verifies default state initialization
- `test_list_available_workflows` - Tests workflow discovery
- `test_list_plugins_for_ui` - Tests plugin listing
- `test_workflow_graph_default` - Tests empty graph creation
- `test_graph_node_creation` - Tests node creation and properties
- `test_graph_edge_creation` - Tests edge creation
- `test_calculate_execution_levels_*` - Tests execution level calculation for various graph patterns:
  - Empty graphs
  - Single nodes
  - Parallel execution (independent nodes)
  - Sequential execution (chained nodes)
  - Fan-out patterns (one-to-many)
  - Fan-in patterns (many-to-one)
- `test_get_workflow_graph_*` - Tests workflow loading from YAML files

### Component Tests (`component_tests.rs`)
Tests for UI component state and data structures:
- `test_graph_state_default` - Tests graph editor state initialization
- `test_backend_state_with_graph` - Tests state with graph data
- `test_backend_state_debug_mode` - Tests debug mode toggle
- `test_workflow_result_creation` - Tests workflow result structures
- `test_parallel_execution_metrics` - Tests parallel execution metrics
- `test_graph_node_status_updates` - Tests node status transitions
- `test_graph_edge_validation` - Tests edge validation logic
- `test_graph_node_primary_input` - Tests primary input assignment
- `test_graph_node_execution_level` - Tests execution level assignment

### Integration Tests (`integration_tests.rs`)
Tests for components working together:
- `test_backend_state_arc_mutex` - Tests thread-safe state access
- `test_workflow_graph_operations` - Tests graph CRUD operations
- `test_graph_node_removal` - Tests node deletion and edge cleanup
- `test_execution_level_calculation_with_primary_input` - Tests level calculation with dependencies
- `test_parallel_detection` - Tests automatic parallelism detection
- `test_node_status_transitions` - Tests status state machine
- `test_graph_serialization` - Tests JSON serialization/deserialization

### Layout Tests (`layout_tests.rs`)
Tests for auto-layout and hierarchical visualization:
- `test_auto_layout_empty_graph` - Tests layout with empty graph
- `test_auto_layout_single_node` - Tests layout with single node
- `test_auto_layout_sequential_chain` - Tests layout for sequential workflows
- `test_auto_layout_fan_out` - Tests layout for fan-out patterns (one-to-many)
- `test_auto_layout_fan_in` - Tests layout for fan-in patterns (many-to-one)
- `test_auto_layout_assigns_execution_levels` - Verifies execution levels are assigned
- `test_auto_layout_handles_orphan_nodes` - Tests handling of nodes without edges
- `test_auto_layout_complex_workflow` - Tests layout for complex multi-level workflows

## Running Tests

### Run All Tests
```bash
cd ui/lao-ui
cargo test
```

### Run Specific Test Suite
```bash
# Backend tests only
cargo test --test backend_tests

# Component tests only
cargo test --test component_tests

# Integration tests only
cargo test --test integration_tests

# Layout tests only
cargo test --test layout_tests
```

### Run Specific Test
```bash
cargo test test_backend_state_default
```

### Run Tests with Output
```bash
cargo test -- --nocapture
```

### Run Tests in Parallel
```bash
cargo test -- --test-threads=1  # Sequential
cargo test -- --test-threads=4  # Parallel (default)
```

## Test Coverage

Current test coverage includes:
- ✅ Backend state management
- ✅ Workflow graph operations
- ✅ Execution level calculation
- ✅ Parallel execution detection
- ✅ Node and edge operations
- ✅ State serialization
- ✅ Component state management
- ✅ Auto-layout and hierarchical visualization
- ✅ Layout algorithms for various graph patterns (sequential, fan-out, fan-in, complex)

### Workflow Execution Tests (`workflow_execution_tests.rs`)
End-to-end tests that actually execute workflows:
- `test_workflow_execution_single_step` - Executes a simple single-step workflow
- `test_workflow_execution_multi_step_sequential` - Executes sequential multi-step workflow
- `test_workflow_execution_parallel` - Executes parallel workflow and verifies parallelism metrics
- `test_workflow_execution_with_errors` - Tests error handling during workflow execution
- `test_workflow_execution_progress_tracking` - Verifies progress tracking during execution
- `test_workflow_execution_state_updates` - Tests state transitions (pending → running → completed)
- `test_workflow_execution_logs_accumulation` - Verifies logs are properly accumulated during execution

## Future Test Additions

Potential areas for additional testing:
- [ ] UI interaction tests (using egui_kittest or similar)
- [ ] Workflow execution integration tests
- [ ] Error handling and edge cases
- [ ] Performance tests for large graphs
- [ ] Visual regression tests (snapshot testing)
- [x] Auto-layout algorithm tests (✅ Added)
- [x] Hierarchical visualization tests (✅ Added)

## Notes

- Tests that require plugins may be skipped if plugins aren't available (graceful degradation)
- Some tests create temporary files that are cleaned up automatically
- Tests use `serial_test` where needed to prevent race conditions
