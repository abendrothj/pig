//! macOS Features Integration for LAO UI
//! 
//! Demonstrates macOS native features integration with the UI

use lao_orchestrator_core::macos_integrations::{
    MenuBarManager, SpotlightSearchManager, QuickLookPreview, NotificationManager,
    KeyboardShortcuts,
};
use std::path::Path;

/// Initialize macOS features for the application
pub fn init_macos_features(app_name: &str, version: &str, workflows_path: &Path) {
    // Initialize menu bar
    let menu_manager = MenuBarManager::new(app_name, version, workflows_path);
    let _menu_structure = menu_manager.get_menu_structure();
    let _shortcuts = menu_manager.get_shortcuts();
    println!("✓ Menu bar initialized");

    // Initialize Spotlight search indexing
    let mut spotlight = SpotlightSearchManager::new();
    index_workflows(&mut spotlight, workflows_path);
    index_plugins(&mut spotlight);
    println!("✓ Spotlight search indexed");

    // Initialize Quick Look
    println!("✓ Quick Look preview ready");

    // Initialize Notification Center
    let _notification_manager = NotificationManager::new();
    println!("✓ Notification Center integrated");
}

/// Index all workflows for Spotlight search
fn index_workflows(spotlight: &mut SpotlightSearchManager, workflows_path: &Path) {
    if let Ok(entries) = std::fs::read_dir(workflows_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "yaml") {
                let filename = path.file_stem().unwrap_or_default().to_string_lossy();
                let tags = vec![
                    "workflow".to_string(),
                    "dag".to_string(),
                    filename.to_string(),
                ];
                spotlight.index_workflow(&path, &filename, tags);
            }
        }
    }
}

/// Index all available plugins
fn index_plugins(spotlight: &mut SpotlightSearchManager) {
    let plugins = vec![
        ("EchoPlugin", "Simple text echoing and processing"),
        ("WhisperPlugin", "Speech-to-text transcription"),
        ("OllamaPlugin", "Local LLM integration"),
        ("GGUFPlugin", "GGUF model inference"),
        ("LMStudioPlugin", "LM Studio integration"),
        ("SummarizerPlugin", "Text summarization"),
        ("PromptDispatcherPlugin", "Workflow generation from prompts"),
        ("ANEInferencePlugin", "Apple Neural Engine inference"),
        ("CudaInferencePlugin", "CUDA GPU inference"),
        ("VLLMPlugin", "Vision-language models"),
    ];

    for (name, desc) in plugins {
        let tags = vec!["plugin".to_string(), name.to_lowercase()];
        spotlight.index_plugin(name, desc, tags);
    }
}

/// Get keyboard shortcuts for display
pub fn display_shortcuts() {
    let shortcuts = KeyboardShortcuts::default();
    println!("\n📋 LAO Keyboard Shortcuts:");
    println!("  New Workflow:        {}", shortcuts.new_workflow);
    println!("  Open Workflow:       {}", shortcuts.open_workflow);
    println!("  Save Workflow:       {}", shortcuts.save_workflow);
    println!("  Run Workflow:        {}", shortcuts.run_workflow);
    println!("  Stop Workflow:       {}", shortcuts.stop_workflow);
    println!("  Spotlight Search:    {}", shortcuts.search_spotlight);
    println!("  Quick Look:          {}", shortcuts.quick_look);
}

/// Demonstrate notification features
pub fn demo_notifications() {
    let mut notif_manager = NotificationManager::new();

    // Demo success notification
    notif_manager.notify_workflow_complete("Audio Transcription", 42);

    // Demo error notification
    notif_manager.notify_workflow_error(
        "Video Processing",
        "GPU memory exceeded",
    );

    // Demo background task notification
    notif_manager.notify_background_task("Model Download", "Downloading whisper-medium...");

    println!("✓ Notifications sent to Notification Center");
}

/// Demonstrate Quick Look preview
pub fn demo_quick_look(file_path: &Path) -> std::io::Result<()> {
    if file_path.extension().map_or(false, |ext| ext == "yaml") {
        let preview = QuickLookPreview::for_workflow(file_path)?;
        println!("\n📄 Quick Look Preview: {}", file_path.display());
        println!("{}", "-".repeat(60));
        println!("{}", preview.to_text());
    } else {
        let preview = QuickLookPreview::for_documentation(file_path)?;
        println!("\n📖 Documentation Preview: {}", file_path.display());
        println!("{}", "-".repeat(60));
        println!("{}", preview.to_text());
    }
    Ok(())
}

/// Demonstrate Spotlight search
pub fn demo_spotlight_search(query: &str) -> Vec<String> {
    let mut spotlight = SpotlightSearchManager::new();
    
    // Index sample workflows
    spotlight.index_workflow(
        Path::new("workflows/audio_transcription.yaml"),
        "Audio Transcription",
        vec!["audio".to_string(), "transcribe".to_string()],
    );
    spotlight.index_workflow(
        Path::new("workflows/text_summarization.yaml"),
        "Text Summarization",
        vec!["text".to_string(), "summarize".to_string()],
    );

    let results = spotlight.search(query);
    println!("\n🔍 Spotlight Results for '{}': {} items", query, results.len());
    results
        .iter()
        .map(|item| {
            println!("  • {} ({})", item.title, item.content_type);
            item.title.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_shortcuts() {
        // Just ensure this doesn't panic
        display_shortcuts();
    }

    #[test]
    fn test_demo_notifications() {
        demo_notifications();
    }

    #[test]
    fn test_demo_spotlight_search() {
        let results = demo_spotlight_search("audio");
        assert!(!results.is_empty());
    }

    #[test]
    fn test_init_macos_features() {
        init_macos_features("LAO", "1.0.0", Path::new("./workflows"));
    }
}
