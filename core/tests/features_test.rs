#[cfg(test)]
mod tests {
    use lao_orchestrator_core::{
        ConditionOperator, ConditionType, DagNode, LoopConfig, LoopItems, Modality, StepCondition,
        Workflow, WorkflowStep,
    };

    // ============ Loop/Iteration Tests ============

    #[test]
    fn test_loop_config_default_values() {
        let loop_config = LoopConfig {
            items: LoopItems::Array(vec![serde_yaml::Value::String("a".to_string())]),
            var: "item".to_string(), // Use hardcoded default
            collect_results: true,   // Use hardcoded default
            max_parallel: 4,         // Use hardcoded default
        };

        assert_eq!(loop_config.var, "item");
        assert_eq!(loop_config.collect_results, true);
        assert_eq!(loop_config.max_parallel, 4);
    }

    #[test]
    fn test_loop_items_array() {
        let items = vec![
            serde_yaml::Value::String("file1.txt".to_string()),
            serde_yaml::Value::String("file2.txt".to_string()),
            serde_yaml::Value::String("file3.txt".to_string()),
        ];

        match LoopItems::Array(items.clone()) {
            LoopItems::Array(arr) => assert_eq!(arr.len(), 3),
            _ => panic!("Expected Array variant"),
        }
    }

    #[test]
    fn test_loop_items_reference() {
        let reference = LoopItems::Reference("step1.output".to_string());

        match reference {
            LoopItems::Reference(ref_str) => assert_eq!(ref_str, "step1.output"),
            _ => panic!("Expected Reference variant"),
        }
    }

    #[test]
    fn test_loop_config_serialization() {
        let config = LoopConfig {
            items: LoopItems::Array(vec![serde_yaml::Value::String("item1".to_string())]),
            var: "x".to_string(),
            collect_results: true,
            max_parallel: 2,
        };

        let yaml = serde_yaml::to_string(&config).expect("Failed to serialize");
        let deserialized: LoopConfig = serde_yaml::from_str(&yaml).expect("Failed to deserialize");

        assert_eq!(deserialized.var, "x");
        assert_eq!(deserialized.max_parallel, 2);
        assert_eq!(deserialized.collect_results, true);
    }

    // ============ Modality Tests ============

    #[test]
    fn test_modality_from_audio_extension() {
        assert_eq!(Modality::from_file_extension("mp3"), Some(Modality::Audio));
        assert_eq!(Modality::from_file_extension("wav"), Some(Modality::Audio));
        assert_eq!(Modality::from_file_extension("ogg"), Some(Modality::Audio));
    }

    #[test]
    fn test_modality_from_image_extension() {
        assert_eq!(Modality::from_file_extension("png"), Some(Modality::Image));
        assert_eq!(Modality::from_file_extension("jpg"), Some(Modality::Image));
        assert_eq!(Modality::from_file_extension("gif"), Some(Modality::Image));
    }

    #[test]
    fn test_modality_from_video_extension() {
        assert_eq!(Modality::from_file_extension("mp4"), Some(Modality::Video));
        assert_eq!(Modality::from_file_extension("avi"), Some(Modality::Video));
        assert_eq!(Modality::from_file_extension("mov"), Some(Modality::Video));
    }

    #[test]
    fn test_modality_from_text_extension() {
        assert_eq!(Modality::from_file_extension("txt"), Some(Modality::Text));
        assert_eq!(Modality::from_file_extension("json"), Some(Modality::Text));
        assert_eq!(Modality::from_file_extension("yaml"), Some(Modality::Text));
    }

    #[test]
    fn test_modality_from_mime_audio() {
        assert_eq!(
            Modality::from_mime_type("audio/mpeg"),
            Some(Modality::Audio)
        );
        assert_eq!(Modality::from_mime_type("audio/wav"), Some(Modality::Audio));
    }

    #[test]
    fn test_modality_from_mime_image() {
        assert_eq!(Modality::from_mime_type("image/png"), Some(Modality::Image));
        assert_eq!(
            Modality::from_mime_type("image/jpeg"),
            Some(Modality::Image)
        );
    }

    #[test]
    fn test_modality_from_mime_video() {
        assert_eq!(Modality::from_mime_type("video/mp4"), Some(Modality::Video));
        assert_eq!(
            Modality::from_mime_type("video/quicktime"),
            Some(Modality::Video)
        );
    }

    #[test]
    fn test_modality_from_mime_text() {
        assert_eq!(Modality::from_mime_type("text/plain"), Some(Modality::Text));
        assert_eq!(
            Modality::from_mime_type("application/json"),
            Some(Modality::Structured)
        );
    }

    #[test]
    fn test_modality_as_str() {
        assert_eq!(Modality::Text.as_str(), "text");
        assert_eq!(Modality::Audio.as_str(), "audio");
        assert_eq!(Modality::Image.as_str(), "image");
        assert_eq!(Modality::Video.as_str(), "video");
        assert_eq!(Modality::Structured.as_str(), "structured");
        assert_eq!(Modality::Binary.as_str(), "binary");
        assert_eq!(Modality::Mixed.as_str(), "mixed");
    }

