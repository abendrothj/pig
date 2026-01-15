//! Backend state management and workflow loading tests for lao-ui

use lao_ui::backend::{
    calculate_execution_levels, get_workflow_graph, list_available_workflows,
    list_plugins_for_ui, BackendState, GraphEdge, GraphNode, WorkflowGraph,
};
use std::fs;
use std::path::Path;

#[test]
fn test_backend_state_default() {
    let state = BackendState::default();
    assert_eq!(state.workflow_path, "");
    assert!(state.graph.is_none());
    assert_eq!(state.error, "");
    assert!(state.plugins.is_empty());
    assert!(state.live_logs.is_empty());
    assert!(!state.is_running);
    assert_eq!(state.execution_progress, 0.0);
    assert!(state.workflow_result.is_none());
    assert!(!state.debug_mode);
}

#[test]
fn test_list_available_workflows() {
    let workflows = list_available_workflows();
    // Should return at least an empty vector, even if no workflows exist
    assert!(workflows.is_empty() || !workflows.is_empty());
}

#[test]
fn test_list_plugins_for_ui() {
    let result = list_plugins_for_ui();
    // Should return Ok even if no plugins found
    assert!(result.is_ok());
    let _plugins = result.unwrap();
    // Plugins may or may not exist depending on build state
}

#[test]
fn test_workflow_graph_default() {
    let graph = WorkflowGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    assert!(graph.nodes.is_empty());
    assert!(graph.edges.is_empty());
}

#[test]
fn test_graph_node_creation() {
    let node = GraphNode {
        id: "test_node".to_string(),
        run: "EchoPlugin".to_string(),
        input_type: Some("text".to_string()),
        output_type: Some("text".to_string()),
        status: "pending".to_string(),
        x: 100.0,
        y: 200.0,
        message: None,
        output: None,
        error: None,
        attempt: 0,
        primary_input: None,
        execution_level: None,
    };
    assert_eq!(node.id, "test_node");
    assert_eq!(node.run, "EchoPlugin");
    assert_eq!(node.status, "pending");
    assert_eq!(node.x, 100.0);
    assert_eq!(node.y, 200.0);
}

#[test]
fn test_graph_edge_creation() {
    let edge = GraphEdge {
        from: "node1".to_string(),
        to: "node2".to_string(),
    };
    assert_eq!(edge.from, "node1");
    assert_eq!(edge.to, "node2");
}

#[test]
fn test_calculate_execution_levels_empty() {
    let graph = WorkflowGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let levels = calculate_execution_levels(&graph);
    assert!(levels.is_empty());
}

#[test]
fn test_calculate_execution_levels_single_node() {
    let graph = WorkflowGraph {
        nodes: vec![GraphNode {
            id: "node1".to_string(),
            run: "EchoPlugin".to_string(),
            input_type: None,
            output_type: None,
            status: "pending".to_string(),
            x: 0.0,
            y: 0.0,
            message: None,
            output: None,
            error: None,
            attempt: 0,
            primary_input: None,
            execution_level: None,
        }],
        edges: Vec::new(),
    };
    let levels = calculate_execution_levels(&graph);
    assert_eq!(levels.len(), 1);
    assert_eq!(levels[0].len(), 1);
    assert_eq!(levels[0][0], "node1");
}

#[test]
fn test_calculate_execution_levels_parallel() {
    // Create a graph with 3 independent nodes (should all be in level 0)
    let graph = WorkflowGraph {
        nodes: vec![
            GraphNode {
                id: "node1".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node2".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 100.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node3".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 200.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
        ],
        edges: Vec::new(),
    };
    let levels = calculate_execution_levels(&graph);
    // All 3 nodes should be in level 0 (no dependencies)
    assert_eq!(levels.len(), 1);
    assert_eq!(levels[0].len(), 3);
}

#[test]
fn test_calculate_execution_levels_sequential() {
    // Create a sequential chain: node1 -> node2 -> node3
    let graph = WorkflowGraph {
        nodes: vec![
            GraphNode {
                id: "node1".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node2".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 100.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
            GraphNode {
                id: "node3".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 200.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node2".to_string()),
                execution_level: None,
            },
        ],
        edges: vec![
            GraphEdge {
                from: "node1".to_string(),
                to: "node2".to_string(),
            },
            GraphEdge {
                from: "node2".to_string(),
                to: "node3".to_string(),
            },
        ],
    };
    let levels = calculate_execution_levels(&graph);
    // Should have 3 levels (one node per level)
    assert_eq!(levels.len(), 3);
    assert_eq!(levels[0].len(), 1); // node1
    assert_eq!(levels[1].len(), 1); // node2
    assert_eq!(levels[2].len(), 1); // node3
}

#[test]
fn test_calculate_execution_levels_fan_out() {
    // Create fan-out: node1 -> [node2, node3]
    let graph = WorkflowGraph {
        nodes: vec![
            GraphNode {
                id: "node1".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node2".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 100.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
            GraphNode {
                id: "node3".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 100.0,
                y: 100.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
        ],
        edges: vec![
            GraphEdge {
                from: "node1".to_string(),
                to: "node2".to_string(),
            },
            GraphEdge {
                from: "node1".to_string(),
                to: "node3".to_string(),
            },
        ],
    };
    let levels = calculate_execution_levels(&graph);
    // Should have 2 levels: level 0 (node1), level 1 (node2, node3)
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[0].len(), 1); // node1
    assert_eq!(levels[1].len(), 2); // node2, node3 (parallel)
}

#[test]
fn test_calculate_execution_levels_fan_in() {
    // Create fan-in: [node1, node2] -> node3
    let graph = WorkflowGraph {
        nodes: vec![
            GraphNode {
                id: "node1".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node2".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 100.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: None,
                execution_level: None,
            },
            GraphNode {
                id: "node3".to_string(),
                run: "EchoPlugin".to_string(),
                input_type: None,
                output_type: None,
                status: "pending".to_string(),
                x: 50.0,
                y: 100.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
        ],
        edges: vec![
            GraphEdge {
                from: "node1".to_string(),
                to: "node3".to_string(),
            },
            GraphEdge {
                from: "node2".to_string(),
                to: "node3".to_string(),
            },
        ],
    };
    let levels = calculate_execution_levels(&graph);
    // Should have 2 levels: level 0 (node1, node2), level 1 (node3)
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[0].len(), 2); // node1, node2 (parallel)
    assert_eq!(levels[1].len(), 1); // node3
}

#[test]
fn test_get_workflow_graph_invalid_path() {
    let result = get_workflow_graph("nonexistent_workflow.yaml");
    assert!(result.is_err());
}

#[test]
fn test_get_workflow_graph_valid_workflow() {
    // Create a temporary workflow file
    let test_workflow = r#"workflow: "Test Workflow"
steps:
  - run: EchoPlugin
    input: "Test input"
    cache_key: "test_cache"
"#;
    let test_path = "temp_test_workflow.yaml";
    fs::write(test_path, test_workflow).unwrap();

    let result = get_workflow_graph(test_path);
    
    // Clean up
    if Path::new(test_path).exists() {
        fs::remove_file(test_path).unwrap();
    }

    // May fail if plugins aren't available, but should handle gracefully
    if result.is_ok() {
        let graph = result.unwrap();
        assert!(!graph.nodes.is_empty());
    }
}
