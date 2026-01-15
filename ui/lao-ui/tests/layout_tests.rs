//! Tests for auto-layout and hierarchical visualization features

use lao_ui::backend::{
    auto_layout_graph_hierarchical, calculate_execution_levels, GraphEdge, GraphNode, WorkflowGraph,
};

#[test]
fn test_auto_layout_empty_graph() {
    let mut graph = WorkflowGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    
    // Should not panic on empty graph
    auto_layout_graph_hierarchical(&mut graph);
    assert!(graph.nodes.is_empty());
}

#[test]
fn test_auto_layout_single_node() {
    let mut graph = WorkflowGraph {
        nodes: vec![GraphNode {
            id: "node1".to_string(),
            run: "EchoPlugin".to_string(),
            input_type: None,
            output_type: None,
            status: "pending".to_string(),
            x: 999.0, // Random initial position
            y: 999.0,
            message: None,
            output: None,
            error: None,
            attempt: 0,
            primary_input: None,
            execution_level: None,
        }],
        edges: Vec::new(),
    };
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Node should be positioned at level 0
    let node = &graph.nodes[0];
    assert_eq!(node.execution_level, Some(0));
    // Should be positioned at start_y (50.0)
    assert_eq!(node.y, 50.0);
    // X should be centered (start_x + (400.0 - total_width / 2.0).max(0.0))
    // For single node: total_width = 0, so level_start_x = start_x + 400.0 = 500.0
    assert_eq!(node.x, 500.0);
}

#[test]
fn test_auto_layout_sequential_chain() {
    // Create sequential chain: node1 -> node2 -> node3
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
                x: 0.0,
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
                x: 0.0,
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
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Check execution levels
    assert_eq!(graph.nodes[0].execution_level, Some(0));
    assert_eq!(graph.nodes[1].execution_level, Some(1));
    assert_eq!(graph.nodes[2].execution_level, Some(2));
    
    // Check vertical positioning (levels should be spaced)
    let level_height = 150.0;
    let level_spacing = 20.0;
    let start_y = 50.0;
    
    assert_eq!(graph.nodes[0].y, start_y + (0.0 * (level_height + level_spacing)));
    assert_eq!(graph.nodes[1].y, start_y + (1.0 * (level_height + level_spacing)));
    assert_eq!(graph.nodes[2].y, start_y + (2.0 * (level_height + level_spacing)));
    
    // All nodes should be centered horizontally (same x for single-node levels)
    // For single-node levels, they're centered: start_x + (400.0 - 0 / 2.0) = 500.0
    let centered_x = 500.0;
    assert_eq!(graph.nodes[0].x, centered_x);
    assert_eq!(graph.nodes[1].x, centered_x);
    assert_eq!(graph.nodes[2].x, centered_x);
}

#[test]
fn test_auto_layout_fan_out() {
    // Create fan-out: node1 -> [node2, node3]
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
                x: 0.0,
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
                x: 0.0,
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
            GraphEdge {
                from: "node1".to_string(),
                to: "node3".to_string(),
            },
        ],
    };
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Check execution levels
    assert_eq!(graph.nodes[0].execution_level, Some(0)); // node1
    assert_eq!(graph.nodes[1].execution_level, Some(1)); // node2
    assert_eq!(graph.nodes[2].execution_level, Some(1)); // node3
    
    // Level 0 should have node1
    // Level 1 should have node2 and node3 (parallel)
    let levels = calculate_execution_levels(&graph);
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[0].len(), 1); // node1
    assert_eq!(levels[1].len(), 2); // node2, node3
    
    // Check horizontal spacing for parallel nodes
    let horizontal_spacing = 180.0;
    
    // node2 and node3 should be horizontally spaced
    let node2_x = graph.nodes.iter().find(|n| n.id == "node2").unwrap().x;
    let node3_x = graph.nodes.iter().find(|n| n.id == "node3").unwrap().x;
    
    // They should be spaced horizontally (centered, so node2 at ~410, node3 at ~590)
    assert!((node3_x - node2_x).abs() >= horizontal_spacing - 1.0); // Allow small floating point error
}

