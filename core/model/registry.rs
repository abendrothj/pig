//! Model registry: resolves logical roles to physical model files, backends, and
//! metadata. Pure data plus validation — no network I/O, no download logic, only a
//! filesystem existence check for availability.

use crate::model::types::{ModelId, ModelRole};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelEntry {
    pub id: ModelId,
    pub format: String,
    pub path: PathBuf,
    pub backend: String,
    pub context_tokens: Option<u32>,
    pub estimated_memory_bytes: Option<u64>,
    pub roles: Vec<ModelRole>,
    /// Backend-specific execution parameters (e.g. gpu_layers, flash_attention,
    /// parallel, cache_type_k). Merged into the load request when no explicit
    /// execution_config is provided by the caller.
    #[serde(default)]
    pub execution_config: serde_json::Value,
    /// Override the backend's default tool-calling capability for this specific model.
    /// `None` means "defer to the backend" (llama_cpp defaults true, mlx defaults false).
    /// Set `true` to assert a model supports tool calls regardless of backend default,
    /// or `false` to disable tool routing even on a capable backend.
    #[serde(default)]
    pub tool_calling: Option<bool>,
    /// Whether this model supports extended reasoning / chain-of-thought (e.g. Qwen3
    /// `/think` mode, DeepSeek R1). `None` means not declared. Used as a hard routing
    /// constraint: requests with `requirements.reasoning = true` are only sent to models
    /// where this is `Some(true)`.
    #[serde(default)]
    pub reasoning: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedModelEntry {
    pub entry: ModelEntry,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub file_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegistryError {
    pub message: String,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model registry: {}", self.message)
    }
}

impl std::error::Error for RegistryError {}

impl RegistryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    entries: BTreeMap<ModelId, ModelEntry>,
    roles: BTreeMap<ModelRole, Vec<ModelId>>,
}

impl ModelRegistry {
    pub fn new(
        entries: Vec<ModelEntry>,
        roles: BTreeMap<ModelRole, Vec<ModelId>>,
    ) -> Result<Self, RegistryError> {
        let mut map = BTreeMap::new();
        for entry in entries {
            if map.insert(entry.id.clone(), entry.clone()).is_some() {
                return Err(RegistryError::new(format!(
                    "duplicate model id '{}'",
                    entry.id
                )));
            }
        }
        for (role, ids) in &roles {
            for id in ids {
                if !map.contains_key(id) {
                    return Err(RegistryError::new(format!(
                        "role '{}' references unknown model '{}'",
                        role, id
                    )));
                }
            }
        }
        Ok(Self {
            entries: map,
            roles,
        })
    }

    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        #[derive(Deserialize)]
        struct RoleToml {
            candidates: Vec<String>,
        }
        #[derive(Deserialize)]
        struct EntryToml {
            format: String,
            path: String,
            backend: String,
            context_tokens: Option<u32>,
            estimated_memory_bytes: Option<u64>,
            #[serde(default)]
            roles: Vec<String>,
            #[serde(default)]
            execution_config: serde_json::Value,
            #[serde(default)]
            tool_calling: Option<bool>,
            #[serde(default)]
            reasoning: Option<bool>,
        }
        #[derive(Deserialize)]
        struct ModelsToml {
            #[serde(default)]
            roles: BTreeMap<String, RoleToml>,
            #[serde(default)]
            entries: BTreeMap<String, EntryToml>,
        }
        #[derive(Deserialize)]
        struct RootToml {
            models: Option<ModelsToml>,
        }

        let root: RootToml =
            toml::from_str(text).map_err(|e| RegistryError::new(format!("invalid TOML: {}", e)))?;
        let Some(models) = root.models else {
            return Ok(Self::default());
        };

