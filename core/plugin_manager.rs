use crate::plugins::PluginRegistry;
use anyhow::{anyhow, Result};
use lao_plugin_api::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Plugin configuration and settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub enabled: bool,
    pub settings: HashMap<String, serde_json::Value>,
    pub permissions: Vec<String>,
    pub auto_update: bool,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            settings: HashMap::new(),
            permissions: vec!["read_files".to_string(), "write_files".to_string()],
            auto_update: false,
        }
    }
}

/// Plugin lifecycle events used by the workflow engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginEvent {
    WorkflowStarted {
        workflow_id: String,
        workflow_name: String,
    },
    WorkflowCompleted {
        workflow_id: String,
        success: bool,
    },
    StepStarted {
        workflow_id: String,
        step_id: String,
        plugin_name: String,
    },
    StepCompleted {
        workflow_id: String,
        step_id: String,
        plugin_name: String,
        output: String,
    },
    PluginLoaded {
        plugin_name: String,
    },
    PluginUnloaded {
        plugin_name: String,
    },
}

/// Plugin manager: handles loading, configuration, and lifecycle
#[derive(Debug)]
pub struct PluginManager {
    pub registry: PluginRegistry,
    pub configs: HashMap<String, PluginConfig>,
    pub plugin_directory: PathBuf,
    pub config_directory: PathBuf,
}

impl PluginManager {
    pub fn new<P: AsRef<Path>>(plugin_dir: P) -> Result<Self> {
        let plugin_directory = plugin_dir.as_ref().to_path_buf();
        let config_directory = plugin_directory.join("configs");

        std::fs::create_dir_all(&plugin_directory)?;
        std::fs::create_dir_all(&config_directory)?;

        let mut manager = Self {
            registry: PluginRegistry::new(),
            configs: HashMap::new(),
            plugin_directory,
            config_directory,
        };

        manager.load_plugins()?;
        manager.load_configs()?;

        Ok(manager)
    }

    /// Load all plugins from the plugin directory
    pub fn load_plugins(&mut self) -> Result<()> {
        self.registry = PluginRegistry::dynamic_registry(
            self.plugin_directory
                .to_str()
                .ok_or_else(|| anyhow!("Invalid plugin directory path"))?,
        );
        Ok(())
    }

    /// Load plugin configurations from disk
    pub fn load_configs(&mut self) -> Result<()> {
        for plugin_name in self.registry.plugins.keys() {
            let config_path = self.config_directory.join(format!("{}.json", plugin_name));
            if config_path.exists() {
                let config_data = std::fs::read_to_string(&config_path)?;
                let config: PluginConfig = serde_json::from_str(&config_data)?;
                self.configs.insert(plugin_name.clone(), config);
            } else {
                let default_config = PluginConfig::default();
                self.configs
                    .insert(plugin_name.clone(), default_config.clone());
                self.save_plugin_config(plugin_name, &default_config)?;
            }
        }
        Ok(())
    }

    /// Save plugin configuration to disk
    pub fn save_plugin_config(&self, plugin_name: &str, config: &PluginConfig) -> Result<()> {
        let config_path = self.config_directory.join(format!("{}.json", plugin_name));
        let config_data = serde_json::to_string_pretty(config)?;
        std::fs::write(config_path, config_data)?;
        Ok(())
    }

    /// Get plugin configuration
    pub fn get_plugin_config(&self, name: &str) -> Option<&PluginConfig> {
        self.configs.get(name)
    }

    /// Update plugin configuration
    pub fn update_plugin_config(&mut self, name: &str, config: PluginConfig) -> Result<()> {
        self.save_plugin_config(name, &config)?;
        self.configs.insert(name.to_string(), config);
        Ok(())
    }