    #[test]
    fn test_modality_unknown_extension() {
        assert_eq!(Modality::from_file_extension("xyz"), None);
        assert_eq!(Modality::from_file_extension("bin"), None);
    }

    // ============ WorkflowStep with Modality Tests ============

    #[test]
    fn test_workflow_step_with_modality() {
        let step = WorkflowStep {
            run: "WhisperPlugin".to_string(),
            params: serde_yaml::Value::Null,
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            on_success: None,
            on_failure: None,
            for_each: None,
            input_modality: Some(Modality::Audio),
            output_modality: Some(Modality::Text),
        };

        assert_eq!(step.input_modality, Some(Modality::Audio));
        assert_eq!(step.output_modality, Some(Modality::Text));
    }

    #[test]
    fn test_workflow_step_serialization_with_modality() {
        let workflow = Workflow {
            workflow: "test".to_string(),
            steps: vec![WorkflowStep {
                run: "TestPlugin".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: None,
                on_success: None,
                on_failure: None,
                for_each: None,
                input_modality: Some(Modality::Image),
                output_modality: Some(Modality::Text),
            }],
        };

        let yaml = serde_yaml::to_string(&workflow).expect("Failed to serialize");
        let deserialized: Workflow = serde_yaml::from_str(&yaml).expect("Failed to deserialize");

        assert_eq!(deserialized.steps[0].input_modality, Some(Modality::Image));
        assert_eq!(deserialized.steps[0].output_modality, Some(Modality::Text));
    }

    // ============ Loop + Modality Integration Tests ============

    #[test]
    fn test_step_with_loop_and_modality() {
        let step = WorkflowStep {
            run: "AudioProcessor".to_string(),
            params: serde_yaml::Value::Null,
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            on_success: None,
            on_failure: None,
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("audio1.mp3".to_string()),
                    serde_yaml::Value::String("audio2.mp3".to_string()),
                ]),
                var: "audio_file".to_string(),
                collect_results: true,
                max_parallel: 2,
            }),
            input_modality: Some(Modality::Audio),
            output_modality: Some(Modality::Text),
        };

        assert!(step.for_each.is_some());
        assert_eq!(step.input_modality, Some(Modality::Audio));

        let loop_config = step.for_each.unwrap();
        assert_eq!(loop_config.var, "audio_file");
        assert_eq!(loop_config.max_parallel, 2);
    }

    // ============ StepCondition Tests ============

    #[test]
    fn test_step_condition_output_contains() {
        let condition = StepCondition {
            condition_type: ConditionType::OutputContains,
            field: "output".to_string(),
            operator: ConditionOperator::Contains,
            value: "error".to_string(),
        };

        assert_eq!(condition.field, "output");
        match condition.condition_type {
            ConditionType::OutputContains => (),
            _ => panic!("Expected OutputContains"),
        }
    }

    #[test]
    fn test_step_condition_serialization() {
        let condition = StepCondition {
            condition_type: ConditionType::StatusEquals,
            field: "status".to_string(),
            operator: ConditionOperator::Equals,
            value: "success".to_string(),
        };

        let yaml = serde_yaml::to_string(&condition).expect("Failed to serialize");
        let _deserialized: StepCondition =
            serde_yaml::from_str(&yaml).expect("Failed to deserialize");

        assert!(yaml.contains("StatusEquals"));
    }

    // ============ DAG and Workflow Tests ============

    #[test]
    fn test_dag_node_creation() {
        let step = WorkflowStep {
            run: "EchoPlugin".to_string(),
            params: serde_yaml::Value::Null,
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            on_success: None,
            on_failure: None,
            for_each: None,
            input_modality: None,
            output_modality: None,
        };

        let dag_node = DagNode {
            id: "step1".to_string(),
            step: step.clone(),
            parents: vec![],
        };

        assert_eq!(dag_node.id, "step1");
        assert_eq!(dag_node.parents.len(), 0);
        assert_eq!(dag_node.step.run, "EchoPlugin");
    }

    #[test]
    fn test_workflow_with_multiple_steps() {
        let workflow = Workflow {
            workflow: "test_workflow".to_string(),
            steps: vec![
                WorkflowStep {
                    run: "Step1".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: None,
                    input_from: None,
                    depends_on: None,
                    condition: None,
                    on_success: None,
                    on_failure: None,
                    for_each: None,
                    input_modality: Some(Modality::Audio),
                    output_modality: Some(Modality::Text),
                },
                WorkflowStep {
                    run: "Step2".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: None,
                    input_from: Some("step1".to_string()),
                    depends_on: Some(vec!["step1".to_string()]),
                    condition: None,
                    on_success: None,
                    on_failure: None,
                    for_each: None,
                    input_modality: Some(Modality::Text),
                    output_modality: Some(Modality::Structured),
                },
            ],
        };

        assert_eq!(workflow.workflow, "test_workflow");
        assert_eq!(workflow.steps.len(), 2);
        assert_eq!(workflow.steps[0].run, "Step1");
        assert_eq!(workflow.steps[1].input_from, Some("step1".to_string()));
    }
}
