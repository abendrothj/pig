use crate::backend::{get_workflow_graph, list_available_workflows, resolve_workflows_dir, run_workflow_stream, BackendState};
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::{Arc, Mutex};

pub fn show(ui: &mut Ui, state_arc: &Arc<Mutex<BackendState>>) {
    ui.group(|ui| {
        ui.heading(RichText::new("📋 Workflow Management").size(18.0));

        let mut should_run = false;
        let mut should_run_parallel = false;

        // Scope for lock
        {
            let mut state = state_arc.lock().unwrap();
            
            // Workflow file selection section
            ui.horizontal(|ui| {
                ui.label(RichText::new("Workflow:").size(14.0).strong());
                
                // Dropdown for workflow selection (auto-loads on selection)
                let available = list_available_workflows();
                if !available.is_empty() {
                    let mut workflow_selected = false;
                    let mut selected_workflow = String::new();
                    
                    egui::ComboBox::from_id_salt("workflow_selector")
                        .width(300.0)
                        .selected_text(if state.workflow_path.is_empty() {
                            "Select workflow..."
                        } else {
                            &state.workflow_path
                        })
                        .show_ui(ui, |ui| {
                            for workflow in &available {
                                if ui.selectable_label(false, workflow).clicked() {
                                    workflow_selected = true;
                                    selected_workflow = workflow.clone();
                                    state.workflow_path = workflow.clone();
                                }
                            }
                        });
                    
                    // Auto-load workflow when selected from dropdown
                    if workflow_selected {
                        match get_workflow_graph(&selected_workflow) {
                            Ok(graph) => {
                                state.graph = Some(graph);
                                state.error.clear();
                            }
                            Err(e) => {
                                state.error = e;
                                state.graph = None;
                            }
                        }
                    }
                } else {
                    ui.colored_label(
                        Color32::from_rgb(255, 152, 0),
                        RichText::new("No workflows found").size(12.0),
                    );
                }
            });

            ui.add_space(8.0);

            // Action buttons grouped logically
            ui.horizontal(|ui| {
                // File operations group
                ui.group(|ui| {
                    ui.set_min_width(150.0);
                    ui.label(RichText::new("File Operations").size(12.0).weak());
                    if ui.add(egui::Button::new("🆕 New")).clicked() {
                        state.graph = Some(crate::backend::WorkflowGraph {
                            nodes: Vec::new(),
                            edges: Vec::new(),
                        });
                        state.workflow_path.clear();
                        state.error.clear();
                    }
                });

                ui.add_space(10.0);

                // Execution controls group
                ui.group(|ui| {
                    ui.set_min_width(300.0);
                    ui.label(RichText::new("Execution").size(12.0).weak());
                    
                    // Detect if workflow has parallelizable steps (any level with >1 node)
                    let has_parallel_steps = if let Some(ref graph) = state.graph {
                        let levels = crate::backend::calculate_execution_levels(graph);
                        levels.iter().any(|level| level.len() > 1)
                    } else {
                        false
                    };
                    
                    ui.horizontal(|ui| {
                        // Single Run button - automatically uses parallel execution when possible
                        let button_text = if has_parallel_steps {
                            "▶️ Run (Parallel)"
                        } else {
                            "▶️ Run"
                        };
                        
                        let button_color = if state.is_running || state.workflow_path.is_empty() {
                            Color32::from_gray(100)
                        } else if has_parallel_steps {
                            Color32::from_rgb(156, 39, 176) // Purple for parallel
                        } else {
                            Color32::from_rgb(33, 150, 243) // Blue for sequential
                        };
                        
                        let run_button = egui::Button::new(button_text).fill(button_color);
                        let run_response = ui.add(run_button);
                        let run_clicked = run_response.clicked();
                        
                        let tooltip_text = if has_parallel_steps {
                            "Run workflow with automatic parallel execution\n\nIndependent steps will run concurrently based on dependencies.\nThe workflow structure determines execution order.\n\nVisual guide:\n• Green edges = primary input\n• Purple edges = dependencies\n• L# = execution level"
                        } else {
                            "Run workflow sequentially\n\nAll steps will execute one at a time in dependency order."
                        };
                        run_response.on_hover_text(tooltip_text);
                        
                        if run_clicked
                            && !state.workflow_path.is_empty()
                            && !state.is_running
                        {
                            if let Some(ref graph) = state.graph {
                                let mut graph_clone = graph.clone();
                                for node in &mut graph_clone.nodes {
                                    node.status = "pending".to_string();
                                    node.message = None;
                                    node.output = None;
                                    node.error = None;
                                    node.attempt = 0;
                                }
                                state.graph = Some(graph_clone);
                            }
                            // Use debug mode (sequential) if enabled, otherwise auto-detect parallel execution
                            if state.debug_mode {
                                should_run = true; // Sequential for debugging
                            } else if has_parallel_steps {
                                should_run_parallel = true; // Parallel when possible
                            } else {
                                should_run = true; // Sequential when no parallelism
                            }
                        }
                        
                        // Optional: Debug mode toggle (forces sequential execution)
                        ui.add_space(5.0);
                        ui.separator();
                        ui.add_space(5.0);
                        let debug_label = if state.debug_mode {
                            "🐛 Debug (Sequential)"
                        } else {
                            "🐛 Debug"
                        };
                        if ui.checkbox(&mut state.debug_mode, debug_label).clicked() {
                            // Toggle handled by checkbox
                        }
                    });
                });
            });

            ui.add_space(8.0);

            // Settings button
            ui.horizontal(|ui| {
                if ui.button("⚙️ Settings").clicked() {
                    state.show_settings = true;
                }
            });
            
            ui.add_space(5.0);
            
            // Error display with better styling
            if !state.error.is_empty() {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        Color32::from_rgb(244, 67, 54),
                        RichText::new("⚠️").size(16.0),
                    );
                    ui.colored_label(
                        Color32::from_rgb(244, 67, 54),
                        RichText::new(&state.error).size(12.0),
                    );
                });
            }

            // Workflow summary (collapsible)
            if let Some(ref graph) = state.graph {
                ui.add_space(8.0);
                ui.collapsing(
                    RichText::new(format!("📊 Workflow Summary ({} nodes, {} connections)", 
                        graph.nodes.len(), graph.edges.len())).size(13.0),
                    |ui| {
                        ui.add_space(5.0);
                        
                        // Node list with status indicators
                        if !graph.nodes.is_empty() {
                            ui.label(RichText::new("Nodes:").size(12.0).strong());
                            for node in &graph.nodes {
                                let status_color = match node.status.as_str() {
                                    "running" => Color32::from_rgb(33, 150, 243),
                                    "success" => Color32::from_rgb(76, 175, 80),
                                    "error" => Color32::from_rgb(244, 67, 54),
                                    "cache" => Color32::from_rgb(156, 39, 176),
                                    _ => Color32::GRAY,
                                };

                                ui.horizontal(|ui| {
                                    ui.colored_label(status_color, "●");
                                    ui.label(RichText::new(&node.id).strong());
                                    ui.label(format!("({})", node.run));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(&node.status).weak().size(11.0));
                                    });
                                });
                            }
                        }

                        // Edge list
                        if !graph.edges.is_empty() {
                            ui.add_space(5.0);
                            ui.separator();
                            ui.add_space(5.0);
                            ui.label(RichText::new("Connections:").size(12.0).strong());
                            for edge in &graph.edges {
                                ui.horizontal(|ui| {
                                    ui.label("  →");
                                    ui.label(RichText::new(&edge.from).weak());
                                    ui.label("→");
                                    ui.label(RichText::new(&edge.to).weak());
                                });
                            }
                        }
                    },
                );
            }

            // Note about workflows location
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(5.0);
            let workflows_dir = resolve_workflows_dir();
            let workflows_full_path = std::path::Path::new(&workflows_dir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&workflows_dir))
                .display()
                .to_string();
            ui.horizontal(|ui| {
                ui.label(RichText::new("📂 Workflows location:").size(11.0).weak());
                ui.label(RichText::new(&workflows_full_path).size(11.0).weak().monospace());
            });
        } // End lock scope

        if should_run {
            let state = state_arc.lock().unwrap();
            let path = state.workflow_path.clone();
            drop(state); // Drop lock before async call
            let _ = run_workflow_stream(path, false, Arc::clone(state_arc));
        }

        if should_run_parallel {
            let state = state_arc.lock().unwrap();
            let path = state.workflow_path.clone();
            drop(state);
            let _ = run_workflow_stream(path, true, Arc::clone(state_arc));
        }
    });
}