    /// Enable/disable a plugin
    pub fn set_plugin_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        let config = self
            .configs
            .get(name)
            .ok_or_else(|| anyhow!("Plugin '{}' not found", name))?
            .clone();
        let mut updated = config;
        updated.enabled = enabled;
        self.update_plugin_config(name, updated)
    }

    /// Check if a plugin is enabled
    pub fn is_plugin_enabled(&self, name: &str) -> bool {
        self.configs.get(name).map(|c| c.enabled).unwrap_or(true)
    }

    /// Hot reload a plugin (unload and reload)
    pub fn hot_reload_plugin(&mut self, name: &str) -> Result<()> {
        if self.registry.plugins.contains_key(name) {
            self.registry.plugins.remove(name);
        }
        self.load_plugins()
    }

    /// List all plugins with their enabled status
    pub fn list_plugins_with_status(&self) -> Vec<(String, bool, &PluginInfo)> {
        self.registry
            .plugins
            .iter()
            .map(|(name, plugin)| {
                let enabled = self.is_plugin_enabled(name);
                (name.clone(), enabled, &plugin.info)
            })
            .collect()
    }

    /// Uninstall a plugin (remove from registry and delete config)
    pub fn uninstall_plugin(&mut self, name: &str) -> Result<()> {
        if let Err(e) = self.registry.remove_plugin(name) {
            return Err(anyhow!("Failed to remove plugin: {}", e));
        }

        self.configs.remove(name);
        let config_path = self.config_directory.join(format!("{}.json", name));
        if config_path.exists() {
            std::fs::remove_file(config_path)?;
        }

        Ok(())
    }

    /// Validate a plugin's permissions for a requested action
    pub fn validate_plugin_permissions(&self, name: &str, permission: &str) -> bool {
        self.configs
            .get(name)
            .map(|c| c.permissions.contains(&permission.to_string()))
            .unwrap_or(false)
    }

    /// Validate that all required dependencies are available
    pub fn validate_plugin_dependencies(&self, name: &str) -> Result<()> {
        let plugin = self
            .registry
            .plugins
            .get(name)
            .ok_or_else(|| anyhow!("Plugin '{}' not found", name))?;

        let missing: Vec<&str> = plugin
            .info
            .dependencies
            .iter()
            .filter(|dep| !dep.optional && !self.registry.plugins.contains_key(&dep.name))
            .map(|dep| dep.name.as_str())
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(
                "Missing required dependencies: {}",
                missing.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_plugin_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("plugins");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        (dir, plugin_dir)
    }

    #[test]
    fn test_plugin_config_default() {
        let config = PluginConfig::default();
        assert!(config.enabled);
        assert!(!config.auto_update);
        assert!(config.permissions.contains(&"read_files".to_string()));
        assert!(config.permissions.contains(&"write_files".to_string()));
    }

    #[test]
    fn test_plugin_config_serialization_roundtrip() {
        let mut config = PluginConfig::default();
        config.enabled = false;
        config.settings.insert(
            "model".to_string(),
            serde_json::Value::String("gpt-4".to_string()),
        );

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: PluginConfig = serde_json::from_str(&json).unwrap();

        assert!(!deserialized.enabled);
        assert_eq!(
            deserialized.settings.get("model").unwrap().as_str(),
            Some("gpt-4")
        );
    }

    #[test]
    fn test_plugin_manager_new_creates_dirs() {
        let (_dir, plugin_dir) = temp_plugin_dir();
        let manager = PluginManager::new(&plugin_dir).unwrap();

        assert!(plugin_dir.join("configs").exists());
        assert_eq!(manager.registry.plugin_count(), 0);
    }

    #[test]
    fn test_plugin_manager_save_and_load_config() {
        let (_dir, plugin_dir) = temp_plugin_dir();
        let manager = PluginManager::new(&plugin_dir).unwrap();

        let config = PluginConfig {
            enabled: false,
            settings: HashMap::new(),
            permissions: vec!["network".to_string()],
            auto_update: true,
        };

        manager.save_plugin_config("test_plugin", &config).unwrap();

        // Verify file exists
        let config_path = plugin_dir.join("configs").join("test_plugin.json");
        assert!(config_path.exists());

        // Read back
        let data = std::fs::read_to_string(&config_path).unwrap();
        let loaded: PluginConfig = serde_json::from_str(&data).unwrap();
        assert!(!loaded.enabled);
        assert!(loaded.auto_update);
        assert_eq!(loaded.permissions, vec!["network".to_string()]);
    }

    #[test]
    fn test_is_plugin_enabled_default_true() {
        let (_dir, plugin_dir) = temp_plugin_dir();
        let manager = PluginManager::new(&plugin_dir).unwrap();

        // Unknown plugin defaults to enabled
        assert!(manager.is_plugin_enabled("nonexistent"));
    }

    #[test]
    fn test_validate_permissions() {
        let (_dir, plugin_dir) = temp_plugin_dir();
        let mut manager = PluginManager::new(&plugin_dir).unwrap();

        let config = PluginConfig {
            enabled: true,
            settings: HashMap::new(),
            permissions: vec!["read_files".to_string()],
            auto_update: false,
        };
        manager.configs.insert("test".to_string(), config);

        assert!(manager.validate_plugin_permissions("test", "read_files"));
        assert!(!manager.validate_plugin_permissions("test", "write_files"));
        assert!(!manager.validate_plugin_permissions("unknown", "read_files"));
    }

    #[test]
    fn test_update_plugin_config() {
        let (_dir, plugin_dir) = temp_plugin_dir();
        let mut manager = PluginManager::new(&plugin_dir).unwrap();

        let config = PluginConfig::default();
        manager.configs.insert("test".to_string(), config);

        let mut updated = PluginConfig::default();
        updated.enabled = false;
        manager.update_plugin_config("test", updated).unwrap();

        assert!(!manager.is_plugin_enabled("test"));

        // Verify persisted to disk
        let config_path = plugin_dir.join("configs").join("test.json");
        let data = std::fs::read_to_string(&config_path).unwrap();
        let loaded: PluginConfig = serde_json::from_str(&data).unwrap();
        assert!(!loaded.enabled);
    }

    #[test]
    fn test_plugin_event_serialization() {
        let event = PluginEvent::StepCompleted {
            workflow_id: "wf-1".to_string(),
            step_id: "step-1".to_string(),
            plugin_name: "EchoPlugin".to_string(),
            output: "hello".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("StepCompleted"));
        assert!(json.contains("EchoPlugin"));

        let deserialized: PluginEvent = serde_json::from_str(&json).unwrap();
        if let PluginEvent::StepCompleted { plugin_name, .. } = deserialized {
            assert_eq!(plugin_name, "EchoPlugin");
        } else {
            panic!("wrong variant");
        }
    }
}
