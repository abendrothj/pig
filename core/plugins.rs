use crate::cross_platform::{PathUtils, Platform};
use lao_plugin_api::*;
use libloading::{Library, Symbol};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PluginInstance {
    pub library: Arc<Library>,
    pub vtable: PluginVTablePtr,
    pub info: PluginInfo,
    pub metadata: PluginInfo, // Use PluginInfo instead of PluginMetadata for Debug/Clone
}

impl PluginInstance {
    /// # Safety
    ///
    /// This function is unsafe because it dereferences the raw `vtable` pointer to check metadata.
    /// The caller must ensure that `vtable` is a valid pointer to a `PluginVTable`.
    pub unsafe fn new(library: Library, vtable: PluginVTablePtr) -> Result<Self, String> {
        unsafe {
            tracing::debug!("Creating PluginInstance with vtable: {:?}", vtable);

            if vtable.is_null() {
                return Err("VTable pointer is null".to_string());
            }

            let vtable_ref = &*vtable;
            tracing::debug!("VTable version: {}", vtable_ref.version);

            let metadata = (vtable_ref.get_metadata)();
            tracing::debug!("Got metadata from plugin");

            let info = PluginInfo::from_metadata(&metadata);
            tracing::debug!("Created PluginInfo: {}", info.name);

            Ok(PluginInstance {
                library: Arc::new(library),
                vtable,
                info: info.clone(),
                metadata: info,
            })
        }
    }

    pub fn validate_input(&self, input: &PluginInput) -> bool {
        unsafe { ((*self.vtable).validate_input)(input) }
    }

    /// Execute the plugin with a pre-built PluginInput, returning the output string.
    /// Encapsulates the unsafe vtable call, null checks, and memory cleanup.
    pub fn run_plugin(&self, input: &PluginInput) -> Result<String, String> {
        if self.vtable.is_null() {
            return Err("Plugin vtable is null".to_string());
        }
        unsafe {
            let result = ((*self.vtable).run)(input);
            if result.text.is_null() {
                return Err(format!("Plugin '{}' returned null output", self.info.name));
            }
            let output_str = CStr::from_ptr(result.text)
                .to_string_lossy()
                .to_string();
            ((*self.vtable).free_output)(result);
            Ok(output_str)
        }
    }

    /// Convenience: build a PluginInput from a string and run the plugin.
    pub fn run_with_text(&self, text: &str) -> Result<String, String> {
        let c_string = CString::new(text)
            .map_err(|e| format!("Invalid input string: {}", e))?;
        let input = PluginInput {
            text: c_string.into_raw(),
        };
        self.run_plugin(&input)
    }

    pub fn get_capabilities(&self) -> Vec<PluginCapability> {
        unsafe {
            let caps_ptr = ((*self.vtable).get_capabilities)();
            if caps_ptr.is_null() {
                return Vec::new();
            }

            let caps_str = CStr::from_ptr(caps_ptr).to_string_lossy();
            serde_json::from_str(&caps_str).unwrap_or_default()
        }
    }
}

#[derive(Debug)]
pub struct PluginRegistry {
    pub plugins: HashMap<String, PluginInstance>,
    pub plugin_versions: HashMap<String, Vec<String>>, // name -> versions
    pub plugin_dependencies: HashMap<String, Vec<PluginDependency>>,
}

