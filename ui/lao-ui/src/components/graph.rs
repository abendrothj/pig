use crate::backend::{
    export_workflow_yaml, save_workflow_yaml, calculate_execution_levels, auto_layout_graph_hierarchical, GraphEdge, GraphNode, UiPluginInfo, WorkflowGraph,
};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Stroke, Ui, Vec2, RichText};

pub struct GraphEditorState {
    pub pan_offset: Vec2,
    pub connecting_from: Option<String>,
    pub selected_node: Option<String>,
    pub connect_mode_active: bool,
    pub show_node_input_selector: bool,

    // Editor UI state
    pub new_node_name: String,
    pub new_node_type: String,

    // Dialog state
    pub show_save_dialog: bool,
    pub show_export_dialog: bool,
    pub new_workflow_filename: String,
}

impl Default for GraphEditorState {
    fn default() -> Self {
        Self {
            pan_offset: Vec2::ZERO,
            connecting_from: None,
            selected_node: None,
            connect_mode_active: false,
            show_node_input_selector: false,
            new_node_name: String::new(),
            new_node_type: "EchoPlugin".to_string(), // Default safe value
            show_save_dialog: false,
            show_export_dialog: false,
            new_workflow_filename: "new_workflow.yaml".to_string(),
        }
    }
}

pub fn show(
    ui: &mut Ui,
    graph: &mut WorkflowGraph,
    state: &mut GraphEditorState,
    plugins: &[UiPluginInfo],
) {
    ui.group(|ui| {
        ui.heading(RichText::new("🎨 Visual Flow Builder").size(18.0));

        // Toolbar with grouped buttons
        ui.horizontal(|ui| {
            // File operations
            ui.group(|ui| {
                ui.set_min_width(180.0);
                if ui.add(egui::Button::new("🆕 New")).clicked() {
                    graph.nodes.clear();
                    graph.edges.clear();
                    state.selected_node = None;
                }
                if ui.add(egui::Button::new("💾 Save")).clicked() {
                    state.show_save_dialog = true;
                }
                if ui.add(egui::Button::new("📤 Export")).clicked() {
                    state.show_export_dialog = true;
                }
            });

            ui.add_space(8.0);

            // Connection controls
            ui.group(|ui| {
                ui.set_min_width(120.0);
                if ui.add(egui::Button::new("🔗 Connect")
                    .fill(if state.connect_mode_active {
                        Color32::from_rgb(255, 193, 7)
                    } else {
                        Color32::TRANSPARENT
                    })).clicked() {
                    state.connect_mode_active = true;
                    state.connecting_from = None;
                }
                if ui
                    .add(egui::Button::new("🗑️ Clear")
                        .fill(Color32::from_rgb(244, 67, 54)))
                    .clicked()
                {
                    graph.nodes.clear();
                    graph.edges.clear();
                    state.selected_node = None;
                }
            });

            ui.add_space(8.0);

            // Layout controls
            ui.group(|ui| {
                ui.set_min_width(140.0);
                if ui.add(egui::Button::new("📐 Auto-Layout")
                    .fill(Color32::from_rgb(33, 150, 243))).clicked() {
                    auto_layout_graph_hierarchical(graph);
                }
            });

            ui.add_space(10.0);

            // Status/hints area
            if state.connect_mode_active {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            Color32::from_rgb(255, 193, 7),
                            RichText::new("🔗").size(16.0),
                        );
                        if state.connecting_from.is_some() {
                            ui.label(RichText::new("Click target node to connect").size(12.0));
                        } else {
                            ui.label(RichText::new("Click source node first").size(12.0));
                        }
                        if ui.small_button("❌ Cancel").clicked() {
                            state.connect_mode_active = false;
                            state.connecting_from = None;
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        Color32::from_gray(150),
                        RichText::new("💡").size(12.0),
                    );
                    ui.label(RichText::new("Left-click: select • Drag: move • Right-click edge: delete").size(11.0).weak());
                    ui.separator();
                    ui.colored_label(
                        Color32::from_rgb(76, 175, 80),
                        RichText::new("Green").size(11.0),
                    );
                    ui.label(RichText::new("= primary •").size(11.0).weak());
                    ui.colored_label(
                        Color32::from_rgb(156, 39, 176),
                        RichText::new("Purple").size(11.0),
                    );
                    ui.label(RichText::new("= dependencies •").size(11.0).weak());
                    ui.colored_label(
                        Color32::from_rgb(255, 193, 7),
                        RichText::new("L#").size(11.0),
                    );
                    ui.label(RichText::new("= level").size(11.0).weak());
                });
            }
        });

        ui.add_space(8.0);

        // Save dialog
        if state.show_save_dialog {
            let mut close_dialog = false;
            egui::Window::new("Save Workflow")
                .open(&mut state.show_save_dialog)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Filename:");
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_workflow_filename)
                                .id_source("save_workflow_filename_input"),
                        );
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            match save_workflow_yaml(graph, &state.new_workflow_filename) {
                                Ok(_) => {
                                    close_dialog = true;
                                }
                                Err(e) => {
                                    eprintln!("Save error: {}", e);
                                }
                            }
                        }

                        if ui.button("Cancel").clicked() {
                            close_dialog = true;
                        }
                    });
                });

            if close_dialog {
                state.show_save_dialog = false;
            }
        }

        // Export dialog
        if state.show_export_dialog {
            let mut close_dialog = false;
            egui::Window::new("Export YAML")
                .open(&mut state.show_export_dialog)
                .show(ui.ctx(), |ui| {
                    match export_workflow_yaml(graph) {
                        Ok(yaml) => {
                            ui.label("Generated YAML:");
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut yaml.clone())
                                            .id_source("export_yaml_text_input"),
                                    );
                                });
                        }
                        Err(e) => {
                            ui.colored_label(Color32::RED, format!("Export error: {}", e));
                        }
                    }

                    if ui.button("Close").clicked() {
                        close_dialog = true;
                    }
                });

            if close_dialog {
                state.show_export_dialog = false;
            }
        }

        // Add node controls with better layout
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Add Node:").size(13.0).strong());
                
                ui.add(
                    egui::TextEdit::singleline(&mut state.new_node_name)
                        .hint_text("Node name (optional)")
                        .desired_width(150.0)
                        .id_source("new_node_name_input"),
                );

                ui.label("→");

                egui::ComboBox::from_id_salt("plugin_type_combo")
                    .width(180.0)
                    .selected_text(&state.new_node_type)
                    .show_ui(ui, |ui| {
                        for (i, plugin) in plugins.iter().enumerate() {
                            ui.push_id(format!("plugin_option_{}", i), |ui| {
                                ui.selectable_value(
                                    &mut state.new_node_type,
                                    plugin.name.clone(),
                                    &plugin.name,
                                );
                            });
                        }
                    });

                if ui
                    .add(
                        egui::Button::new("➕ Add")
                            .fill(Color32::from_rgb(76, 175, 80)),
                    )
                    .clicked()
                {
                    let node_id = if state.new_node_name.is_empty() {
                        format!("node_{}", graph.nodes.len() + 1)
                    } else {
                        state.new_node_name.clone()
                    };

                    // Calculate better initial position
                    let node_count = graph.nodes.len();
                    let cols = 4;
                    let col = node_count % cols;
                    let row = node_count / cols;
                    let spacing_x = 200.0;
                    let spacing_y = 120.0;

                    graph.nodes.push(GraphNode {
                        id: node_id,
                        run: state.new_node_type.clone(),
                        input_type: None,
                        output_type: None,
                        status: "pending".to_string(),
                        x: 50.0 + (col as f32 * spacing_x),
                        y: 50.0 + (row as f32 * spacing_y),
                        message: None,
                        output: None,
                        error: None,
                        attempt: 0,
                        primary_input: None,
                        execution_level: None,
                    });

                    state.new_node_name.clear();
                }
            });
        });

        ui.add_space(8.0);

        // Visual graph area - calculate height based on execution levels
        let available_rect = ui.available_rect_before_wrap();
        let levels = calculate_execution_levels(graph);
        let level_height = 150.0;
        let level_spacing = 20.0;
        let min_height = 400.0;
        let calculated_height = if levels.is_empty() {
            min_height
        } else {
            let total_levels_height = (levels.len() as f32 * (level_height + level_spacing)) + 100.0; // +100 for padding
            total_levels_height.max(min_height)
        };
        
        let graph_rect = Rect::from_min_size(
            available_rect.min,
            egui::vec2(available_rect.width(), calculated_height),
        );

        let response = ui.allocate_rect(graph_rect, egui::Sense::click_and_drag());
        
        // Store graph_rect for popup positioning
        let graph_rect_for_popup = graph_rect;

        if ui.is_rect_visible(graph_rect) {
            let painter = ui.painter();

            // Draw background
            painter.rect_filled(graph_rect, 4.0, Color32::from_gray(248));
            
            // Show empty state if no nodes
            if graph.nodes.is_empty() {
                let empty_rect = Rect::from_min_size(
                    graph_rect.center(),
                    egui::vec2(400.0, 150.0),
                ).translate(-egui::vec2(200.0, 75.0));
                
                // Draw empty state text directly with painter
                painter.text(
                    empty_rect.center() - egui::vec2(0.0, 40.0),
                    egui::Align2::CENTER_CENTER,
                    "🎨",
                    egui::FontId::proportional(48.0),
                    Color32::from_gray(150),
                );
                painter.text(
                    empty_rect.center() - egui::vec2(0.0, 10.0),
                    egui::Align2::CENTER_CENTER,
                    "Empty Workflow",
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
                painter.text(
                    empty_rect.center() + egui::vec2(0.0, 20.0),
                    egui::Align2::CENTER_CENTER,
                    "Add nodes above to start building your workflow",
                    egui::FontId::proportional(12.0),
                    Color32::from_gray(150),
                );
                return;
            }

            // Draw grid (respecting pan)
            let grid_size = 40.0;
            let cols = (graph_rect.width() / grid_size).ceil() as i32;
            let rows = (graph_rect.height() / grid_size).ceil() as i32;

            // Clip drawing to graph rect
            let painter = painter.with_clip_rect(graph_rect);

            // Draw execution level bands (background regions for each level)
            // Use the levels calculated above for height calculation
            let start_y = 50.0;
            
            for (level_idx, level_nodes) in levels.iter().enumerate() {
                if !level_nodes.is_empty() {
                    let band_y = start_y + (level_idx as f32 * (level_height + level_spacing));
                    // Respect pan offset for bands and labels
                    let band_y_with_pan = band_y + state.pan_offset.y;
                    let band_rect = Rect::from_min_max(
                        Pos2::new(graph_rect.min.x + state.pan_offset.x, graph_rect.min.y + band_y_with_pan - 10.0),
                        Pos2::new(graph_rect.max.x + state.pan_offset.x, graph_rect.min.y + band_y_with_pan + level_height - 10.0),
                    );
                    
                    // Draw subtle background band for each execution level
                    let band_color = if level_idx % 2 == 0 {
                        Color32::from_rgba_unmultiplied(33, 150, 243, 8) // Light blue tint
                    } else {
                        Color32::from_rgba_unmultiplied(156, 39, 176, 8) // Light purple tint
                    };
                    painter.rect_filled(band_rect, 0.0, band_color);
                    
                    // Draw level label on the left (respects pan offset)
                    painter.text(
                        Pos2::new(graph_rect.min.x + state.pan_offset.x + 10.0, graph_rect.min.y + band_y_with_pan + 10.0),
                        egui::Align2::LEFT_CENTER,
                        format!("Level {}", level_idx),
                        egui::FontId::proportional(10.0),
                        Color32::from_gray(120),
                    );
                }
            }

            for i in 0..cols {
                let x = graph_rect.min.x + (state.pan_offset.x % grid_size) + i as f32 * grid_size;
                painter.line_segment(
                    [
                        Pos2::new(x, graph_rect.min.y),
                        Pos2::new(x, graph_rect.max.y),
                    ],
                    Stroke::new(1.0, Color32::from_gray(238)),
                );
            }
            for j in 0..rows {
                let y = graph_rect.min.y + (state.pan_offset.y % grid_size) + j as f32 * grid_size;
                painter.line_segment(
                    [
                        Pos2::new(graph_rect.min.x, y),
                        Pos2::new(graph_rect.max.x, y),
                    ],
                    Stroke::new(1.0, Color32::from_gray(238)),
                );
            }

            // Draw edges
            let mut edge_to_delete: Option<usize> = None;
            for (i, edge) in graph.edges.iter().enumerate() {
                if let (Some(from_node), Some(to_node)) = (
                    graph.nodes.iter().find(|n| n.id == edge.from),
                    graph.nodes.iter().find(|n| n.id == edge.to),
                ) {
                    let from_pos = Pos2::new(
                        graph_rect.min.x + state.pan_offset.x + from_node.x + 120.0,
                        graph_rect.min.y + state.pan_offset.y + from_node.y + 30.0,
                    );
                    let to_pos = Pos2::new(
                        graph_rect.min.x + state.pan_offset.x + to_node.x,
                        graph_rect.min.y + state.pan_offset.y + to_node.y + 30.0,
                    );

                    // Determine if this is a primary input edge or dependency edge
                    let is_primary = to_node.primary_input.as_ref() == Some(&edge.from);
                    let edge_color = if is_primary {
                        Color32::from_rgb(76, 175, 80) // Green for primary input
                    } else {
                        Color32::from_rgb(156, 39, 176) // Purple for dependencies (parallel)
                    };
                    let edge_width = if is_primary { 3.0 } else { 2.0 };

                    // Draw curved edge for better visualization (especially for parallel flows)
                    let mid_x = (from_pos.x + to_pos.x) / 2.0;
                    let control_offset = 30.0;
                    let control1 = Pos2::new(mid_x, from_pos.y + control_offset);
                    let control2 = Pos2::new(mid_x, to_pos.y - control_offset);
                    
                    // Draw curved bezier path
                    let steps = 20;
                    let mut prev_point = from_pos;
                    for i in 1..=steps {
                        let t = i as f32 / steps as f32;
                        let point = bezier_point(from_pos, control1, control2, to_pos, t);
                        painter.line_segment([prev_point, point], Stroke::new(edge_width, edge_color));
                        prev_point = point;
                    }

                    // Draw arrowhead at the end of the curve
                    let final_direction = (to_pos - prev_point).normalized();
                    let arrow_size = 8.0;
                    let arrow_tip = to_pos - final_direction * 5.0;
                    let perpendicular = Vec2::new(-final_direction.y, final_direction.x);

                    let arrow_p1 =
                        arrow_tip - final_direction * arrow_size + perpendicular * arrow_size * 0.5;
                    let arrow_p2 =
                        arrow_tip - final_direction * arrow_size - perpendicular * arrow_size * 0.5;

                    painter.line_segment(
                        [arrow_tip, arrow_p1],
                        Stroke::new(edge_width, edge_color),
                    );
                    painter.line_segment(
                        [arrow_tip, arrow_p2],
                        Stroke::new(edge_width, edge_color),
                    );

                    // Check for edge click to delete
                    let edge_center = (from_pos + to_pos.to_vec2()) * 0.5;
                    let edge_rect = Rect::from_center_size(edge_center, Vec2::splat(20.0));
                    let edge_response = ui.interact(
                        edge_rect,
                        Id::new(format!("edge_{}", i)),
                        egui::Sense::click(),
                    );
                    if edge_response.secondary_clicked() {
                        edge_to_delete = Some(i);
                    }
                }
            }
            if let Some(idx) = edge_to_delete {
                if idx < graph.edges.len() {
                    graph.edges.remove(idx);
                }
            }

            // Count plugin instances for display (before mutable borrow)
            let plugin_counts: std::collections::HashMap<String, usize> = graph.nodes
                .iter()
                .map(|n| n.run.clone())
                .fold(std::collections::HashMap::new(), |mut acc, plugin| {
                    *acc.entry(plugin).or_insert(0) += 1;
                    acc
                });

            // Draw nodes
            let mut node_clicked = None;
            for node in &mut graph.nodes {
                let node_pos = Pos2::new(
                    graph_rect.min.x + state.pan_offset.x + node.x,
                    graph_rect.min.y + state.pan_offset.y + node.y,
                );
                let node_rect = Rect::from_min_size(node_pos, egui::vec2(120.0, 60.0));

                // Node background color based on status
                let node_color = match node.status.as_str() {
                    "running" => Color32::from_rgb(33, 150, 243),
                    "success" => Color32::from_rgb(76, 175, 80),
                    "error" => Color32::from_rgb(244, 67, 54),
                    "cache" => Color32::from_rgb(156, 39, 176),
                    "pending" => Color32::from_rgb(96, 125, 139),
                    _ => Color32::from_rgb(34, 34, 34),
                };

                painter.rect_filled(node_rect, 12.0, node_color);

                // Highlight/Stroke
                if state.connecting_from.as_ref() == Some(&node.id) {
                    painter.rect_stroke(node_rect, 12.0, Stroke::new(3.0, Color32::YELLOW));
                } else if state.selected_node.as_ref() == Some(&node.id) {
                    painter.rect_stroke(node_rect, 12.0, Stroke::new(2.0, Color32::WHITE));
                } else {
                    painter.rect_stroke(node_rect, 12.0, Stroke::new(2.0, Color32::from_gray(68)));
                }

                // Display plugin name prominently with instance number if multiple exist
                let instance_num = node.id.split('_').last().unwrap_or("");
                let display_name = if instance_num.parse::<usize>().is_ok() 
                    && plugin_counts.get(&node.run).copied().unwrap_or(0) > 1 {
                    // Multiple instances of same plugin - show instance number
                    format!("{} #{}", node.run, instance_num)
                } else {
                    // Single instance or can't parse - just show plugin name
                    node.run.clone()
                };
                
                painter.text(
                    node_rect.center() - egui::vec2(0.0, 8.0),
                    egui::Align2::CENTER_CENTER,
                    &display_name,
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );

                // Display status below plugin name
                painter.text(
                    node_rect.center() + egui::vec2(0.0, 8.0),
                    egui::Align2::CENTER_CENTER,
                    &node.status,
                    egui::FontId::proportional(10.0),
                    Color32::from_gray(200),
                );

                // Visual indicators for parallel nodes (fan-in/fan-out)
                let incoming_count = graph.edges.iter().filter(|e| e.to == node.id).count();
                let outgoing_count = graph.edges.iter().filter(|e| e.from == node.id).count();
                
                // Fan-in indicator (multiple inputs = can run in parallel)
                if incoming_count > 1 {
                    painter.circle_filled(
                        node_rect.min + egui::vec2(5.0, 5.0),
                        4.0,
                        Color32::from_rgb(156, 39, 176), // Purple for parallel inputs
                    );
                }
                
                // Fan-out indicator (multiple outputs = can spawn parallel branches)
                if outgoing_count > 1 {
                    painter.circle_filled(
                        node_rect.max - egui::vec2(5.0, 5.0),
                        4.0,
                        Color32::from_rgb(33, 150, 243), // Blue for parallel outputs
                    );
                }
                
                // Execution level indicator (for parallel execution visualization)
                if let Some(level) = node.execution_level {
                    painter.text(
                        node_rect.min + egui::vec2(8.0, 18.0),
                        egui::Align2::LEFT_TOP,
                        format!("L{}", level),
                        egui::FontId::proportional(8.0),
                        Color32::from_rgb(255, 193, 7), // Amber for level indicator
                    );
                }

                let node_response =
                    ui.interact(node_rect, Id::new(&node.id), egui::Sense::click_and_drag());

                // Left-click: connection logic or select node (no inspector)
                if node_response.clicked() {
                    if state.connect_mode_active {
                        // Two-step connection: first click sets source, second creates edge
                        if let Some(ref from_id) = state.connecting_from {
                            // Second step: create edge from source to this node
                            if from_id != &node.id {
                                let edge = GraphEdge {
                                    from: from_id.clone(),
                                    to: node.id.clone(),
                                };
                                if !graph
                                    .edges
                                    .iter()
                                    .any(|e| e.from == edge.from && e.to == edge.to)
                                {
                                    graph.edges.push(edge);
                                }
                            }
                            // Exit connect mode after creating edge
                            state.connect_mode_active = false;
                            state.connecting_from = None;
                        } else {
                            // First step: set this node as source
                            state.connecting_from = Some(node.id.clone());
                        }
                    } else {
                        // Normal mode: just select the node
                        node_clicked = Some(node.id.clone());
                    }
                }

                // Right-click: currently no special behavior (no menus)

                if node_response.dragged() && !state.connect_mode_active {
                    let drag_delta = node_response.drag_delta();
                    node.x += drag_delta.x;
                    node.y += drag_delta.y;
                }
            }

            if let Some(click_id) = node_clicked {
                state.selected_node = Some(click_id);
                state.show_node_input_selector = true;
            }

            // Pan interaction
            if response.dragged() {
                state.pan_offset += response.drag_delta();
            }
        }

        // Show lightweight input selector popup for selected node
        if let Some(ref selected_id) = state.selected_node {
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == *selected_id) {
                // Find incoming edges for this node
                let incoming: Vec<String> = graph
                    .edges
                    .iter()
                    .filter(|e| e.to == node.id)
                    .map(|e| e.from.clone())
                    .collect();

                if !incoming.is_empty() && state.show_node_input_selector {
                    // Calculate node position for popup placement
                    let node_pos = Pos2::new(
                        graph_rect_for_popup.min.x + state.pan_offset.x + node.x + 130.0,
                        graph_rect_for_popup.min.y + state.pan_offset.y + node.y,
                    );

                    egui::Window::new("Select Primary Input")
                        .collapsible(false)
                        .resizable(false)
                        .default_pos(node_pos)
                        .show(ui.ctx(), |ui| {
                            ui.heading(RichText::new(format!("{}", node.run)).size(16.0));
                            ui.add_space(5.0);
                            ui.label(RichText::new("Choose which input provides the primary data flow:").size(12.0).weak());
                            ui.add_space(8.0);

                            let mut current_primary = node.primary_input.clone();
                            for source_id in &incoming {
                                let is_selected = current_primary.as_ref() == Some(source_id);
                                ui.horizontal(|ui| {
                                    if is_selected {
                                        ui.colored_label(Color32::from_rgb(76, 175, 80), "✓");
                                    } else {
                                        ui.label(" ");
                                    }
                                    if ui.selectable_label(is_selected, source_id).clicked() {
                                        current_primary = Some(source_id.clone());
                                    }
                                });
                            }

                            ui.add_space(5.0);
                            ui.separator();
                            ui.add_space(5.0);
                            
                            if ui.button("None (no primary input)").clicked() {
                                current_primary = None;
                            }

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new("✓ Done").fill(Color32::from_rgb(76, 175, 80))).clicked() {
                                    node.primary_input = current_primary;
                                    state.show_node_input_selector = false;
                                }
                                if ui.button("Cancel").clicked() {
                                    state.show_node_input_selector = false;
                                }
                            });
                        });
                }
            }
        }
    });
}

/// Calculate a point on a cubic Bezier curve
fn bezier_point(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;
    
    Pos2::new(
        mt3 * p0.x + 3.0 * mt2 * t * p1.x + 3.0 * mt * t2 * p2.x + t3 * p3.x,
        mt3 * p0.y + 3.0 * mt2 * t * p1.y + 3.0 * mt * t2 * p2.y + t3 * p3.y,
    )
}

