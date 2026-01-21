//! Undo/Redo system for graph editing operations

use crate::backend::{GraphNode, GraphEdge};
use std::collections::HashMap;

/// Command pattern for undoable operations
pub trait Command: std::fmt::Debug {
    fn execute(&mut self, state: &mut EditorState);
    fn undo(&mut self, state: &mut EditorState);
    fn redo(&mut self, state: &mut EditorState) {
        self.execute(state);
    }
}

/// Editor state for undo/redo
#[derive(Clone, Debug)]
pub struct EditorState {
    pub nodes: HashMap<String, GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Command history manager
#[derive(Default)]
pub struct CommandHistory {
    pub undo_stack: Vec<Box<dyn Command>>,
    pub redo_stack: Vec<Box<dyn Command>>,
    pub max_history: usize,
}

impl CommandHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history: 100,
        }
    }

    pub fn execute(&mut self, mut command: Box<dyn Command>, state: &mut EditorState) {
        command.execute(state);
        self.undo_stack.push(command);
        self.redo_stack.clear(); // Clear redo stack when new action performed
        
        // Keep history size under limit
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    pub fn undo(&mut self, state: &mut EditorState) -> bool {
        if let Some(mut command) = self.undo_stack.pop() {
            command.undo(state);
            self.redo_stack.push(command);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, state: &mut EditorState) -> bool {
        if let Some(mut command) = self.redo_stack.pop() {
            command.redo(state);
            self.undo_stack.push(command);
            true
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

// --- Concrete Commands ---

/// Add node command
#[derive(Debug)]
pub struct AddNodeCommand {
    pub node: GraphNode,
}

impl Command for AddNodeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        state.nodes.insert(self.node.id.clone(), self.node.clone());
    }

    fn undo(&mut self, state: &mut EditorState) {
        state.nodes.remove(&self.node.id);
    }
}

/// Delete node command
#[derive(Debug)]
pub struct DeleteNodeCommand {
    pub node: GraphNode,
    pub connected_edges: Vec<GraphEdge>,
}

impl Command for DeleteNodeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        state.nodes.remove(&self.node.id);
        // Remove all edges connected to this node
        state.edges.retain(|e| e.from != self.node.id && e.to != self.node.id);
    }

    fn undo(&mut self, state: &mut EditorState) {
        state.nodes.insert(self.node.id.clone(), self.node.clone());
        // Restore connected edges
        for edge in &self.connected_edges {
            state.edges.push(edge.clone());
        }
    }
}

/// Move node command
#[derive(Debug)]
pub struct MoveNodeCommand {
    pub node_id: String,
    pub old_x: f32,
    pub old_y: f32,
    pub new_x: f32,
    pub new_y: f32,
}

impl Command for MoveNodeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(node) = state.nodes.get_mut(&self.node_id) {
            node.x = self.new_x;
            node.y = self.new_y;
        }
    }

    fn undo(&mut self, state: &mut EditorState) {
        if let Some(node) = state.nodes.get_mut(&self.node_id) {
            node.x = self.old_x;
            node.y = self.old_y;
        }
    }
}

/// Add edge command
#[derive(Debug)]
pub struct AddEdgeCommand {
    pub edge: GraphEdge,
}

impl Command for AddEdgeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        state.edges.push(self.edge.clone());
    }

    fn undo(&mut self, state: &mut EditorState) {
        state.edges.retain(|e| e.from != self.edge.from || e.to != self.edge.to);
    }
}

/// Delete edge command
#[derive(Debug)]
pub struct DeleteEdgeCommand {
    pub edge: GraphEdge,
}

impl Command for DeleteEdgeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        state.edges.retain(|e| e.from != self.edge.from || e.to != self.edge.to);
    }

    fn undo(&mut self, state: &mut EditorState) {
        state.edges.push(self.edge.clone());
    }
}

/// Edit node command (change plugin, params, etc.)
#[derive(Debug)]
pub struct EditNodeCommand {
    pub node_id: String,
    pub old_node: GraphNode,
    pub new_node: GraphNode,
}

impl Command for EditNodeCommand {
    fn execute(&mut self, state: &mut EditorState) {
        state.nodes.insert(self.node_id.clone(), self.new_node.clone());
    }

    fn undo(&mut self, state: &mut EditorState) {
        state.nodes.insert(self.node_id.clone(), self.old_node.clone());
    }
}

/// Batch command for multiple operations
#[derive(Debug)]
pub struct BatchCommand {
    pub commands: Vec<Box<dyn Command>>,
    pub description: String,
}

impl Command for BatchCommand {
    fn execute(&mut self, state: &mut EditorState) {
        for cmd in &mut self.commands {
            cmd.execute(state);
        }
    }

    fn undo(&mut self, state: &mut EditorState) {
        // Undo in reverse order
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(state);
        }
    }
}
