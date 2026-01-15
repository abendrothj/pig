use crate::backend::{BackendState, download_whisper_cpp, AppSettings};
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::{Arc, Mutex};
use std::path::Path;

pub fn show(ctx: &egui::Context, state_arc: &Arc<Mutex<BackendState>>) {
    let mut state = state_arc.lock().unwrap();
    
    egui::Window::new("⚙️ Settings")
        .collapsible(false)
        .resizable(true)
        .default_size([500.0, 400.0])
        .show(ctx, |ui| {
            ui.heading(RichText::new("Plugin Configuration").size(18.0));
            ui.add_space(10.0);
            
            // Whisper.cpp Configuration
            ui.group(|ui| {
                ui.heading(RichText::new("🎤 Whisper.cpp").size(16.0));
                ui.add_space(5.0);
                
                ui.label(RichText::new("Path to whisper.cpp binary:").size(12.0));
                ui.horizontal(|ui| {
                    let mut path = state.settings.whisper_cpp_path.clone();
                    let response = ui.text_edit_singleline(&mut path);
                    
                    if response.changed() {
                        state.settings.whisper_cpp_path = path.clone();
                    }
                    
                    // Browse button
                    if ui.button("📁 Browse").clicked() {
                        if let Some(selected_path) = rfd::FileDialog::new()
                            .set_title("Select whisper.cpp binary")
                            .pick_file()
                        {
                            state.settings.whisper_cpp_path = selected_path.to_string_lossy().to_string();
                        }
                    }
                    
                    // Auto-detect button
                    if ui.button("🔍 Auto-detect").clicked() {
                        if let Some(detected) = auto_detect_whisper() {
                            state.settings.whisper_cpp_path = detected;
                        } else {
                            state.error = "Could not auto-detect whisper.cpp. Please install it or set the path manually.".to_string();
                        }
                    }
                });
                
                // Show current status
                if !state.settings.whisper_cpp_path.is_empty() {
                    let path_exists = Path::new(&state.settings.whisper_cpp_path).exists();
                    if path_exists {
                        ui.colored_label(
                            Color32::from_rgb(76, 175, 80),
                            RichText::new(format!("✓ Found: {}", state.settings.whisper_cpp_path)).size(11.0)
                        );
                    } else {
                        ui.colored_label(
                            Color32::from_rgb(244, 67, 54),
                            RichText::new(format!("✗ Not found: {}", state.settings.whisper_cpp_path)).size(11.0)
                        );
                    }
                } else {
                    ui.colored_label(
                        Color32::from_gray(150),
                        RichText::new("No path set. WhisperPlugin will search common locations.").size(11.0)
                    );
                }
                
                ui.add_space(10.0);
                
                // Download/Build button
                ui.horizontal(|ui| {
                    if ui.button("⬇️ Download & Build whisper.cpp").clicked() {
                        let state_clone = state_arc.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = download_whisper_cpp() {
                                let mut state = state_clone.lock().unwrap();
                                state.error = format!("Failed to download whisper.cpp: {}", e);
                                state.live_logs.push(format!("❌ Download failed: {}", e));
                            } else {
                                let mut state = state_clone.lock().unwrap();
                                state.live_logs.push("✅ whisper.cpp downloaded and built successfully!".to_string());
                                // Try to auto-detect the new binary
                                if let Some(detected) = auto_detect_whisper() {
                                    state.settings.whisper_cpp_path = detected;
                                }
                            }
                        });
                    }
                    
                    ui.label(RichText::new("(This may take several minutes)").size(10.0).color(Color32::from_gray(150)));
                });
            });
            
            ui.add_space(20.0);
            
            // Save/Cancel buttons
            ui.horizontal(|ui| {
                if ui.button("💾 Save").clicked() {
                    if let Err(e) = state.settings.save() {
                        state.error = format!("Failed to save settings: {}", e);
                    } else {
                        state.live_logs.push("✅ Settings saved successfully".to_string());
                        state.show_settings = false;
                    }
                }
                
                if ui.button("❌ Cancel").clicked() {
                    // Reload settings to discard changes
                    state.settings = AppSettings::load();
                    state.show_settings = false;
                }
            });
        });
}

fn auto_detect_whisper() -> Option<String> {
    // Check environment variable
    if let Ok(path) = std::env::var("WHISPER_CPP_PATH") {
        if Path::new(&path).exists() {
            return Some(path);
        }
    }
    
    // Check common locations
    let candidates = vec![
        "./whisper.cpp",
        "./whisper-cpp",
        "/usr/local/bin/whisper.cpp",
        "/usr/local/bin/whisper-cpp",
        "/usr/bin/whisper.cpp",
        "/usr/bin/whisper-cpp",
    ];
    
    for candidate in candidates {
        if Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    
    // Try which command
    #[cfg(unix)]
    {
        for cmd in &["whisper.cpp", "whisper-cpp"] {
            if let Ok(output) = std::process::Command::new("which").arg(cmd).output() {
                if output.status.success() {
                    if let Ok(path_str) = String::from_utf8(output.stdout) {
                        let path = path_str.trim().to_string();
                        if !path.is_empty() && Path::new(&path).exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    
    None
}
