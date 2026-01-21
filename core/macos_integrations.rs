//! macOS Native Integrations for LAO
//! 
//! Provides native macOS UI/UX features:
//! - Menu bar with workflow management
//! - Spotlight search indexing
//! - Quick Look preview support
//! - Notification Center integration

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// macOS Menu Bar Manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuBarManager {
    pub app_name: String,
    pub version: String,
    pub workflows_path: PathBuf,
}

impl MenuBarManager {
    /// Create a new menu bar manager
    pub fn new(app_name: &str, version: &str, workflows_path: impl AsRef<Path>) -> Self {
        Self {
            app_name: app_name.to_string(),
            version: version.to_string(),
            workflows_path: workflows_path.as_ref().to_path_buf(),
        }
    }

    /// Get menu bar structure for integration
    pub fn get_menu_structure(&self) -> MenuStructure {        MenuStructure {
            app_menu: AppMenu::default(),
            file_menu: FileMenu::default(),
            edit_menu: EditMenu::default(),
            view_menu: ViewMenu::default(),
            help_menu: HelpMenu::default(),
        }
    }

    /// Get keyboard shortcuts configuration
    pub fn get_shortcuts(&self) -> KeyboardShortcuts {
        KeyboardShortcuts::default()
    }
}

/// Menu bar structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuStructure {
    pub app_menu: AppMenu,
    pub file_menu: FileMenu,
    pub edit_menu: EditMenu,
    pub view_menu: ViewMenu,
    pub help_menu: HelpMenu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppMenu {
    pub about: MenuItem,
    pub preferences: MenuItem,
    pub quit: MenuItem,
}

