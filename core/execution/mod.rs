//! Structured execution core: typed artifacts, explicit step outcomes, plugin
//! descriptors, and the legacy-ABI adapter that quarantines the empty-output/`error:`
//! conventions away from the rest of the engine.

pub mod artifact;
pub mod descriptor;
pub mod legacy_adapter;
pub mod result;

pub use artifact::{Artifact, CodeGraphArtifact, CommandResultArtifact, FileArtifact};
pub use descriptor::{synthetic_descriptor, ExecutionMode, PluginDescriptor};
pub use result::{StepError, StepErrorKind, StepMetadata, StepResult, StepStatus};
