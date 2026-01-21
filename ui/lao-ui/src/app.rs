use eframe::egui::{self, Color32, RichText};
use std::sync::{Arc, Mutex};

use crate::backend::{list_available_workflows, list_plugins_for_ui, BackendState};
use crate::components::{graph, logs, settings, toolbar};
use crate::undo::CommandHistory;
use crate::metrics::ExecutionMetrics;
use crate::file_upload::FileDropState;
use crate::timeline::TimelineState;
use crate::multimodal::ModalityInfo;

pub struct LaoApp {
    state: Arc<Mutex<BackendState>>,

    // UI Logic states
    graph_state: graph::GraphEditorState,
    
    // New feature states
    undo_history: CommandHistory,
    metrics: ExecutionMetrics,
    file_state: FileDropState,
    timeline_state: TimelineState,
    modality_info: ModalityInfo,
    show_metrics_panel: bool,
    show_timeline_panel: bool,
    show_modality_panel: bool,
}

impl LaoApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut state = BackendState::default();

        // Try to load plugins on startup
        match list_plugins_for_ui() {
            Ok(plugins) => {
                if plugins.is_empty() {
                    eprintln!("[WARN] No plugins found. Make sure plugins are built:");
                    eprintln!("  Run: bash scripts/build-plugins.sh");
                    eprintln!("  Or build manually: cd plugins/<PluginName> && cargo build --release");
                } else {
                    println!("[INFO] Loaded {} plugins", plugins.len());
                }
                state.plugins = plugins;
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to load plugins: {}", e);
                eprintln!("[INFO] Plugin directory resolution may have failed");
                eprintln!("[INFO] Try setting LAO_PLUGINS_DIR environment variable");
            }
        }

        // Auto-populate workflow path from workflows folder on launch (if any)
        let available_workflows = list_available_workflows();
        if let Some(first) = available_workflows.first() {
            state.workflow_path = first.clone();
        }

        Self {
            state: Arc::new(Mutex::new(state)),
            graph_state: graph::GraphEditorState::default(),
            undo_history: CommandHistory::new(),
            metrics: ExecutionMetrics::new(),
            file_state: FileDropState::default(),
            timeline_state: TimelineState::default(),
            modality_info: ModalityInfo {
                input_modality: None,
                output_modality: None,
                data_flow: Vec::new(),
            },
            show_metrics_panel: false,
            show_timeline_panel: false,
            show_modality_panel: false,
        }
    }
}

impl eframe::App for LaoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set a more professional theme
        ctx.set_visuals(egui::Visuals::dark());
        
        // Show settings window if requested
        {
            let state = self.state.lock().unwrap();
            if state.show_settings {
                drop(state);
                settings::show(ctx, &self.state);
            }
        }

        // Handle keyboard shortcuts
        // TODO: Integrate undo/redo with graph editing operations
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            if ctx.input(|i| i.modifiers.shift) {
                // Cmd+Shift+Z: Redo
                // if self.undo_history.can_redo() {
                //     let _ = self.undo_history.redo(&mut editor_state);
                // }
            } else {
                // Cmd+Z: Undo
                // if self.undo_history.can_undo() {
                //     let _ = self.undo_history.undo(&mut editor_state);
                // }
            }
        }
        
        // Delete key handler
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            let mut state = self.state.lock().unwrap();
            if let (Some(selected_id), Some(ref mut graph)) =
                (self.graph_state.selected_node.clone(), &mut state.graph)
            {
                graph.nodes.retain(|n| n.id != selected_id);
                graph
                    .edges
                    .retain(|e| e.from != selected_id && e.to != selected_id);
                self.graph_state.selected_node = None;
            }
        }
        
        // Cmd+M: Toggle metrics panel
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::M)) {
            self.show_metrics_panel = !self.show_metrics_panel;
        }
        
        // Cmd+T: Toggle timeline panel
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::T)) {
            self.show_timeline_panel = !self.show_timeline_panel;
        }
        
        // Cmd+M: Toggle multimodal panel  
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.show_modality_panel = !self.show_modality_panel;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header with better styling (fixed at top)
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 70.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.add_space(8.0);
                    ui.heading(
                        RichText::new("⚡ LAO Orchestrator")
                            .size(26.0)
                            .color(Color32::from_rgb(33, 150, 243)),
                    );
                    ui.label(
                        RichText::new("Local AI Workflow Orchestrator")
                            .size(12.0)
                            .color(Color32::from_gray(150)),
                    );
                    ui.add_space(5.0);
                },
            );

            // Main scrollable content area
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(10.0);

                    // 1. Top Bar / Workflow Management
                    toolbar::show(ui, &self.state);
                    
                    // Show metrics panel if toggled
                    if self.show_metrics_panel {
                        ui.add_space(10.0);
                        crate::metrics::show_metrics_panel(ui, &self.metrics);
                        ui.add_space(10.0);
                    }
                    
                    // Show timeline panel if toggled
                    if self.show_timeline_panel {
                        ui.add_space(10.0);
                        let steps: Vec<_> = self.metrics.step_metrics.clone();
                        crate::timeline::show_timeline(ui, &steps, &mut self.timeline_state);
                        crate::timeline::show_timeline_legend(ui);
                        ui.add_space(10.0);
                    }
                    
                    // Show multimodal panel if toggled
                    if self.show_modality_panel {
                        ui.add_space(10.0);
                        crate::multimodal::show_modality_flow(ui, &self.modality_info);
                        ui.add_space(10.0);
                    }
                    
                    // Show settings window if requested (must be outside ScrollArea)
                    {
                        let state = self.state.lock().unwrap();
                        if state.show_settings {
                            drop(state);
                            // Settings window will be shown by egui::Window
                        }
                    }

                    ui.add_space(15.0);

                    // 2. Main Workspace (Graph + Inspector)
                    // We need to access state.graph. Since we're borrowing self.state (arc) for toolbar,
                    // we can lock it here.

                    let mut state = self.state.lock().unwrap();
                    let is_running = state.is_running;
                    let execution_progress = state.execution_progress;
                    let workflow_result = state.workflow_result.clone();
                    // Clone plugins so we can use them while graph is borrowed mutably
                    let plugins = state.plugins.clone();

                    // Ensure graph always exists - create empty one if needed
                    if state.graph.is_none() {
                        state.graph = Some(crate::backend::WorkflowGraph {
                            nodes: Vec::new(),
                            edges: Vec::new(),
                        });
                    }

                    // Always show Visual Flow Builder (full width, no inspector)
                    if let Some(ref mut graph) = state.graph {
                        graph::show(ui, graph, &mut self.graph_state, &plugins);
                    }

                    ui.add_space(15.0);

                    // 3. Bottom: Logs
                    logs::show(
                        ui,
                        &mut state.live_logs,
                        is_running,
                        execution_progress,
                        &workflow_result,
                    );
                });
        });
    }
}
