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
