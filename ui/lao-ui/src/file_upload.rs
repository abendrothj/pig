//! File drag-drop and attachment system for workflow nodes

use std::path::PathBuf;

/// File attachment associated with a workflow node
#[derive(Debug, Clone)]
pub struct FileAttachment {
    pub node_id: String,
    pub file_path: PathBuf,
    pub file_name: String,
    pub file_size: u64,
    pub mime_type: Option<String>,
    pub modality: Option<String>, // Detected modality: "audio", "image", "video", "text", etc.
}

/// State for managing file drag-drop and attachments
#[derive(Default)]
pub struct FileDropState {
    pub dropped_files: Vec<FileAttachment>,
    pub show_file_picker: bool,
    pub selected_node_for_file: Option<String>,
}

impl FileDropState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get files attached to a specific node
    pub fn get_node_files(&self, node_id: &str) -> Vec<&FileAttachment> {
        self.dropped_files
            .iter()
            .filter(|f| f.node_id == node_id)
            .collect()
    }

    /// Remove a file attachment
    pub fn remove_file(&mut self, node_id: &str, file_path: &PathBuf) {
        self.dropped_files
            .retain(|f| !(f.node_id == node_id && &f.file_path == file_path));
    }

    /// Clear all files for a node
    pub fn clear_node_files(&mut self, node_id: &str) {
        self.dropped_files.retain(|f| f.node_id != node_id);
    }
}

/// Detect modality from file extension or MIME type
fn detect_modality(file_name: &str, mime_type: Option<&str>) -> Option<String> {
    // Check MIME type first
    if let Some(mime) = mime_type {
        if mime.starts_with("audio/") {
            return Some("audio".to_string());
        } else if mime.starts_with("image/") {
            return Some("image".to_string());
        } else if mime.starts_with("video/") {
            return Some("video".to_string());
        } else if mime.starts_with("text/") || mime.contains("json") || mime.contains("yaml") {
            return Some("text".to_string());
        }
    }

    // Check file extension
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" => Some("audio".to_string()),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" => Some("image".to_string()),
        "mp4" | "avi" | "mov" | "mkv" | "webm" | "flv" => Some("video".to_string()),
        "txt" | "json" | "yaml" | "yml" | "csv" | "md" => Some("text".to_string()),
        _ => None,
    }
}

/// Handle dropped files from egui
pub fn handle_dropped_files(
    ctx: &egui::Context,
    node_id: &str,
    file_state: &mut FileDropState,
) {
    ctx.input(|i| {
        if !i.raw.dropped_files.is_empty() {
            for file in &i.raw.dropped_files {
                if let Some(path) = &file.path {
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let file_size = std::fs::metadata(path)
                        .map(|m| m.len())
                        .unwrap_or(0);

                    let modality = detect_modality(&file_name, Some(&file.mime));

                    file_state.dropped_files.push(FileAttachment {
                        node_id: node_id.to_string(),
                        file_path: path.clone(),
                        file_name,
                        file_size,
                        mime_type: Some(file.mime.clone()),
                        modality,
                    });
                }
            }
        }
    });
}

/// Show file attachments UI for a node
pub fn show_file_attachments(
    ui: &mut egui::Ui,
    node_id: &str,
    file_state: &mut FileDropState,
) {
    ui.group(|ui| {
        ui.label("📎 Attached Files:");

        let files: Vec<_> = file_state.get_node_files(node_id)
            .iter()
            .map(|f| (*f).clone())
            .collect();

        if files.is_empty() {
            ui.label("No files attached. Drag files here or click Browse.");
        } else {
            for file in &files {
                ui.horizontal(|ui| {
                    // Show modality icon
                    let modality_icon = match file.modality.as_deref() {
                        Some("audio") => "🔊",
                        Some("image") => "🖼️",
                        Some("video") => "🎬",
                        Some("text") => "📄",
                        _ => "📁",
                    };
                    ui.label(modality_icon);
                    
                    ui.label(&file.file_name);
                    ui.label(format!("({})", format_file_size(file.file_size)));

                    if ui.button("❌").clicked() {
                        file_state.remove_file(node_id, &file.file_path);
                    }
                });
            }
        }

        if ui.button("📁 Browse...").clicked() {
            file_state.show_file_picker = true;
            file_state.selected_node_for_file = Some(node_id.to_string());
        }
    });
}

/// Show file picker dialog
pub fn show_file_picker_dialog(ctx: &egui::Context, file_state: &mut FileDropState) {
    if file_state.show_file_picker {
        if let Some(node_id) = &file_state.selected_node_for_file {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let file_size = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(0);

                let modality = detect_modality(&file_name, None);

                file_state.dropped_files.push(FileAttachment {
                    node_id: node_id.clone(),
                    file_path: path,
                    file_name,
                    file_size,
                    mime_type: None,
                    modality,
                });
            }

            file_state.show_file_picker = false;
            file_state.selected_node_for_file = None;
        }
    }
}

/// Format file size in human-readable format
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
