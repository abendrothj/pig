//! Execution timeline visualization - Gantt chart style view

use crate::metrics::StepMetric;
use chrono::{DateTime, Local};
use egui::{Color32, Pos2, Rect, Stroke};

/// Timeline visualization state
pub struct TimelineState {
    pub zoom_level: f32,
    pub scroll_offset: f32,
    pub show_details: bool,
    pub selected_step: Option<String>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            zoom_level: 1.0,
            scroll_offset: 0.0,
            show_details: true,
            selected_step: None,
        }
    }
}

/// Show timeline visualization
pub fn show_timeline(
    ui: &mut egui::Ui,
    steps: &[StepMetric],
    state: &mut TimelineState,
) {
    ui.group(|ui| {
        ui.heading("⏱️ Execution Timeline");
        
        // Controls
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            if ui.button("➕").clicked() {
                state.zoom_level *= 1.2;
            }
            if ui.button("➖").clicked() {
                state.zoom_level /= 1.2;
            }
            ui.label(format!("{:.0}%", state.zoom_level * 100.0));
            
            ui.separator();
            
            ui.checkbox(&mut state.show_details, "Show Details");
        });

        ui.separator();

        if steps.is_empty() {
            ui.label("No execution data available");
            return;
        }

        // Find time range
        let min_time = steps
            .iter()
            .map(|s| s.start_time)
            .min()
            .unwrap_or_else(Local::now);
        
        let max_time = steps
            .iter()
            .filter_map(|s| s.end_time)
            .max()
            .unwrap_or_else(Local::now);

        let total_duration = (max_time - min_time).num_milliseconds() as f64;
        
        if total_duration <= 0.0 {
            ui.label("Execution not started or too short to visualize");
            return;
        }

        // Draw timeline
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(
            egui::vec2(available.x, 40.0 * steps.len() as f32),
            egui::Sense::click_and_drag(),
        );

        let rect = response.rect;
        let time_width = rect.width() * state.zoom_level;
        
        // Background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(250, 250, 250));

        // Draw grid lines
        let grid_spacing = 100.0 * state.zoom_level;
        let mut x = rect.left();
        while x < rect.right() {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_rgb(230, 230, 230)),
            );
            x += grid_spacing;
        }

        // Draw each step as a bar
        for (i, step) in steps.iter().enumerate() {
            let y_pos = rect.top() + (i as f32 * 40.0) + 5.0;
            let bar_height = 30.0;

            // Calculate bar position and width
            let start_offset = (step.start_time - min_time).num_milliseconds() as f64;
            let x_start = rect.left() + (start_offset / total_duration * time_width as f64) as f32;
            
            let duration_ms = if let Some(end) = step.end_time {
                (end - step.start_time).num_milliseconds() as f64
            } else {
                (Local::now() - step.start_time).num_milliseconds() as f64
            };
            
            let bar_width = ((duration_ms / total_duration) * time_width as f64).max(2.0) as f32;

            // Color based on status
            let color = match step.status.as_str() {
                "success" => Color32::from_rgb(76, 175, 80),
                "error" => Color32::from_rgb(244, 67, 54),
                "running" => Color32::from_rgb(255, 193, 7),
                "cached" => Color32::from_rgb(156, 39, 176),
                _ => Color32::GRAY,
            };

            // Draw bar
            let bar_rect = Rect::from_min_size(
                Pos2::new(x_start, y_pos),
                egui::vec2(bar_width, bar_height),
            );

            painter.rect_filled(bar_rect, 2.0, color);
            painter.rect_stroke(bar_rect, 2.0, Stroke::new(1.0, Color32::BLACK));

            // Draw label if space allows
            if state.show_details && bar_width > 60.0 {
                let text = format!("{}", step.step_name);
                painter.text(
                    Pos2::new(x_start + 5.0, y_pos + bar_height / 2.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    egui::FontId::proportional(10.0),
                    Color32::WHITE,
                );
            }

            // Show duration
            if let Some(duration) = step.duration_ms {
                let duration_text = format!("{}ms", duration);
                painter.text(
                    Pos2::new(x_start + bar_width + 5.0, y_pos + bar_height / 2.0),
                    egui::Align2::LEFT_CENTER,
                    duration_text,
                    egui::FontId::proportional(9.0),
                    Color32::DARK_GRAY,
                );
            }
        }

        // Time labels at bottom
        painter.text(
            Pos2::new(rect.left(), rect.bottom() + 15.0),
            egui::Align2::LEFT_TOP,
            "0ms",
            egui::FontId::proportional(10.0),
            Color32::BLACK,
        );

        painter.text(
            Pos2::new(rect.right(), rect.bottom() + 15.0),
            egui::Align2::RIGHT_TOP,
            format!("{:.0}ms", total_duration),
            egui::FontId::proportional(10.0),
            Color32::BLACK,
        );
    });
}

/// Show compact timeline legend
pub fn show_timeline_legend(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Legend:");
        
        ui.colored_label(Color32::from_rgb(76, 175, 80), "■");
        ui.label("Success");
        
        ui.colored_label(Color32::from_rgb(255, 193, 7), "■");
        ui.label("Running");
        
        ui.colored_label(Color32::from_rgb(244, 67, 54), "■");
        ui.label("Error");
        
        ui.colored_label(Color32::from_rgb(156, 39, 176), "■");
        ui.label("Cached");
    });
}