        let entries: Vec<ModelEntry> = models
            .entries
            .into_iter()
            .map(|(id, e)| ModelEntry {
                id: ModelId::from(id),
                format: e.format,
                path: PathBuf::from(e.path),
                backend: e.backend,
                context_tokens: e.context_tokens,
                estimated_memory_bytes: e.estimated_memory_bytes,
                roles: e.roles.iter().map(|r| ModelRole::parse(r)).collect(),
                execution_config: e.execution_config,
                tool_calling: e.tool_calling,
                reasoning: e.reasoning,
            })
            .collect();

        let roles: BTreeMap<ModelRole, Vec<ModelId>> = models
            .roles
            .into_iter()
            .map(|(role, r)| {
                (
                    ModelRole::parse(&role),
                    r.candidates.into_iter().map(ModelId::from).collect(),
                )
            })
            .collect();

        Self::new(entries, roles)
    }

    pub fn get(&self, id: &ModelId) -> Option<&ModelEntry> {
        self.entries.get(id)
    }

    pub fn resolve(&self, id: &ModelId) -> Option<ResolvedModelEntry> {
        self.entries
            .get(id)
            .map(|entry| resolve_entry(entry.clone()))
    }

    /// Candidate models for a role, in configured priority order — the order the
    /// scheduler should try them in, all else being equal.
    pub fn candidates_for_role(&self, role: &ModelRole) -> Vec<&ModelEntry> {
        self.roles
            .get(role)
            .map(|ids| ids.iter().filter_map(|id| self.entries.get(id)).collect())
            .unwrap_or_default()
    }

    pub fn all_resolved(&self) -> Vec<ResolvedModelEntry> {
        self.entries.values().cloned().map(resolve_entry).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn resolve_entry(entry: ModelEntry) -> ResolvedModelEntry {
    match std::fs::metadata(&entry.path) {
        Ok(meta) if meta.is_file() => ResolvedModelEntry {
            file_size_bytes: Some(meta.len()),
            available: true,
            unavailable_reason: None,
            entry,
        },
        // Directories are valid for backends that use HuggingFace model dirs (e.g. mlx).
        Ok(meta) if meta.is_dir() => ResolvedModelEntry {
            file_size_bytes: None,
            available: true,
            unavailable_reason: None,
            entry,
        },
        Ok(_) => ResolvedModelEntry {
            available: false,
            unavailable_reason: Some(format!(
                "{} is neither a regular file nor a directory",
                entry.path.display()
            )),
            file_size_bytes: None,
            entry,
        },
        Err(e) => ResolvedModelEntry {
            available: false,
            unavailable_reason: Some(format!("{}: {}", entry.path.display(), e)),
            file_size_bytes: None,
            entry,
        },
    }
}

/// Scan a directory (recursively) for `.gguf` files. Read-only — does not write or
/// mutate any configuration; callers decide what, if anything, to do with the result.
pub fn discover_gguf_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = r#"
[models.roles.reasoning]
candidates = ["qwen3-14b-q4", "qwen3-8b-q6"]

[models.roles.coding]
candidates = ["qwen-coder-7b-q6", "qwen3-8b-q6"]

[models.entries.qwen3-14b-q4]
format = "gguf"
path = "/models/qwen3-14b-q4_k_m.gguf"
backend = "llama_cpp"
context_tokens = 32768
estimated_memory_bytes = 11000000000
roles = ["reasoning", "coding"]

[models.entries.qwen3-8b-q6]
format = "gguf"
path = "/models/qwen3-8b-q6_k.gguf"
backend = "llama_cpp"
context_tokens = 32768
roles = ["reasoning", "coding", "summarization"]

[models.entries.qwen-coder-7b-q6]
format = "gguf"
path = "/models/qwen-coder-7b-q6_k.gguf"
backend = "llama_cpp"
roles = ["coding"]
"#;

    #[test]
    fn execution_config_is_preserved_per_model() {
        let toml = r#"
[models.entries."qwen3-8b-q4"]
format = "gguf"
path = "/models/qwen3-8b.gguf"
backend = "llama_cpp"
roles = ["reasoning"]

[models.entries."qwen3-8b-q4".execution_config]
gpu_layers = -1
flash_attention = true
parallel = 2
cache_type_k = "q8_0"
"#;
        let registry = ModelRegistry::from_toml_str(toml).unwrap();
        let entry = registry.get(&ModelId::from("qwen3-8b-q4")).unwrap();
        assert_eq!(entry.execution_config["gpu_layers"], -1);
        assert_eq!(entry.execution_config["flash_attention"], true);
        assert_eq!(entry.execution_config["parallel"], 2);
        assert_eq!(entry.execution_config["cache_type_k"], "q8_0");
    }

    #[test]
    fn parses_the_example_config() {
        let registry = ModelRegistry::from_toml_str(EXAMPLE_TOML).unwrap();
        assert_eq!(registry.len(), 3);
        let reasoning = registry.candidates_for_role(&ModelRole::Reasoning);
        assert_eq!(reasoning.len(), 2);
        assert_eq!(reasoning[0].id, ModelId::from("qwen3-14b-q4"));
    }

    #[test]
    fn duplicate_model_id_is_rejected() {
        let entries = vec![
            ModelEntry {
                id: ModelId::from("a"),
                format: "gguf".to_string(),
                path: PathBuf::from("/x.gguf"),
                backend: "llama_cpp".to_string(),
                context_tokens: None,
                estimated_memory_bytes: None,
                roles: vec![],
                execution_config: serde_json::Value::Null,
                tool_calling: None,
                reasoning: None,
            },
            ModelEntry {
                id: ModelId::from("a"),
                format: "gguf".to_string(),
                path: PathBuf::from("/y.gguf"),
                backend: "llama_cpp".to_string(),
                context_tokens: None,
                estimated_memory_bytes: None,
                roles: vec![],
                execution_config: serde_json::Value::Null,
                tool_calling: None,
                reasoning: None,
            },
        ];
        let err = ModelRegistry::new(entries, BTreeMap::new()).unwrap_err();
        assert!(err.message.contains("duplicate model id"));
    }

    #[test]
    fn role_referencing_unknown_model_is_rejected() {
        let mut roles = BTreeMap::new();
        roles.insert(ModelRole::Reasoning, vec![ModelId::from("ghost")]);
        let err = ModelRegistry::new(vec![], roles).unwrap_err();
        assert!(err.message.contains("unknown model"));
    }

    #[test]
    fn missing_model_file_is_unavailable_not_a_crash() {
        let registry = ModelRegistry::from_toml_str(EXAMPLE_TOML).unwrap();
        let resolved = registry.resolve(&ModelId::from("qwen3-14b-q4")).unwrap();
        assert!(!resolved.available);
        assert!(resolved.unavailable_reason.is_some());
    }

    #[test]
    fn resolve_reports_real_file_size_for_an_existing_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"not a real gguf but a real file").unwrap();
        let entry = ModelEntry {
            id: ModelId::from("local"),
            format: "gguf".to_string(),
            path: tmp.path().to_path_buf(),
            backend: "llama_cpp".to_string(),
            context_tokens: None,
            estimated_memory_bytes: None,
            roles: vec![],
            execution_config: serde_json::Value::Null,
            tool_calling: None,
            reasoning: None,
        };
        let registry = ModelRegistry::new(vec![entry], BTreeMap::new()).unwrap();
        let resolved = registry.resolve(&ModelId::from("local")).unwrap();
        assert!(resolved.available);
        assert!(resolved.file_size_bytes.unwrap() > 0);
    }

    #[test]
    fn discover_gguf_files_finds_nested_files_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.gguf"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("b.gguf"), b"x").unwrap();

        let found = discover_gguf_files(dir.path());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "gguf"));
    }

    #[test]
    fn empty_models_section_yields_empty_registry() {
        let registry = ModelRegistry::from_toml_str("[trust]\nallow_shell = false\n").unwrap();
        assert!(registry.is_empty());
    }
}
