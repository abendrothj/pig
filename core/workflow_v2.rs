//! Workflow schema v2: explicit step IDs and named-output references.
//!
//! v2 workflows are normalized into the exact same `Workflow`/`WorkflowStep` shape v1
//! produces: named `id`/`input_from: {step, output}` references are resolved to the
//! classic positional `step{n}` id scheme right here, at the parsing boundary. Every
//! downstream consumer — DAG building, execution, caching, condition evaluation, CLI —
//! needs no v2-awareness at all; there is exactly one internal representation and one
//! execution engine (ADR 0002), just as there is for v1.
//!
//! Only the default `"output"` name is resolvable today: a step still produces exactly
//! one output (see `crate::execution::StepResult`/legacy adapter), so `output: <name>`
//! is validated against that single name rather than silently accepted.

use crate::workflow_types::{LoopConfig, StepCondition, Workflow, WorkflowStep};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct WorkflowV2 {
    pub workflow: String,
    pub schema_version: u32,
    pub steps: Vec<WorkflowStepV2>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowStepV2 {
    pub id: String,
    pub run: String,
    #[serde(flatten)]
    pub params: serde_yaml::Value,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub retry_delay: Option<u64>,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub input_from: Option<OutputRef>,
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<StepCondition>,
    #[serde(default)]
    pub for_each: Option<LoopConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputRef {
    pub step: String,
    #[serde(default = "default_output_name")]
    pub output: String,
}

fn default_output_name() -> String {
    "output".to_string()
}

/// Normalize a v2 workflow into v1's `Workflow`/`WorkflowStep` shape.
pub fn normalize_v2(v2: WorkflowV2) -> Result<Workflow, String> {
    let mut position_of: HashMap<String, usize> = HashMap::new();
    for (index, step) in v2.steps.iter().enumerate() {
        if position_of.insert(step.id.clone(), index).is_some() {
            return Err(format!(
                "workflow schema v2: duplicate step id '{}'",
                step.id
            ));
        }
    }

    let resolve =
        |from_id: &str, referencing_step: &str, ref_kind: &str| -> Result<String, String> {
            position_of
                .get(from_id)
                .map(|idx| format!("step{}", idx + 1))
                .ok_or_else(|| {
                    format!(
                        "workflow schema v2: step '{}' has {} referencing unknown step '{}'",
                        referencing_step, ref_kind, from_id
                    )
                })
        };

    let mut steps = Vec::with_capacity(v2.steps.len());
    for step in v2.steps {
        let input_from = match step.input_from {
            Some(r) => {
                if r.output != "output" {
                    return Err(format!(
                        "workflow schema v2: step '{}' references output '{}' but only the \
                         default 'output' artifact is produced today",
                        step.id, r.output
                    ));
                }
                Some(resolve(&r.step, &step.id, "input_from")?)
            }
            None => None,
        };

        let depends_on = match step.depends_on {
            Some(deps) => Some(
                deps.iter()
                    .map(|d| resolve(d, &step.id, "depends_on"))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => None,
        };

        steps.push(WorkflowStep {
            run: step.run,
            params: step.params,
            retries: step.retries,
            retry_delay: step.retry_delay,
            cache_key: step.cache_key,
            input_from,
            depends_on,
            condition: step.condition,
            for_each: step.for_each,
        });
    }

    Ok(Workflow {
        workflow: v2.workflow,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Workflow, String> {
        let v2: WorkflowV2 = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        normalize_v2(v2)
    }

    #[test]
    fn named_input_from_resolves_to_positional_id() {
        let wf = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: locate_execution
    run: EchoPlugin
    input: "hi"
  - id: summarize
    run: EchoPlugin
    input_from:
      step: locate_execution
      output: output
"#,
        )
        .unwrap();
        assert_eq!(wf.steps.len(), 2);
        assert_eq!(wf.steps[1].input_from.as_deref(), Some("step1"));
    }

    #[test]
    fn input_from_output_defaults_when_omitted() {
        let wf = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "hi"
  - id: b
    run: EchoPlugin
    input_from:
      step: a
"#,
        )
        .unwrap();
        assert_eq!(wf.steps[1].input_from.as_deref(), Some("step1"));
    }

    #[test]
    fn depends_on_resolves_multiple_named_ids() {
        let wf = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "a"
  - id: b
    run: EchoPlugin
    input: "b"
  - id: merge
    run: EchoPlugin
    input_from:
      step: a
    depends_on: [a, b]
"#,
        )
        .unwrap();
        assert_eq!(
            wf.steps[2].depends_on.as_ref().unwrap(),
            &vec!["step1".to_string(), "step2".to_string()]
        );
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let err = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "1"
  - id: a
    run: EchoPlugin
    input: "2"
"#,
        )
        .unwrap_err();
        assert!(err.contains("duplicate step id 'a'"));
    }

    #[test]
    fn unknown_input_from_reference_is_rejected() {
        let err = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "1"
    input_from:
      step: does_not_exist
"#,
        )
        .unwrap_err();
        assert!(err.contains("unknown step 'does_not_exist'"));
    }

    #[test]
    fn unknown_depends_on_reference_is_rejected() {
        let err = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "1"
    depends_on: [ghost]
"#,
        )
        .unwrap_err();
        assert!(err.contains("unknown step 'ghost'"));
    }

    #[test]
    fn non_default_output_name_is_rejected() {
        let err = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    input: "1"
  - id: b
    run: EchoPlugin
    input_from:
      step: a
      output: result
"#,
        )
        .unwrap_err();
        assert!(err.contains("only the default 'output' artifact"));
    }

    #[test]
    fn for_each_and_condition_survive_normalization() {
        let wf = parse(
            r#"
workflow: "V2"
schema_version: 2
steps:
  - id: a
    run: EchoPlugin
    for_each:
      items: ["x", "y"]
      var: item
  - id: b
    run: EchoPlugin
    input: "check"
    condition:
      condition_type: OutputContains
      field: output
      operator: Contains
      value: "x"
"#,
        )
        .unwrap();
        assert!(wf.steps[0].for_each.is_some());
        assert!(wf.steps[1].condition.is_some());
    }
}
