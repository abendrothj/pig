use eframe::egui::{self, Color32, RichText, Ui};

pub fn show(
    ui: &mut Ui,
    logs: &mut Vec<String>,
    is_running: bool,
    execution_progress: f32,
    workflow_result: &Option<crate::backend::WorkflowResult>,
) {
    ui.group(|ui| {
        ui.heading("📊 Live Logs & Execution Status");

        // Show execution status indicator with better design
        ui.horizontal(|ui| {
            if is_running {
                ui.spinner();
                ui.colored_label(
                    Color32::from_rgb(33, 150, 243),
                    RichText::new("🔄 Workflow Executing").size(14.0),
                );
                ui.add(
                    egui::ProgressBar::new(execution_progress)
                        .show_percentage()
                        .fill(Color32::from_rgb(33, 150, 243)),
                );
            } else if let Some(ref result) = workflow_result {
                if result.success {
                    ui.colored_label(
                        Color32::from_rgb(76, 175, 80),
                        RichText::new("✅ Execution Complete").size(14.0),
                    );
                } else {
                    ui.colored_label(
                        Color32::from_rgb(244, 67, 54),
                        RichText::new("❌ Execution Failed").size(14.0),
                    );
                }
            } else {
                ui.colored_label(Color32::GRAY, RichText::new("⏸️ Ready").size(14.0));
            }
        });

        // Show parallel execution metrics if available
        if let Some(ref result) = workflow_result {
            if let Some(ref metrics) = result.parallel_execution {
                ui.add_space(5.0);
                ui.separator();
                ui.add_space(5.0);
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            Color32::from_rgb(156, 39, 176),
                            RichText::new("⚡ Parallel Execution Metrics").size(13.0).strong(),
                        );
                    });
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Levels:").size(11.0).weak());
                        ui.label(RichText::new(format!("{}", metrics.execution_levels)).strong());
                        ui.add_space(15.0);
                        ui.label(RichText::new("Max concurrent:").size(11.0).weak());
                        ui.label(RichText::new(format!("{}", metrics.max_parallelism)).strong());
                        ui.add_space(15.0);
                        ui.label(RichText::new("Avg concurrent:").size(11.0).weak());
                        ui.label(RichText::new(format!("{:.1}", metrics.average_parallelism)).strong());
                        ui.add_space(15.0);
                        ui.colored_label(
                            Color32::from_rgb(76, 175, 80),
                            RichText::new(format!("{:.1}x speedup", metrics.speedup)).strong(),
                        );
                    });
                });
            }
        }

        ui.add_space(10.0);

        ui.add_space(8.0);

        // Log controls with better styling
        ui.horizontal(|ui| {
            ui.label(RichText::new("📝 Execution Logs").size(14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("🗑️ Clear").small()).clicked() {
                    logs.clear();
                }
            });
        });

        // Live logs display with improved styling
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for log in logs.iter() {
                    // Color code based on log content with better colors
                    let (color, icon) = if log.contains("✓ DONE") {
                        (Color32::from_rgb(76, 175, 80), "✅")
                    } else if log.contains("✗ ERROR") {
                        (Color32::from_rgb(244, 67, 54), "❌")
                    } else if log.contains("running") {
                        (Color32::from_rgb(33, 150, 243), "🔄")
                    } else if log.contains("success") || log.contains("cache") {
                        (Color32::from_rgb(76, 175, 80), "✅")
                    } else if log.contains("error") || log.contains("failed") {
                        (Color32::from_rgb(244, 67, 54), "❌")
                    } else {
                        (Color32::WHITE, "ℹ️")
                    };

                    ui.horizontal(|ui| {
                        ui.label(icon);
                        ui.colored_label(color, log);
                    });
                }

                if logs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.colored_label(
                            Color32::GRAY,
                            RichText::new(
                                "No logs yet. Run a workflow to see execution logs here.",
                            )
                            .size(12.0),
                        );
                    });
                }
            });
    });
}
