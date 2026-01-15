//! Component-level tests for lao-ui components

use lao_ui::backend::{BackendState, GraphEdge, GraphNode, WorkflowGraph};
use std::sync::{Arc, Mutex};

#[test]
fn test_graph_state_default() {
    use lao_ui::components::graph::GraphEditorState;
    let state = GraphEditorState::default();
    assert!(state.selected_node.is_none());
    assert_eq!(state.pan_offset, eframe::egui::Vec2::ZERO);
    assert!(!state.connect_mode_active);
    assert!(state.connecting_from.is_none());
    assert!(!state.show_node_input_selector);
    assert_eq!(state.new_node_type, "EchoPlugin");
    assert!(!state.show_save_dialog);
    assert!(!state.show_export_dialog);
}

#[test]
fn test_backend_state_with_graph() {
    let mut state = BackendState::default();
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
    state.graph = Some(graph);
    assert!(state.graph.is_some());
    assert_eq!(state.graph.as_ref().unwrap().nodes.len(), 1);
}

#[test]
fn test_backend_state_debug_mode() {
    let mut state = BackendState::default();
    assert!(!state.debug_mode);
    state.debug_mode = true;
    assert!(state.debug_mode);
}

#[test]
fn test_workflow_result_creation() {
    use lao_ui::backend::WorkflowResult;
    let result = WorkflowResult {
        success: true,
        total_steps: 3,
        completed_steps: 3,
        failed_steps: 0,
        execution_time: 1.5,
        final_message: "Workflow completed successfully".to_string(),
        parallel_execution: None,
    };
    assert!(result.success);
    assert_eq!(result.total_steps, 3);
    assert_eq!(result.completed_steps, 3);
    assert_eq!(result.failed_steps, 0);
}

#[test]
fn test_parallel_execution_metrics() {
    use lao_ui::backend::ParallelExecutionMetrics;
    let metrics = ParallelExecutionMetrics {
        execution_levels: 3,
        max_parallelism: 4,
        average_parallelism: 2.5,
        estimated_sequential_time: 10.0,
        speedup: 4.0,
    };
    assert_eq!(metrics.execution_levels, 3);
    assert_eq!(metrics.max_parallelism, 4);
    assert_eq!(metrics.average_parallelism, 2.5);
    assert_eq!(metrics.speedup, 4.0);
}

#[test]
fn test_graph_node_status_updates() {
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
    assert_eq!(node.status, "pending");
    node.status = "running".to_string();
    assert_eq!(node.status, "running");
    node.status = "success".to_string();
    assert_eq!(node.status, "success");
}

#[test]
fn test_graph_edge_validation() {
    let edge1 = GraphEdge {
        from: "node1".to_string(),
        to: "node2".to_string(),
    };
    let edge2 = GraphEdge {
        from: "node1".to_string(),
        to: "node2".to_string(),
    };
    // Edges with same from/to should be considered duplicates
    assert_eq!(edge1.from, edge2.from);
    assert_eq!(edge1.to, edge2.to);
}

#[test]
fn test_graph_node_primary_input() {
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
    assert!(node.primary_input.is_none());
    node.primary_input = Some("node1".to_string());
    assert_eq!(node.primary_input, Some("node1".to_string()));
}

#[test]
fn test_graph_node_execution_level() {
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
    assert!(node.execution_level.is_none());
    node.execution_level = Some(0);
    assert_eq!(node.execution_level, Some(0));
    node.execution_level = Some(2);
    assert_eq!(node.execution_level, Some(2));
}
