//! Integration tests for lao-ui components working together

use lao_ui::backend::{BackendState, GraphEdge, GraphNode, WorkflowGraph};
use std::sync::{Arc, Mutex};

#[test]
fn test_backend_state_arc_mutex() {
    let state = Arc::new(Mutex::new(BackendState::default()));
    {
        let state_guard = state.lock().unwrap();
        assert_eq!(state_guard.workflow_path, "");
    }
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.workflow_path = "test.yaml".to_string();
        assert_eq!(state_guard.workflow_path, "test.yaml");
    }
}

#[test]
fn test_workflow_graph_operations() {
    let mut graph = WorkflowGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    
    // Add nodes
    let node1 = GraphNode {
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
    };
    let node2 = GraphNode {
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
    };
    
    graph.nodes.push(node1);
    graph.nodes.push(node2);
    
    // Add edge
    graph.edges.push(GraphEdge {
        from: "node1".to_string(),
        to: "node2".to_string(),
    });
    
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    
    // Test node lookup
    let node1_ref = graph.nodes.iter().find(|n| n.id == "node1");
    assert!(node1_ref.is_some());
    assert_eq!(node1_ref.unwrap().run, "EchoPlugin");
    
    // Test edge lookup
    let edge = graph.edges.iter().find(|e| e.from == "node1" && e.to == "node2");
    assert!(edge.is_some());
}

#[test]
fn test_graph_node_removal() {
    let mut graph = WorkflowGraph {
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
        ],
        edges: vec![
            GraphEdge {
                from: "node1".to_string(),
                to: "node2".to_string(),
            },
        ],
    };
    
    // Remove node1
    graph.nodes.retain(|n| n.id != "node1");
    // Edges should also be removed
    graph.edges.retain(|e| e.from != "node1" && e.to != "node1");
    
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.edges.len(), 0);
}

#[test]
fn test_execution_level_calculation_with_primary_input() {
    use lao_ui::backend::calculate_execution_levels;
    
    // Create a graph where node2 uses node1 as primary input
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
        ],
        edges: vec![
            GraphEdge {
                from: "node1".to_string(),
                to: "node2".to_string(),
            },
        ],
    };
    
    let levels = calculate_execution_levels(&graph);
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[0].len(), 1); // node1
    assert_eq!(levels[1].len(), 1); // node2
}

#[test]
fn test_parallel_detection() {
    use lao_ui::backend::calculate_execution_levels;
    
    // Create a graph with parallel steps
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
        edges: Vec::new(), // No edges = all parallel
    };
    
    let levels = calculate_execution_levels(&graph);
    // All 3 nodes should be in level 0 (parallel)
    assert_eq!(levels.len(), 1);
    assert_eq!(levels[0].len(), 3);
    
    // Check if any level has more than 1 node (parallelism detected)
    let has_parallel = levels.iter().any(|level| level.len() > 1);
    assert!(has_parallel);
}

#[test]
fn test_node_status_transitions() {
    let mut node = GraphNode {
        id: "test_node".to_string(),
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
    };
    
    // Simulate status transitions
    assert_eq!(node.status, "pending");
    node.status = "running".to_string();
    assert_eq!(node.status, "running");
    node.status = "success".to_string();
    assert_eq!(node.status, "success");
    
    // Test error state
    node.status = "error".to_string();
    node.error = Some("Test error".to_string());
    assert_eq!(node.status, "error");
    assert_eq!(node.error, Some("Test error".to_string()));
}

#[test]
fn test_graph_serialization() {
    use serde_json;
    
    let graph = WorkflowGraph {
        nodes: vec![GraphNode {
            id: "test_node".to_string(),
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
    
    // Test serialization
    let json = serde_json::to_string(&graph);
    assert!(json.is_ok());
    
    // Test deserialization
    let json_str = json.unwrap();
    let deserialized: Result<WorkflowGraph, _> = serde_json::from_str(&json_str);
    assert!(deserialized.is_ok());
    let deserialized_graph = deserialized.unwrap();
    assert_eq!(deserialized_graph.nodes.len(), 1);
    assert_eq!(deserialized_graph.nodes[0].id, "test_node");
}