#[test]
fn test_auto_layout_fan_in() {
    // Create fan-in: [node1, node2] -> node3
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
                id: "node3".to_string(),
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
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Check execution levels
    let node1 = graph.nodes.iter().find(|n| n.id == "node1").unwrap();
    let node2 = graph.nodes.iter().find(|n| n.id == "node2").unwrap();
    let node3 = graph.nodes.iter().find(|n| n.id == "node3").unwrap();
    
    assert_eq!(node1.execution_level, Some(0));
    assert_eq!(node2.execution_level, Some(0)); // Parallel with node1
    assert_eq!(node3.execution_level, Some(1));
    
    // node1 and node2 should be at same level (y coordinate)
    assert_eq!(node1.y, node2.y);
    
    // node3 should be at next level
    let level_height = 150.0;
    let level_spacing = 20.0;
    assert_eq!(node3.y, node1.y + level_height + level_spacing);
}

#[test]
fn test_auto_layout_assigns_execution_levels() {
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
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
        ],
        edges: vec![GraphEdge {
            from: "node1".to_string(),
            to: "node2".to_string(),
        }],
    };
    
    // Initially, execution levels should be None
    assert!(graph.nodes[0].execution_level.is_none());
    assert!(graph.nodes[1].execution_level.is_none());
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // After layout, execution levels should be assigned
    assert!(graph.nodes[0].execution_level.is_some());
    assert!(graph.nodes[1].execution_level.is_some());
    assert_eq!(graph.nodes[0].execution_level, Some(0));
    assert_eq!(graph.nodes[1].execution_level, Some(1));
}

#[test]
fn test_auto_layout_handles_orphan_nodes() {
    // Create graph with a node that has no edges (orphan)
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
                id: "orphan".to_string(),
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
        ],
        edges: Vec::new(), // No edges = orphan
    };
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Orphan node should be placed at the end
    let orphan = graph.nodes.iter().find(|n| n.id == "orphan").unwrap();
    let node1 = graph.nodes.iter().find(|n| n.id == "node1").unwrap();
    
    // Orphan should be at a level after node1 (or same level if both are orphans)
    // Since both have no edges, they should both be in level 0
    assert_eq!(orphan.execution_level, Some(0));
    assert_eq!(node1.execution_level, Some(0));
}

#[test]
fn test_auto_layout_complex_workflow() {
    // Create a complex workflow: fan-out followed by fan-in
    // node1 -> [node2, node3] -> node4
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
                x: 0.0,
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
                x: 0.0,
                y: 0.0,
                message: None,
                output: None,
                error: None,
                attempt: 0,
                primary_input: Some("node1".to_string()),
                execution_level: None,
            },
            GraphNode {
                id: "node4".to_string(),
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
                from: "node1".to_string(),
                to: "node3".to_string(),
            },
            GraphEdge {
                from: "node2".to_string(),
                to: "node4".to_string(),
            },
            GraphEdge {
                from: "node3".to_string(),
                to: "node4".to_string(),
            },
        ],
    };
    
    auto_layout_graph_hierarchical(&mut graph);
    
    // Verify structure
    let node1 = graph.nodes.iter().find(|n| n.id == "node1").unwrap();
    let node2 = graph.nodes.iter().find(|n| n.id == "node2").unwrap();
    let node3 = graph.nodes.iter().find(|n| n.id == "node3").unwrap();
    let node4 = graph.nodes.iter().find(|n| n.id == "node4").unwrap();
    
    assert_eq!(node1.execution_level, Some(0));
    assert_eq!(node2.execution_level, Some(1));
    assert_eq!(node3.execution_level, Some(1)); // Parallel with node2
    assert_eq!(node4.execution_level, Some(2));
    
    // Verify vertical spacing
    let level_height = 150.0;
    let level_spacing = 20.0;
    assert_eq!(node2.y, node3.y); // Same level
    assert!(node4.y > node2.y); // Next level
    assert_eq!(node4.y, node2.y + level_height + level_spacing);
}
