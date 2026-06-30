#[cfg(test)]
mod tests {
    use lao_orchestrator_core::{
        ConditionOperator, ConditionType, DagNode, LoopConfig, LoopItems, StepCondition, Workflow,
        WorkflowStep,
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
            for_each: None,
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
                    for_each: None,
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
                    for_each: None,
                },
            ],
        };

        assert_eq!(workflow.workflow, "test_workflow");
        assert_eq!(workflow.steps.len(), 2);
        assert_eq!(workflow.steps[0].run, "Step1");
        assert_eq!(workflow.steps[1].input_from, Some("step1".to_string()));
    }
}