// Safety: PluginRegistry is only accessed through Mutex in parallel execution,
// and PluginInstance's raw pointers are only accessed while holding the mutex lock.
unsafe impl Send for PluginRegistry {}
unsafe impl Sync for PluginRegistry {}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        PluginRegistry {
            plugins: HashMap::new(),
            plugin_versions: HashMap::new(),
            plugin_dependencies: HashMap::new(),
        }
    }

    pub fn dynamic_registry(plugin_dir: &str) -> Self {
        let mut registry = PluginRegistry::new();
        registry.load_plugins_from_directory(plugin_dir);
        registry
    }

    /// Create a dynamic registry using the default plugin directory
    pub fn default_registry() -> Self {
        let plugin_dir = PathUtils::plugin_dir();
        let plugin_dir_str = plugin_dir.to_str().unwrap_or("plugins");
        tracing::debug!("PluginRegistry::default_registry() using directory: {}", plugin_dir_str);
        let registry = Self::dynamic_registry(plugin_dir_str);
        tracing::debug!("PluginRegistry loaded {} plugins: {:?}", registry.plugin_count(), registry.plugin_names());
        registry
    }

    pub fn load_plugins_from_directory(&mut self, plugin_dir: &str) {
        tracing::debug!("Loading plugins from directory: {}", plugin_dir);

        if let Ok(entries) = std::fs::read_dir(plugin_dir) {
            let mut found_files = 0;
            let mut loaded_count = 0;

            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(files) = std::fs::read_dir(&path) {
                        for f in files.filter_map(|e| e.ok()) {
                            let fpath = f.path();
                            if let Some(ext) = fpath.extension().and_then(|s| s.to_str()) {
                                if self.is_shared_library_extension(ext) {
                                    found_files += 1;
                                    tracing::debug!("Found plugin file in subdirectory: {}", fpath.display());
                                    match self.load_plugin(&fpath) {
                                        Ok(plugin) => {
                                            self.register_plugin(plugin);
                                            loaded_count += 1;
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to load plugin {}: {}", fpath.display(), e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if self.is_shared_library_file(&path) {
                    found_files += 1;
                    tracing::debug!("Found plugin file in root: {}", path.display());
                    match self.load_plugin(&path) {
                        Ok(plugin) => {
                            self.register_plugin(plugin);
                            loaded_count += 1;
                        }
                        Err(e) => {
                            tracing::error!("Failed to load plugin {}: {}", path.display(), e);
                        }
                    }
                }
            }

            tracing::debug!("Plugin loading summary: {} files found, {} plugins loaded", found_files, loaded_count);
        } else {
            tracing::error!("Failed to read plugin directory: {}", plugin_dir);
        }
    }

    fn is_shared_library_extension(&self, ext: &str) -> bool {
        Platform::is_shared_lib_extension(ext)
    }

    fn is_shared_library_file(&self, path: &std::path::Path) -> bool {
        Platform::is_shared_lib_file(path)
    }

    pub fn load_plugin(&self, dll_path: &Path) -> Result<PluginInstance, String> {
        unsafe {
            tracing::debug!("Loading plugin from: {}", dll_path.display());

            if !dll_path.exists() {
                return Err(format!("Plugin file does not exist: {}", dll_path.display()));
            }

            let library = Library::new(dll_path)
                .map_err(|e| format!("Failed to load plugin library {}: {} (check if dependencies are available)", dll_path.display(), e))?;

            tracing::debug!("Library loaded successfully");

            let plugin_vtable_fn: Symbol<unsafe extern "C" fn() -> PluginVTablePtr> =
                library.get(b"plugin_vtable").map_err(|e| {
                    format!(
                        "Failed to get plugin_vtable symbol from {}: {} (plugin may not be built correctly)",
                        dll_path.display(),
                        e
                    )
                })?;

            tracing::debug!("Got plugin_vtable function");

            let vtable = plugin_vtable_fn();
            if vtable.is_null() {
                return Err(format!("plugin_vtable returned null pointer for {}", dll_path.display()));
            }

            tracing::debug!("Called plugin_vtable function, got pointer: {:?}", vtable);

            PluginInstance::new(library, vtable)
        }
    }

    pub fn register_plugin(&mut self, plugin: PluginInstance) {
        let name = plugin.info.name.clone();
        let version = plugin.info.version.clone();
        let dependencies = plugin.info.dependencies.clone();

        self.plugins.insert(name.clone(), plugin);

        self.plugin_versions
            .entry(name.clone())
            .or_default()
            .push(version.clone());

        self.plugin_dependencies.insert(name.clone(), dependencies);

        tracing::info!("Loaded plugin: {} (version: {})", name, version);
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn plugin_names(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<&PluginInstance> {
        self.plugins.get(name)
    }

    pub fn get_with_version(&self, name: &str, version: &str) -> Option<&PluginInstance> {
        self.plugins.get(name).filter(|p| p.info.version == version)
    }

    pub fn list_plugins(&self) -> Vec<&PluginInfo> {
        self.plugins.values().map(|p| &p.info).collect()
    }

    pub fn find_plugins_by_tag(&self, tag: &str) -> Vec<&PluginInfo> {
        self.plugins
            .values()
            .filter(|p| p.info.tags.iter().any(|t| t == tag))
            .map(|p| &p.info)
            .collect()
    }

    pub fn find_plugins_by_capability(&self, capability: &str) -> Vec<&PluginInfo> {
        self.plugins
            .values()
            .filter(|p| p.info.capabilities.iter().any(|c| c.name == capability))
            .map(|p| &p.info)
            .collect()
    }

    pub fn resolve_dependencies(&self, plugin_name: &str) -> Result<Vec<String>, String> {
        let mut resolved = Vec::new();
        let mut visited = std::collections::HashSet::new();

        self.resolve_dependencies_recursive(plugin_name, &mut resolved, &mut visited)?;

        Ok(resolved)
    }

    fn resolve_dependencies_recursive(
        &self,
        plugin_name: &str,
        resolved: &mut Vec<String>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        if visited.contains(plugin_name) {
            return Ok(());
        }

        visited.insert(plugin_name.to_string());

        if let Some(dependencies) = self.plugin_dependencies.get(plugin_name) {
            for dep in dependencies {
                if !dep.optional || self.plugins.contains_key(&dep.name) {
                    self.resolve_dependencies_recursive(&dep.name, resolved, visited)?;
                }
            }
        }

        resolved.push(plugin_name.to_string());
        Ok(())
    }

    pub fn validate_plugin_compatibility(&self, plugin_name: &str) -> Result<(), String> {
        if let Some(plugin) = self.plugins.get(plugin_name) {
            for dep in &plugin.info.dependencies {
                if !self.plugins.contains_key(&dep.name) && !dep.optional {
                    return Err(format!("Missing required dependency: {}", dep.name));
                }
            }
        }
        Ok(())
    }

    pub fn update_plugin(
        &mut self,
        plugin_name: &str,
        new_plugin: PluginInstance,
    ) -> Result<(), String> {
        if self.plugins.contains_key(plugin_name) {
            self.validate_plugin_compatibility(plugin_name)?;
            self.plugins.insert(plugin_name.to_string(), new_plugin);
            Ok(())
        } else {
            Err(format!("Plugin {} not found", plugin_name))
        }
    }

    pub fn remove_plugin(&mut self, plugin_name: &str) -> Result<(), String> {
        for (name, deps) in &self.plugin_dependencies {
            if name != plugin_name {
                for dep in deps {
                    if dep.name == plugin_name && !dep.optional {
                        return Err(format!(
                            "Cannot remove {}: required by {}",
                            plugin_name, name
                        ));
                    }
                }
            }
        }

        self.plugins.remove(plugin_name);
        self.plugin_versions.remove(plugin_name);
        self.plugin_dependencies.remove(plugin_name);

        Ok(())
    }
}