impl Default for AppMenu {
    fn default() -> Self {
        Self {
            about: MenuItem::new("About LAO", ""),
            preferences: MenuItem::new("Preferences", "Cmd+,"),
            quit: MenuItem::new("Quit LAO", "Cmd+Q"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMenu {
    pub new_workflow: MenuItem,
    pub open_workflow: MenuItem,
    pub recent_workflows: MenuItem,
    pub save_workflow: MenuItem,
}

impl Default for FileMenu {
    fn default() -> Self {
        Self {
            new_workflow: MenuItem::new("New Workflow", "Cmd+N"),
            open_workflow: MenuItem::new("Open Workflow", "Cmd+O"),
            recent_workflows: MenuItem::new("Open Recent", ""),
            save_workflow: MenuItem::new("Save Workflow", "Cmd+S"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditMenu {
    pub undo: MenuItem,
    pub redo: MenuItem,
    pub cut: MenuItem,
    pub copy: MenuItem,
    pub paste: MenuItem,
}

impl Default for EditMenu {
    fn default() -> Self {
        Self {
            undo: MenuItem::new("Undo", "Cmd+Z"),
            redo: MenuItem::new("Redo", "Cmd+Shift+Z"),
            cut: MenuItem::new("Cut", "Cmd+X"),
            copy: MenuItem::new("Copy", "Cmd+C"),
            paste: MenuItem::new("Paste", "Cmd+V"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMenu {
    pub toggle_sidebar: MenuItem,
    pub toggle_console: MenuItem,
    pub fullscreen: MenuItem,
    pub zoom_in: MenuItem,
    pub zoom_out: MenuItem,
}

impl Default for ViewMenu {
    fn default() -> Self {
        Self {
            toggle_sidebar: MenuItem::new("Toggle Sidebar", "Cmd+B"),
            toggle_console: MenuItem::new("Toggle Console", "Cmd+J"),
            fullscreen: MenuItem::new("Enter Full Screen", "Cmd+Ctrl+F"),
            zoom_in: MenuItem::new("Zoom In", "Cmd++"),
            zoom_out: MenuItem::new("Zoom Out", "Cmd+-"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpMenu {
    pub documentation: MenuItem,
    pub keyboard_shortcuts: MenuItem,
    pub report_issue: MenuItem,
    pub check_updates: MenuItem,
}

impl Default for HelpMenu {
    fn default() -> Self {
        Self {
            documentation: MenuItem::new("LAO Documentation", ""),
            keyboard_shortcuts: MenuItem::new("Keyboard Shortcuts", "Cmd+?"),
            report_issue: MenuItem::new("Report Issue", ""),
            check_updates: MenuItem::new("Check for Updates", ""),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: String,
}

impl MenuItem {
    pub fn new(label: &str, shortcut: &str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardShortcuts {
    pub new_workflow: String,
    pub open_workflow: String,
    pub save_workflow: String,
    pub run_workflow: String,
    pub stop_workflow: String,
    pub search_spotlight: String,
    pub quick_look: String,
}

impl Default for KeyboardShortcuts {
    fn default() -> Self {
        Self {
            new_workflow: "Cmd+N".to_string(),
            open_workflow: "Cmd+O".to_string(),
            save_workflow: "Cmd+S".to_string(),
            run_workflow: "Cmd+R".to_string(),
            stop_workflow: "Cmd+.".to_string(),
            search_spotlight: "Cmd+Space".to_string(),
            quick_look: "Space".to_string(),
        }
    }
}

/// Spotlight Search Integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotlightSearchManager {
    pub indexed_items: Vec<SpotlightItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotlightItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub content_type: String, // UTType
    pub path: PathBuf,
    pub keywords: Vec<String>,
    pub thumbnail_path: Option<PathBuf>,
}

impl SpotlightSearchManager {
    /// Create a new spotlight search manager
    pub fn new() -> Self {
        Self {
            indexed_items: Vec::new(),
        }
    }

    /// Index a workflow for Spotlight
    pub fn index_workflow(&mut self, workflow_path: &Path, title: &str, tags: Vec<String>) {
        let item = SpotlightItem {
            id: format!("workflow-{}", workflow_path.file_stem().unwrap_or_default().to_string_lossy()),
            title: title.to_string(),
            description: format!("Workflow: {}", title),
            content_type: "com.lao.workflow".to_string(),
            path: workflow_path.to_path_buf(),
            keywords: tags,
            thumbnail_path: None,
        };
        self.indexed_items.push(item);
    }

    /// Index a plugin for Spotlight
    pub fn index_plugin(&mut self, plugin_name: &str, description: &str, tags: Vec<String>) {
        let item = SpotlightItem {
            id: format!("plugin-{}", plugin_name.to_lowercase()),
            title: plugin_name.to_string(),
            description: description.to_string(),
            content_type: "com.lao.plugin".to_string(),
            path: PathBuf::new(),
            keywords: tags,
            thumbnail_path: None,
        };
        self.indexed_items.push(item);
    }

    /// Search indexed items
    pub fn search(&self, query: &str) -> Vec<SpotlightItem> {
        let query_lower = query.to_lowercase();
        self.indexed_items
            .iter()
            .filter(|item| {
                item.title.to_lowercase().contains(&query_lower)
                    || item.description.to_lowercase().contains(&query_lower)
                    || item.keywords.iter().any(|k| k.to_lowercase().contains(&query_lower))
            })
            .cloned()
            .collect()
    }

    /// Get recent workflows
    pub fn get_recent_items(&self, limit: usize) -> Vec<SpotlightItem> {
        self.indexed_items.iter().take(limit).cloned().collect()
    }

    /// Clear index (for refresh)
    pub fn clear(&mut self) {
        self.indexed_items.clear();
    }
}

impl Default for SpotlightSearchManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Quick Look Preview Support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickLookPreview {
    pub file_path: PathBuf,
    pub preview_type: PreviewType,
    pub preview_text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PreviewType {
    YAML,
    JSON,
    Markdown,
    Text,
    Unknown,
}

impl QuickLookPreview {
    /// Create preview for a workflow file
    pub fn for_workflow(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self {
            file_path: path.to_path_buf(),
            preview_type: PreviewType::YAML,
            preview_text: content,
        })
    }

    /// Create preview for documentation
    pub fn for_documentation(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self {
            file_path: path.to_path_buf(),
            preview_type: PreviewType::Markdown,
            preview_text: content,
        })
    }

    /// Get preview as HTML (for display)
    pub fn to_html(&self) -> String {
        let escaped_text = self.preview_text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        format!(
            "<html><head><style>body {{ font-family: monospace; padding: 10px; }} pre {{ white-space: pre-wrap; }}</style></head><body><pre>{}</pre></body></html>",
            escaped_text
        )
    }

    /// Get preview as plain text
    pub fn to_text(&self) -> &str {
        &self.preview_text
    }

    /// Truncate preview to reasonable size
    pub fn truncate(&mut self, max_lines: usize) {
        let lines: Vec<&str> = self.preview_text.lines().collect();
        if lines.len() > max_lines {
            self.preview_text = lines[..max_lines].join("\n");
            self.preview_text.push_str("\n\n[Preview truncated...]");
        }
    }
}

/// Notification Center Integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationManager {
    pub notifications: Vec<Notification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub notification_type: NotificationType,
    pub timestamp: String,
    pub action_buttons: Vec<ActionButton>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotificationType {
    Success,
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionButton {
    pub label: String,
    pub action_id: String,
}

impl NotificationManager {
    /// Create a new notification manager
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
        }
    }

    /// Send workflow completion notification
    pub fn notify_workflow_complete(&mut self, workflow_name: &str, duration_secs: u64) {
        let notification = Notification {
            id: format!("workflow-complete-{}", chrono::Local::now().timestamp()),
            title: "Workflow Completed".to_string(),
            body: format!("{} completed in {} seconds", workflow_name, duration_secs),
            notification_type: NotificationType::Success,
            timestamp: chrono::Local::now().to_rfc3339(),
            action_buttons: vec![
                ActionButton {
                    label: "View Results".to_string(),
                    action_id: "view_results".to_string(),
                },
            ],
        };
        #[cfg(target_os = "macos")]
        send_notification_impl(&notification);
        self.notifications.push(notification);
    }

    /// Send workflow error notification
    pub fn notify_workflow_error(&mut self, workflow_name: &str, error: &str) {
        let notification = Notification {
            id: format!("workflow-error-{}", chrono::Local::now().timestamp()),
            title: "Workflow Error".to_string(),
            body: format!("{}: {}", workflow_name, error),
            notification_type: NotificationType::Error,
            timestamp: chrono::Local::now().to_rfc3339(),
            action_buttons: vec![
                ActionButton {
                    label: "View Details".to_string(),
                    action_id: "view_details".to_string(),
                },
            ],
        };
        #[cfg(target_os = "macos")]
        send_notification_impl(&notification);
        self.notifications.push(notification);
    }

    /// Send background task notification
    pub fn notify_background_task(&mut self, task_name: &str, status: &str) {
        let notification = Notification {
            id: format!("task-update-{}", chrono::Local::now().timestamp()),
            title: "Background Task".to_string(),
            body: format!("{}: {}", task_name, status),
            notification_type: NotificationType::Info,
            timestamp: chrono::Local::now().to_rfc3339(),
            action_buttons: vec![],
        };
        #[cfg(target_os = "macos")]
        send_notification_impl(&notification);
        self.notifications.push(notification);
    }

    /// Get all notifications
    pub fn get_notifications(&self) -> &[Notification] {
        &self.notifications
    }

    /// Clear notifications
    pub fn clear(&mut self) {
        self.notifications.clear();
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Platform-specific notification implementation
#[cfg(target_os = "macos")]
fn send_notification_impl(notification: &Notification) {
    // macOS notification center integration
    // This would use NSUserNotification or UserNotifications framework
    // For now, a placeholder that demonstrates the pattern
    use std::process::Command;
    
    let _ = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "display notification \"{}\" with title \"{}\"",
            notification.body, notification.title
        ))
        .output();
}

#[cfg(not(target_os = "macos"))]
fn send_notification_impl(_notification: &Notification) {
    // Fallback for non-macOS systems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_bar_manager() {
        let manager = MenuBarManager::new("LAO", "1.0.0", "./workflows");
        let menu = manager.get_menu_structure();
        assert_eq!(menu.app_menu.quit.label, "Quit LAO");
        assert_eq!(menu.file_menu.new_workflow.shortcut, "Cmd+N");
    }

    #[test]
    fn test_spotlight_indexing() {
        let mut spotlight = SpotlightSearchManager::new();
        spotlight.index_workflow(
            Path::new("./workflows/test.yaml"),
            "Test Workflow",
            vec!["test".to_string(), "demo".to_string()],
        );
        assert_eq!(spotlight.indexed_items.len(), 1);
    }

    #[test]
    fn test_spotlight_search() {
        let mut spotlight = SpotlightSearchManager::new();
        spotlight.index_workflow(
            Path::new("./workflows/transcribe.yaml"),
            "Audio Transcription",
            vec!["audio".to_string(), "transcribe".to_string()],
        );
        spotlight.index_plugin("WhisperPlugin", "Speech-to-text", vec!["audio".to_string()]);

        let results = spotlight.search("audio");
        assert!(results.len() >= 2);
    }

    #[test]
    fn test_quick_look_preview() {
        let preview = QuickLookPreview::for_documentation(Path::new("./README.md"));
        if let Ok(preview) = preview {
            assert_eq!(preview.preview_type, PreviewType::Markdown);
        }
    }

    #[test]
    fn test_notification_manager() {
        let mut manager = NotificationManager::new();
        manager.notify_workflow_complete("Test Workflow", 42);
        assert_eq!(manager.notifications.len(), 1);
        assert_eq!(manager.notifications[0].notification_type, NotificationType::Success);

        manager.notify_workflow_error("Failed Workflow", "Connection timeout");
        assert_eq!(manager.notifications.len(), 2);
        assert_eq!(manager.notifications[1].notification_type, NotificationType::Error);
    }

    #[test]
    fn test_keyboard_shortcuts() {
        let shortcuts = KeyboardShortcuts::default();
        assert_eq!(shortcuts.new_workflow, "Cmd+N");
        assert_eq!(shortcuts.run_workflow, "Cmd+R");
    }
}
