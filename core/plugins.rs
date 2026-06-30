use crate::cross_platform::{PathUtils, Platform};
use crate::plugin_result::PluginRunResult;
use lao_plugin_api::*;
use libloading::{Library, Symbol};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr::addr_of;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PluginInstance {
    pub library: Arc<Library>,
    pub vtable: PluginVTablePtr,
    pub info: PluginInfo,
    /// Negotiated ABI version (1 = text channel, 2 = structured results).
    pub abi_version: u32,
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

            // Read `version` (ABI offset 0, present in every version) via a raw field
            // read so we never form a full-struct reference to a possibly-shorter v1
            // vtable before we know its size.
            let version = addr_of!((*vtable).version).read();
            tracing::debug!("VTable version: {}", version);
            if !(LAO_PLUGIN_ABI_MIN_SUPPORTED..=LAO_PLUGIN_ABI_VERSION).contains(&version) {
                return Err(format!(
                    "Unsupported plugin ABI version {} (supported: {}..={})",
                    version, LAO_PLUGIN_ABI_MIN_SUPPORTED, LAO_PLUGIN_ABI_VERSION
                ));
            }

            // `get_metadata` lives in the stable v1 prefix, so it is in-bounds for any
            // supported version.
            let get_metadata = addr_of!((*vtable).get_metadata).read();
            let metadata = get_metadata();
            tracing::debug!("Got metadata from plugin");

            let info = PluginInfo::from_metadata(&metadata);
            tracing::debug!("Created PluginInfo: {}", info.name);

            Ok(PluginInstance {
                library: Arc::new(library),
                vtable,
                info,
                abi_version: version,
            })
        }
    }

    pub fn validate_input(&self, input: &PluginInput) -> bool {
        unsafe {
            let validate = addr_of!((*self.vtable).validate_input).read();
            validate(input)
        }
    }

    /// Execute the plugin with a pre-built PluginInput.
    ///
    /// ABI v2 plugins return a structured status via `run_structured`; v1 plugins fall
    /// back to the legacy text channel and the `error:` convention.
    pub fn run_plugin(&self, input: &PluginInput) -> PluginRunResult {
        if self.vtable.is_null() {
            return PluginRunResult::runtime_error("Plugin vtable is null");
        }
        if !self.validate_input(input) {
            return PluginRunResult::validation_failed(format!(
                "Plugin '{}' rejected input",
                self.info.name
            ));
        }
        if self.abi_version >= 2 {
            self.run_structured(input)
        } else {
            self.run_legacy_text(input)
        }
    }

    fn run_structured(&self, input: &PluginInput) -> PluginRunResult {
        unsafe {
            let run_structured = addr_of!((*self.vtable).run_structured).read();
            let free_result = addr_of!((*self.vtable).free_result).read();
            let result = run_structured(input);
            let text = if result.text.is_null() {
                None
            } else {
                Some(CStr::from_ptr(result.text).to_string_lossy().to_string())
            };
            let status = result.status;
            free_result(result);
            PluginRunResult::from_status_code(status, text, &self.info.name)
        }
    }

    fn run_legacy_text(&self, input: &PluginInput) -> PluginRunResult {
        unsafe {
            let run = addr_of!((*self.vtable).run).read();
            let free_output = addr_of!((*self.vtable).free_output).read();
            let result = run(input);
            if result.text.is_null() {
                return PluginRunResult::runtime_error(format!(
                    "Plugin '{}' returned null output",
                    self.info.name
                ));
            }
            let output_str = CStr::from_ptr(result.text).to_string_lossy().to_string();
            free_output(result);
            PluginRunResult::from_plugin_text(output_str)
        }
    }

    /// Convenience: build a PluginInput from a string and run the plugin.
    pub fn run_with_text(&self, text: &str) -> PluginRunResult {
        let c_string = match CString::new(text) {
            Ok(s) => s,
            Err(e) => {
                return PluginRunResult::runtime_error(format!("Invalid input string: {}", e))
            }
        };
        let raw_text = c_string.into_raw();
        let input = PluginInput { text: raw_text };
        let result = self.run_plugin(&input);
        unsafe {
            let _ = CString::from_raw(raw_text);
        }
        result
    }

    pub fn get_capabilities(&self) -> Vec<PluginCapability> {
        unsafe {
            let get_caps = addr_of!((*self.vtable).get_capabilities).read();
            let caps_ptr = get_caps();
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
    loaded_paths: HashSet<std::path::PathBuf>,
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
            loaded_paths: HashSet::new(),
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
        tracing::debug!(
            "PluginRegistry::default_registry() using directory: {}",
            plugin_dir_str
        );
        let registry = Self::dynamic_registry(plugin_dir_str);
        tracing::debug!(
            "PluginRegistry loaded {} plugins: {:?}",
            registry.plugin_count(),
            registry.plugin_names()
        );
        registry
    }

    pub fn load_plugins_from_directory(&mut self, plugin_dir: &str) {
        tracing::debug!("Loading plugins from directory: {}", plugin_dir);

        let entries = match std::fs::read_dir(plugin_dir) {
            Ok(e) => e,
            Err(_) => {
                tracing::error!("Failed to read plugin directory: {}", plugin_dir);
                return;
            }
        };

        let ext = Platform::shared_lib_extension();
        let prefix = Platform::shared_lib_prefix();
        let mut found_files = 0;
        let mut loaded_count = 0;

        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();

            if path.is_dir() {
                // Strategy 1: Check target/release/ inside plugin subdirectory
                // Each plugin builds into its own target/release/ with cargo build --release
                let release_dir = path.join("target").join("release");
                if release_dir.is_dir() {
                    loaded_count +=
                        self.load_shared_libs_from(&release_dir, prefix, ext, &mut found_files);
                }

                // Strategy 2: Check for shared libs directly in the subdirectory
                // (for manually placed or pre-built plugins)
                loaded_count += self.load_shared_libs_from(&path, prefix, ext, &mut found_files);
            } else if self.is_shared_library_file(&path) {
                // Strategy 3: Shared libs directly in plugins/ root (legacy layout)
                found_files += 1;
                tracing::debug!("Found plugin file in root: {}", path.display());
                match self.load_plugin(&path) {
                    Ok(plugin) => {
                        if self.try_register_plugin(plugin, &path) {
                            loaded_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to load plugin {}: {}", path.display(), e);
                    }
                }
            }
        }

        tracing::debug!(
            "Plugin loading summary: {} files found, {} plugins loaded",
            found_files,
            loaded_count
        );
    }

    /// Load all shared libraries from a directory, returning count of successfully loaded plugins.
    fn load_shared_libs_from(
        &mut self,
        dir: &std::path::Path,
        prefix: &str,
        ext: &str,
        found_files: &mut usize,
    ) -> usize {
        let files = match std::fs::read_dir(dir) {
            Ok(f) => f,
            Err(_) => return 0,
        };

        let mut loaded = 0;
        for f in files.filter_map(|e| e.ok()) {
            let fpath = f.path();
            let fname = match fpath.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            // Match plugin shared libraries: lib*plugin*.{dylib,so,dll}
            let matches_ext = fpath.extension().and_then(|e| e.to_str()) == Some(ext);
            let matches_pattern =
                fname.starts_with(prefix) && fname.to_lowercase().contains("plugin");

            if matches_ext && matches_pattern {
                *found_files += 1;
                tracing::debug!("Found plugin: {}", fpath.display());
                match self.load_plugin(&fpath) {
                    Ok(plugin) => {
                        if self.try_register_plugin(plugin, &fpath) {
                            loaded += 1;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to load plugin {}: {}", fpath.display(), e);
                    }
                }
            }
        }
        loaded
    }

    fn is_shared_library_file(&self, path: &std::path::Path) -> bool {
        Platform::is_shared_lib_file(path)
    }

    pub fn load_plugin(&self, dll_path: &Path) -> Result<PluginInstance, String> {
        unsafe {
            tracing::debug!("Loading plugin from: {}", dll_path.display());

            if !dll_path.exists() {
                return Err(format!(
                    "Plugin file does not exist: {}",
                    dll_path.display()
                ));
            }

            let library = Library::new(dll_path).map_err(|e| {
                format!(
                    "Failed to load plugin library {}: {} (check if dependencies are available)",
                    dll_path.display(),
                    e
                )
            })?;

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
                return Err(format!(
                    "plugin_vtable returned null pointer for {}",
                    dll_path.display()
                ));
            }

            tracing::debug!("Called plugin_vtable function, got pointer: {:?}", vtable);

            PluginInstance::new(library, vtable)
        }
    }

    pub fn register_plugin(&mut self, plugin: PluginInstance) -> Result<(), String> {
        let name = plugin.info.name.clone();
        let version = plugin.info.version.clone();
        let dependencies = plugin.info.dependencies.clone();

        if self.plugins.contains_key(&name) {
            return Err(format!(
                "Duplicate plugin '{}'; keeping first loaded instance (version {})",
                name,
                self.plugins
                    .get(&name)
                    .map(|p| p.info.version.as_str())
                    .unwrap_or("?")
            ));
        }
        self.plugins.insert(name.clone(), plugin);

        let versions = self.plugin_versions.entry(name.clone()).or_default();
        if !versions.contains(&version) {
            versions.push(version.clone());
        }

        self.plugin_dependencies.insert(name.clone(), dependencies);

        tracing::info!("Loaded plugin: {} (version: {})", name, version);
        Ok(())
    }

    fn try_register_plugin(&mut self, plugin: PluginInstance, path: &Path) -> bool {
        if self.loaded_paths.contains(path) {
            tracing::debug!("Skipping already-loaded plugin path: {}", path.display());
            return false;
        }
        match self.register_plugin(plugin) {
            Ok(()) => {
                self.loaded_paths.insert(path.to_path_buf());
                true
            }
            Err(e) => {
                tracing::warn!("{}", e);
                false
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_is_empty() {
        let registry = PluginRegistry::new();
        assert_eq!(registry.plugin_count(), 0);
        assert!(registry.plugin_names().is_empty());
    }

    #[test]
    fn test_registry_default_is_empty() {
        let registry = PluginRegistry::default();
        assert_eq!(registry.plugin_count(), 0);
    }

    #[test]
    fn test_load_from_nonexistent_directory() {
        let mut registry = PluginRegistry::new();
        registry.load_plugins_from_directory("/nonexistent/path/to/plugins");
        assert_eq!(registry.plugin_count(), 0);
    }

    #[test]
    fn test_load_from_empty_directory() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let mut registry = PluginRegistry::new();
        registry.load_plugins_from_directory(dir.path().to_str().unwrap());
        assert_eq!(registry.plugin_count(), 0);
    }

    #[test]
    fn test_load_plugin_nonexistent_file() {
        let registry = PluginRegistry::new();
        let result = registry.load_plugin(Path::new("/nonexistent/plugin.dylib"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_load_plugin_invalid_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let fake_plugin = dir.path().join("libfake_plugin.dylib");
        std::fs::write(&fake_plugin, b"not a real shared library").unwrap();

        let registry = PluginRegistry::new();
        let result = registry.load_plugin(&fake_plugin);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_nonexistent_plugin() {
        let registry = PluginRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_get_with_version_nonexistent() {
        let registry = PluginRegistry::new();
        assert!(registry.get_with_version("anything", "1.0").is_none());
    }

    #[test]
    fn test_find_by_tag_empty() {
        let registry = PluginRegistry::new();
        assert!(registry.find_plugins_by_tag("inference").is_empty());
    }

    #[test]
    fn test_find_by_capability_empty() {
        let registry = PluginRegistry::new();
        assert!(registry
            .find_plugins_by_capability("text-generation")
            .is_empty());
    }

    #[test]
    fn test_remove_nonexistent_plugin() {
        let mut registry = PluginRegistry::new();
        let result = registry.remove_plugin("nope");
        // Should succeed (no dependencies block it, nothing to remove)
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_dependencies_unknown_plugin() {
        let registry = PluginRegistry::new();
        let result = registry.resolve_dependencies("unknown");
        // Should return just the plugin itself
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec!["unknown"]);
    }

    #[test]
    fn test_is_shared_library_file() {
        let registry = PluginRegistry::new();
        let ext = Platform::shared_lib_extension();
        let name = format!("libfoo.{}", ext);
        let path = Path::new(&name);
        assert!(registry.is_shared_library_file(path));
        assert!(!registry.is_shared_library_file(Path::new("foo.txt")));
    }

    fn built_echo_plugin_path() -> Option<std::path::PathBuf> {
        let ext = Platform::shared_lib_extension();
        let prefix = Platform::shared_lib_prefix();
        let candidate = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../plugins/EchoPlugin/target/release")
            .join(format!("{}echo_plugin.{}", prefix, ext));
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    }

    #[test]
    fn test_duplicate_registration_rejected_keeps_first() {
        let Some(path) = built_echo_plugin_path() else {
            eprintln!("EchoPlugin not built; skipping duplicate-registration test");
            return;
        };
        let mut registry = PluginRegistry::new();

        let first = registry.load_plugin(&path).expect("load echo plugin");
        let first_version = first.info.version.clone();
        assert!(registry.register_plugin(first).is_ok());

        let second = registry.load_plugin(&path).expect("load echo plugin again");
        let result = registry.register_plugin(second);
        assert!(result.is_err(), "duplicate registration must be rejected");
        assert_eq!(registry.plugin_count(), 1);
        assert_eq!(
            registry.get("EchoPlugin").unwrap().info.version,
            first_version,
            "first instance must be kept"
        );
    }

    #[test]
    fn test_directory_scan_dedups_plugins() {
        let Some(_) = built_echo_plugin_path() else {
            eprintln!("EchoPlugin not built; skipping directory dedup test");
            return;
        };
        let plugin_dir = PathUtils::plugin_dir();
        let registry = PluginRegistry::dynamic_registry(plugin_dir.to_str().unwrap_or("plugins"));
        let mut names = registry.plugin_names();
        names.sort();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(
            names, deduped,
            "registry must not contain duplicate plugins"
        );
    }
}
