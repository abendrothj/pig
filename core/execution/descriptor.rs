//! Plugin descriptors: declared inputs, outputs, capabilities, and execution mode.
//!
//! Distinct from `lao_plugin_api::PluginInfo`, which is a raw parse of a plugin's C-ABI
//! metadata. A `PluginDescriptor` reconciles a plugin's declared `PluginCapability`
//! entries against the host's `CapabilityClass` vocabulary (`crate::trust`), and records
//! how the plugin executes. Every plugin loaded today is in-process (dlopen); no
//! out-of-process execution mode is implemented yet (see ADR 0005) — `synthetic_descriptor`
//! always reports `ExecutionMode::InProcess`.

use crate::trust::{capability_class_for_manifest, CapabilityClass};
use lao_plugin_api::{PluginInfo, PluginInputType, PluginOutputType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    InProcess,
    Subprocess,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginDescriptor {
    pub name: String,
    pub declared_inputs: Vec<PluginInputType>,
    pub declared_outputs: Vec<PluginOutputType>,
    pub capabilities: Vec<CapabilityClass>,
    pub execution_mode: ExecutionMode,
}

/// Synthesize a descriptor for a legacy (ABI v1/v2) plugin from its parsed `PluginInfo`.
/// Capability names the trust system doesn't recognize are dropped, not guessed at.
pub fn synthetic_descriptor(info: &PluginInfo) -> PluginDescriptor {
    let declared_inputs = info
        .capabilities
        .iter()
        .map(|c| c.input_type.clone())
        .collect();
    let declared_outputs = info
        .capabilities
        .iter()
        .map(|c| c.output_type.clone())
        .collect();
    let capabilities = info
        .capabilities
        .iter()
        .filter_map(|c| capability_class_for_manifest(&c.name))
        .collect();

    PluginDescriptor {
        name: info.name.clone(),
        declared_inputs,
        declared_outputs,
        capabilities,
        execution_mode: ExecutionMode::InProcess,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lao_plugin_api::PluginCapability;

    fn info_with_capability(name: &str, cap_name: &str) -> PluginInfo {
        PluginInfo {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            author: String::new(),
            dependencies: vec![],
            tags: vec![],
            capabilities: vec![PluginCapability {
                name: cap_name.to_string(),
                description: String::new(),
                input_type: PluginInputType::Text,
                output_type: PluginOutputType::Text,
            }],
            input_schema: None,
            output_schema: None,
        }
    }

    #[test]
    fn recognized_capability_name_maps_to_capability_class() {
        let info = info_with_capability("FileReadPlugin", "read-file");
        let d = synthetic_descriptor(&info);
        assert_eq!(d.name, "FileReadPlugin");
        assert_eq!(d.capabilities, vec![CapabilityClass::FilesystemRead]);
        assert_eq!(d.execution_mode, ExecutionMode::InProcess);
        assert_eq!(d.declared_inputs, vec![PluginInputType::Text]);
        assert_eq!(d.declared_outputs, vec![PluginOutputType::Text]);
    }

    #[test]
    fn unrecognized_capability_name_is_dropped_not_guessed() {
        let info = info_with_capability("MysteryPlugin", "does-not-exist");
        let d = synthetic_descriptor(&info);
        assert!(d.capabilities.is_empty());
    }

    #[test]
    fn plugin_with_no_capabilities_gets_empty_descriptor() {
        let info = PluginInfo {
            name: "EchoPlugin".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            author: String::new(),
            dependencies: vec![],
            tags: vec![],
            capabilities: vec![],
            input_schema: None,
            output_schema: None,
        };
        let d = synthetic_descriptor(&info);
        assert!(d.capabilities.is_empty());
        assert!(d.declared_inputs.is_empty());
        assert!(d.declared_outputs.is_empty());
    }
}
