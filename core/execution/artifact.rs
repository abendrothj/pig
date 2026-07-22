//! Structured artifact model for step outputs.
//!
//! Replaces the implicit "a step produces one output string" convention with a typed
//! value that can represent text, structured JSON, files, command results, or code-graph
//! query results, while still bridging back to a plain string for legacy consumers.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Artifact {
    Null,
    Text(String),
    Json(serde_json::Value),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    File(FileArtifact),
    FileSet(Vec<FileArtifact>),
    CommandResult(CommandResultArtifact),
    CodeGraph(CodeGraphArtifact),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileArtifact {
    pub path: PathBuf,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandResultArtifact {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Provenance-carrying result of a read-only code-graph query. Populated by the
/// `CodeIntelligenceProvider` adapter (PR3/PR4); defined here now so `Artifact`'s shape
/// is stable from the start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeGraphArtifact {
    pub provider: String,
    pub provider_version: Option<String>,
    pub repo_root: PathBuf,
    pub git_revision: Option<String>,
    pub dirty: bool,
    pub indexed_at: Option<String>,
    pub operation: String,
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_artifact_round_trips() {
        let a = Artifact::Text("hello".to_string());
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn json_artifact_round_trips() {
        let a = Artifact::Json(serde_json::json!({"a": 1, "b": [1,2,3]}));
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn file_artifact_round_trips() {
        let a = Artifact::File(FileArtifact {
            path: PathBuf::from("/tmp/out.txt"),
            mime_type: Some("text/plain".to_string()),
            size_bytes: Some(42),
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn code_graph_artifact_round_trips() {
        let a = Artifact::CodeGraph(CodeGraphArtifact {
            provider: "codebase-memory-mcp".to_string(),
            provider_version: Some("0.9.0".to_string()),
            repo_root: PathBuf::from("/repo"),
            git_revision: Some("deadbeef".to_string()),
            dirty: false,
            indexed_at: None,
            operation: "search_graph".to_string(),
            payload: serde_json::json!({"total": 1}),
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn null_and_scalar_variants_round_trip() {
        for a in [
            Artifact::Null,
            Artifact::Integer(-7),
            Artifact::Float(1.5),
            Artifact::Boolean(true),
        ] {
            let json = serde_json::to_string(&a).unwrap();
            let back: Artifact = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back);
        }
    }
}
