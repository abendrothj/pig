use lao_orchestrator_core::*;
use proptest::prelude::*;
use std::collections::HashMap;

// Property: substitute_vars never panics on any input
proptest! {
    #[test]
    fn substitute_vars_never_panics(s in ".*", key in "[a-z]+", value in ".*") {
        let mut outputs = HashMap::new();
        outputs.insert(key, value);
        // Just call it - the test passes if it doesn't panic
        let mut params = serde_yaml::Value::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String("input".to_string()),
                serde_yaml::Value::String(s),
            );
            m
        });
        lao_orchestrator_core::workflow_helpers::substitute_params(&mut params, &outputs);
    }
}

// Property: build_dag always produces nodes with correct IDs
proptest! {
    #[test]
    fn build_dag_produces_sequential_ids(n in 1usize..20) {
        let steps: Vec<WorkflowStep> = (0..n).map(|_| WorkflowStep {
            run: "Echo".to_string(),
            params: serde_yaml::Value::Null,
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: None,
            depends_on: None,
            condition: None,
            for_each: None,
        }).collect();

        let dag = build_dag(&steps).unwrap();
        prop_assert_eq!(dag.len(), n);
        for (i, node) in dag.iter().enumerate() {
            prop_assert_eq!(&node.id, &format!("step{}", i + 1));
        }
    }
}

// Property: topo_sort of a linear chain produces correct order
proptest! {
    #[test]
    fn topo_sort_linear_chain_is_ordered(n in 2usize..10) {
        let steps: Vec<WorkflowStep> = (0..n).map(|i| WorkflowStep {
            run: format!("Step{}", i + 1),
            params: serde_yaml::Value::Null,
            retries: None,
            retry_delay: None,
            cache_key: None,
            input_from: if i > 0 { Some(format!("step{}", i)) } else { None },
            depends_on: None,
            condition: None,
            for_each: None,
        }).collect();

        let dag = build_dag(&steps).unwrap();
        let order = topo_sort(&dag).unwrap();
        prop_assert_eq!(order.len(), n);
        // Each step should come after its dependency
        for i in 1..n {
            let dep_pos = order.iter().position(|x| x == &format!("step{}", i)).unwrap();
            let step_pos = order.iter().position(|x| x == &format!("step{}", i + 1)).unwrap();
            prop_assert!(dep_pos < step_pos, "step{} should come before step{}", i, i + 1);
        }
    }
}

// Property: build_plugin_input never panics with arbitrary YAML
proptest! {
    #[test]
    fn build_plugin_input_never_panics(input in "[a-zA-Z0-9 ]{0,100}") {
        let params = serde_yaml::Value::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String("input".to_string()),
                serde_yaml::Value::String(input),
            );
            m
        });
        let _ = lao_orchestrator_core::workflow_helpers::build_plugin_input(&params);
    }
}
