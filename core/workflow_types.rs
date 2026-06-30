use lao_plugin_api::{PluginInputType, PluginOutputType};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Workflow {
    pub workflow: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WorkflowStep {
    pub run: String,
    #[serde(flatten)]
    pub params: serde_yaml::Value,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub retry_delay: Option<u64>, // milliseconds
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub input_from: Option<String>,
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<StepCondition>,
    #[serde(default)]
    pub for_each: Option<LoopConfig>, // Loop iteration support
}

/// Loop/iteration configuration for processing arrays
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct LoopConfig {
    /// Array to iterate over (can be reference to previous step output or inline array)
    pub items: LoopItems,
    /// Variable name for current item (default: "item")
    #[serde(default = "default_loop_var")]
    pub var: String,
    /// Whether to collect results into an array (default: true)
    #[serde(default = "default_collect_results")]
    pub collect_results: bool,
    /// Maximum parallel iterations (default: 4)
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
#[serde(untagged)]
pub enum LoopItems {
    /// Reference to previous step output (e.g., "step1.output")
    Reference(String),
    /// Inline array of items
    Array(Vec<serde_yaml::Value>),
}

fn default_loop_var() -> String {
    "item".to_string()
}

fn default_collect_results() -> bool {
    true
}

fn default_max_parallel() -> usize {
    4
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct StepCondition {
    pub condition_type: ConditionType,
    pub field: String, // Which field to evaluate (output, status, error)

    pub operator: ConditionOperator,
    pub value: String, // Value to compare against
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub enum ConditionType {
    OutputContains,
    OutputEquals,
    StatusEquals,
    ErrorContains,
    PreviousStepStatus,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
}

#[derive(Debug)]
pub struct DagNode {
    pub id: String,
    pub step: WorkflowStep,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepLog {
    pub step: usize,
    pub step_id: String,
    pub runner: String,
    pub input: serde_yaml::Value,
    pub output: Option<String>,
    pub error: Option<String>,
    pub attempt: u32,
    pub input_type: Option<PluginInputType>,
    pub output_type: Option<PluginOutputType>,
    pub validation: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepEvent {
    pub step: usize,
    pub step_id: String,
    pub runner: String,
    pub status: String, // pending | running | success | error | cache | skipped
    pub attempt: u32,
    pub message: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
}
