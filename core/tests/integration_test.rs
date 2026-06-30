#[cfg(test)]
mod integration_tests {
    use lao_orchestrator_core::{
        build_dag, load_workflow_yaml, validate_workflow_schema, ConditionOperator, ConditionType,
        LoopConfig, LoopItems, StepCondition, Workflow, WorkflowStep,
    };

    #[test]
    fn loop_workflow_schema_stays_supported() {
        let workflow = Workflow {
            workflow: "audio_batch_transcribe".to_string(),
            steps: vec![WorkflowStep {
                run: "WhisperPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: Some(2),
                retry_delay: Some(1000),
                cache_key: Some("audio_transcription".to_string()),
                input_from: None,
                depends_on: None,
                condition: None,
                for_each: Some(LoopConfig {
                    items: LoopItems::Array(vec![
                        serde_yaml::Value::String("audio1.mp3".to_string()),
                        serde_yaml::Value::String("audio2.mp3".to_string()),
                        serde_yaml::Value::String("audio3.mp3".to_string()),
                    ]),
                    var: "audio_file".to_string(),
                    collect_results: true,
                    max_parallel: 2,
                }),
            }],
        };

        validate_workflow_schema(&workflow).expect("loop workflow is supported");
        let step = &workflow.steps[0];
        assert!(step.for_each.is_some());
        assert!(step.cache_key.is_some());

        let loop_config = step.for_each.as_ref().unwrap();
        assert_eq!(loop_config.var, "audio_file");
        assert!(loop_config.collect_results);
        assert_eq!(loop_config.max_parallel, 2);
    }

    #[test]
    fn dag_construction_with_loop_and_dependency() {
        let workflow = Workflow {
            workflow: "loop_then_summarize".to_string(),
            steps: vec![
                WorkflowStep {
                    run: "EchoPlugin".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: Some("batch".to_string()),
                    input_from: None,
                    depends_on: None,
                    condition: None,
                    for_each: Some(LoopConfig {
                        items: LoopItems::Array(vec![
                            serde_yaml::Value::String("one".to_string()),
                            serde_yaml::Value::String("two".to_string()),
                        ]),
                        var: "item".to_string(),
                        collect_results: true,
                        max_parallel: 2,
                    }),
                },
                WorkflowStep {
                    run: "EchoPlugin".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: None,
                    input_from: Some("step1".to_string()),
                    depends_on: Some(vec!["step1".to_string()]),
                    condition: None,
                    for_each: None,
                },
            ],
        };

        let dag = build_dag(&workflow.steps).expect("dag builds");
        assert_eq!(dag.len(), 2);
        assert!(dag[0].step.for_each.is_some());
        assert_eq!(
            dag[1].parents,
            vec!["step1".to_string(), "step1".to_string()]
        );
    }

    #[test]
    fn condition_schema_stays_supported() {
        let workflow = Workflow {
            workflow: "conditional_process".to_string(),
            steps: vec![WorkflowStep {
                run: "EchoPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: Some(StepCondition {
                    condition_type: ConditionType::OutputContains,
                    field: "step1".to_string(),
                    operator: ConditionOperator::Equals,
                    value: "ready".to_string(),
                }),
                for_each: None,
            }],
        };

        validate_workflow_schema(&workflow).expect("condition workflow is supported");
        let condition = workflow.steps[0].condition.as_ref().unwrap();
        assert_eq!(condition.field, "step1");
        assert_eq!(condition.value, "ready");
    }

    #[test]
    fn retry_schema_stays_supported() {
        let step = WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::Value::Null,
            retries: Some(3),
            retry_delay: Some(2000),
            cache_key: Some("echo".to_string()),
            input_from: None,
            depends_on: None,
            condition: None,
            for_each: None,
        };

        assert_eq!(step.retries, Some(3));
        assert_eq!(step.retry_delay, Some(2000));
        assert!(step.cache_key.is_some());
    }

    #[test]
    fn unsupported_fields_are_rejected_from_yaml() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("unsupported.yaml");
        std::fs::write(
            &path,
            r#"
workflow: "unsupported"
steps:
  - run: EchoPlugin
    input: hello
    input_modality: text
"#,
        )
        .expect("write workflow");

        let err = load_workflow_yaml(path.to_str().unwrap()).expect_err("field is rejected");
        assert!(err.contains("Unsupported workflow field 'input_modality'"));
    }
}
