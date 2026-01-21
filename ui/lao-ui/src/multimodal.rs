//! Multimodal workflow visualization and analysis

use lao_orchestrator_core::Modality;

/// Modality information for visualization
#[derive(Clone, Debug)]
pub struct ModalityInfo {
    pub input_modality: Option<Modality>,
    pub output_modality: Option<Modality>,
    pub data_flow: Vec<(String, String)>, // (step_id, modality_type)
}

/// Show modality flow visualization
pub fn show_modality_flow(ui: &mut egui::Ui, modality_info: &ModalityInfo) {
    ui.group(|ui| {
        ui.heading("🔄 Modality Flow");

        if let Some(input) = modality_info.input_modality {
            ui.horizontal(|ui| {
                ui.label("Input:");
                show_modality_badge(ui, input);
            });
        }

        if !modality_info.data_flow.is_empty() {
            ui.separator();
            ui.label("Pipeline:");
            for (i, (step_id, modality_str)) in modality_info.data_flow.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", i + 1));
                    ui.label(step_id);
                    ui.colored_label(get_modality_color(modality_str), modality_str);
                });
            }
        }

        if let Some(output) = modality_info.output_modality {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Output:");
                show_modality_badge(ui, output);
            });
        }
    });
}

/// Show modality badge with color
pub fn show_modality_badge(ui: &mut egui::Ui, modality: Modality) {
    let (color, icon) = match modality {
        Modality::Text => (egui::Color32::from_rgb(100, 150, 255), "📝 Text"),
        Modality::Audio => (egui::Color32::from_rgb(200, 150, 100), "🔊 Audio"),
        Modality::Image => (egui::Color32::from_rgb(255, 200, 100), "🖼️ Image"),
        Modality::Video => (egui::Color32::from_rgb(200, 100, 255), "🎬 Video"),
        Modality::Structured => (egui::Color32::from_rgb(100, 255, 150), "📊 JSON"),
        Modality::Binary => (egui::Color32::GRAY, "🔲 Binary"),
        Modality::Mixed => (egui::Color32::from_rgb(150, 150, 150), "🔀 Mixed"),
    };

    ui.colored_label(color, icon);
}

/// Get color for modality text
pub fn get_modality_color(modality: &str) -> egui::Color32 {
    match modality {
        "text" => egui::Color32::from_rgb(100, 150, 255),
        "audio" => egui::Color32::from_rgb(200, 150, 100),
        "image" => egui::Color32::from_rgb(255, 200, 100),
        "video" => egui::Color32::from_rgb(200, 100, 255),
        "structured" => egui::Color32::from_rgb(100, 255, 150),
        "binary" => egui::Color32::GRAY,
        _ => egui::Color32::from_rgb(150, 150, 150),
    }
}

/// Validate modality compatibility between connected steps
pub fn validate_modality_flow(
    from_modality: Option<Modality>,
    to_modality: Option<Modality>,
) -> Result<(), String> {
    match (from_modality, to_modality) {
        // Allow any output modality to connect to any input modality
        // (plugins handle conversion)
        (Some(_), Some(_)) => Ok(()),
        _ => Ok(()),
    }
}

/// Show modality compatibility warning
pub fn show_modality_warning(ui: &mut egui::Ui, from: &str, to: &str) {
    let warning = format!(
        "⚠️ Converting {} to {} may lose information",
        from, to
    );
    ui.colored_label(egui::Color32::YELLOW, warning);
}
