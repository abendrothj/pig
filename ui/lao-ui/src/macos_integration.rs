//! macOS Native Features Integration for egui UI
//! 
//! Provides macOS-specific UI enhancements for LAO's egui application

use lao_orchestrator_core::macos_integrations::{
    MenuBarManager, NotificationManager, SpotlightSearchManager, KeyboardShortcuts,
};
use std::path::PathBuf;

/// macOS UI state for egui integration
pub struct MacOSUIState {
    pub menu_manager: MenuBarManager,
    pub notification_manager: NotificationManager,
    pub spotlight_manager: SpotlightSearchManager,
    pub shortcuts: KeyboardShortcuts,
}

impl MacOSUIState {
    /// Initialize macOS features for the egui application
    pub fn new(workflows_path: PathBuf) -> Self {
        let menu_manager = MenuBarManager::new("LAO", "0.1.20", &workflows_path);
        let mut spotlight_manager = SpotlightSearchManager::new();
        
        // Index default workflows
        if let Ok(entries) = std::fs::read_dir(&workflows_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "yaml") {
                    let filename = path.file_stem().unwrap_or_default().to_string_lossy();
                    spotlight_manager.index_workflow(
                        &path,
                        &filename,
                        vec!["workflow".to_string()],
                    );
                }
            }
        }
        
        Self {
            menu_manager,
            notification_manager: NotificationManager::new(),
            spotlight_manager,
            shortcuts: KeyboardShortcuts::default(),
        }
    }

    /// Notify workflow completion
    pub fn notify_workflow_complete(&mut self, workflow_name: &str, duration_secs: u64) {
        self.notification_manager.notify_workflow_complete(workflow_name, duration_secs);
    }

    /// Notify workflow error
    pub fn notify_workflow_error(&mut self, workflow_name: &str, error: &str) {
        self.notification_manager.notify_workflow_error(workflow_name, error);
    }

    /// Get keyboard shortcuts for display in UI
    pub fn get_shortcuts_text(&self) -> String {
        format!(
            "New: {} | Open: {} | Save: {} | Run: {}",
            self.shortcuts.new_workflow,
            self.shortcuts.open_workflow,
            self.shortcuts.save_workflow,
            self.shortcuts.run_workflow
        )
    }
}

/// Add macOS menu integration to egui (call in app initialization)
#[cfg(target_os = "macos")]
pub fn setup_macos_menu() {
    // Note: egui doesn't expose native menu bar API directly
    // This is a placeholder for future native menu integration
    // Currently, keyboard shortcuts work through egui's input handling
    println!("macOS menu integration ready (keyboard shortcuts enabled)");
}

#[cfg(not(target_os = "macos"))]
pub fn setup_macos_menu() {
    // No-op on non-macOS
}

/// Helper to add keyboard shortcuts section to egui UI
pub fn show_keyboard_shortcuts_ui(ui: &mut egui::Ui, shortcuts: &KeyboardShortcuts) {
    ui.heading("⌨️ Keyboard Shortcuts");
    ui.separator();
    
    ui.horizontal(|ui| {
        ui.label("New Workflow:");
        ui.monospace(&shortcuts.new_workflow);
    });
    ui.horizontal(|ui| {
        ui.label("Open Workflow:");
        ui.monospace(&shortcuts.open_workflow);
    });
    ui.horizontal(|ui| {
        ui.label("Save Workflow:");
        ui.monospace(&shortcuts.save_workflow);
    });
    ui.horizontal(|ui| {
        ui.label("Run Workflow:");
        ui.monospace(&shortcuts.run_workflow);
    });
    ui.horizontal(|ui| {
        ui.label("Stop Workflow:");
        ui.monospace(&shortcuts.stop_workflow);
    });
}
