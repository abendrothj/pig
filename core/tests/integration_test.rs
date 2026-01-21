#[cfg(test)]
mod integration_tests {
    use lao_orchestrator_core::{
        Workflow, WorkflowStep, LoopConfig, LoopItems, Modality, DagNode, StepCondition,
        ConditionType, ConditionOperator,
    };

    // ============ Loop + Modality Integration ============

    #[test]
    fn test_audio_loop_workflow() {
        // Scenario: Process multiple audio files in parallel, transcribe each
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
                on_success: None,
                on_failure: None,
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
                input_modality: Some(Modality::Audio),
                output_modality: Some(Modality::Text),
            }],
        };

        assert_eq!(workflow.steps.len(), 1);
        let step = &workflow.steps[0];

        assert_eq!(step.input_modality, Some(Modality::Audio));
        assert_eq!(step.output_modality, Some(Modality::Text));
        assert!(step.for_each.is_some());
        assert!(step.cache_key.is_some());

        let loop_config = step.for_each.as_ref().unwrap();
        assert_eq!(loop_config.var, "audio_file");
        assert_eq!(loop_config.collect_results, true);
        assert_eq!(loop_config.max_parallel, 2);
    }

    #[test]
    fn test_image_analysis_loop() {
        // Scenario: Process multiple images with modality tracking
        let workflow = Workflow {
            workflow: "batch_image_analysis".to_string(),
            steps: vec![
                WorkflowStep {
                    run: "ImageAnalyzer".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: Some("image_analysis".to_string()),
                    input_from: None,
                    depends_on: None,
                    condition: None,
                    on_success: None,
                    on_failure: None,
                    for_each: Some(LoopConfig {
                        items: LoopItems::Array(vec![
                            serde_yaml::Value::String("image1.jpg".to_string()),
                            serde_yaml::Value::String("image2.png".to_string()),
                        ]),
                        var: "image_file".to_string(),
                        collect_results: true,
                        max_parallel: 4,
                    }),
                    input_modality: Some(Modality::Image),
                    output_modality: Some(Modality::Structured),
                },
                WorkflowStep {
                    run: "SummarizerPlugin".to_string(),
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
                    input_modality: Some(Modality::Structured),
                    output_modality: Some(Modality::Text),
                },
            ],
        };

        // Verify first step (loop with modality)
        let step1 = &workflow.steps[0];
        assert_eq!(step1.input_modality, Some(Modality::Image));
        assert_eq!(step1.output_modality, Some(Modality::Structured));
        assert!(step1.for_each.is_some());

        // Verify second step (modality transformation)
        let step2 = &workflow.steps[1];
        assert_eq!(step2.input_modality, Some(Modality::Structured));
        assert_eq!(step2.output_modality, Some(Modality::Text));
        assert_eq!(step2.input_from, Some("step1".to_string()));
    }

    // ============ Condition + Modality Integration ============

    #[test]
    fn test_conditional_modality_processing() {
        // Scenario: Process only if modality matches expectations
        let workflow = Workflow {
            workflow: "conditional_audio_process".to_string(),
            steps: vec![WorkflowStep {
                run: "AudioProcessor".to_string(),
                params: serde_yaml::Value::Null,
                retries: None,
                retry_delay: None,
                cache_key: None,
                input_from: None,
                depends_on: None,
                condition: Some(StepCondition {
                    condition_type: ConditionType::OutputContains,
                    field: "modality".to_string(),
                    operator: ConditionOperator::Equals,
                    value: "audio".to_string(),
                }),
                on_success: None,
                on_failure: None,
                for_each: None,
                input_modality: Some(Modality::Audio),
                output_modality: Some(Modality::Text),
            }],
        };

        let step = &workflow.steps[0];
        assert!(step.condition.is_some());
        assert_eq!(step.input_modality, Some(Modality::Audio));

        let condition = step.condition.as_ref().unwrap();
        assert_eq!(condition.field, "modality");
        assert_eq!(condition.value, "audio");
    }

    // ============ Retry + Modality Integration ============

    #[test]
    fn test_retry_on_modality_conversion_failure() {
        // Scenario: Retry audio-to-text conversion up to 3 times
        let step = WorkflowStep {
            run: "WhisperPlugin".to_string(),
            params: serde_yaml::Value::Null,
            retries: Some(3),
            retry_delay: Some(2000), // 2 second delay between retries
            cache_key: Some("transcription".to_string()),
            input_from: None,
            depends_on: None,
            condition: None,
            on_success: Some(vec!["next_step".to_string()]),
            on_failure: Some(vec!["error_handler".to_string()]),
            for_each: None,
            input_modality: Some(Modality::Audio),
            output_modality: Some(Modality::Text),
        };

        assert_eq!(step.retries, Some(3));
        assert_eq!(step.retry_delay, Some(2000));
        assert!(step.cache_key.is_some());
        assert_eq!(step.on_success, Some(vec!["next_step".to_string()]));
        assert_eq!(step.on_failure, Some(vec!["error_handler".to_string()]));
    }

    // ============ Loop + Retry + Modality Integration ============

    #[test]
    fn test_resilient_batch_processing() {
        // Scenario: Process batch of audio files with retry on failure
        let step = WorkflowStep {
            run: "VideoFrameExtractor".to_string(),
            params: serde_yaml::Value::Null,
            retries: Some(2),
            retry_delay: Some(1000),
            cache_key: Some("video_frames".to_string()),
            input_from: None,
            depends_on: None,
            condition: None,
            on_success: None,
            on_failure: None,
            for_each: Some(LoopConfig {
                items: LoopItems::Array(vec![
                    serde_yaml::Value::String("video1.mp4".to_string()),
                    serde_yaml::Value::String("video2.mp4".to_string()),
                ]),
                var: "video_file".to_string(),
                collect_results: true,
                max_parallel: 1, // Process videos sequentially due to memory
            }),
            input_modality: Some(Modality::Video),
            output_modality: Some(Modality::Image),
        };

        // All features combined:
        assert!(step.for_each.is_some()); // Loop
        assert_eq!(step.retries, Some(2)); // Retry
        assert_eq!(step.input_modality, Some(Modality::Video)); // Modality in
        assert_eq!(step.output_modality, Some(Modality::Image)); // Modality out

        let loop_cfg = step.for_each.as_ref().unwrap();
        assert_eq!(loop_cfg.max_parallel, 1); // Sequential for memory-heavy task
    }

    // ============ Complex Pipeline Tests ============

    #[test]
    fn test_multimodal_pipeline_with_loops() {
        // Scenario: Complex workflow with multiple modalities and loops
        let workflow = Workflow {
            workflow: "complex_multimodal".to_string(),
            steps: vec![
                // Step 1: Batch transcribe audio files
                WorkflowStep {
                    run: "WhisperPlugin".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: Some(2),
                    retry_delay: None,
                    cache_key: Some("transcriptions".to_string()),
                    input_from: None,
                    depends_on: None,
                    condition: None,
                    on_success: None,
                    on_failure: None,
                    for_each: Some(LoopConfig {
                        items: LoopItems::Reference("input_files".to_string()),
                        var: "file".to_string(),
                        collect_results: true,
                        max_parallel: 2,
                    }),
                    input_modality: Some(Modality::Audio),
                    output_modality: Some(Modality::Text),
                },
                // Step 2: Summarize all transcriptions
                WorkflowStep {
                    run: "SummarizerPlugin".to_string(),
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
                    output_modality: Some(Modality::Text),
                },
                // Step 3: Extract insights as JSON
                WorkflowStep {
                    run: "PromptDispatcherPlugin".to_string(),
                    params: serde_yaml::Value::Null,
                    retries: None,
                    retry_delay: None,
                    cache_key: None,
                    input_from: Some("step2".to_string()),
                    depends_on: Some(vec!["step2".to_string()]),
                    condition: None,
                    on_success: None,
                    on_failure: None,
                    for_each: None,
                    input_modality: Some(Modality::Text),
                    output_modality: Some(Modality::Structured),
                },
            ],
        };

        // Verify all steps
        assert_eq!(workflow.steps.len(), 3);

        // Step 1: Audio -> Text with loop
        assert_eq!(workflow.steps[0].input_modality, Some(Modality::Audio));
        assert_eq!(workflow.steps[0].output_modality, Some(Modality::Text));
        assert!(workflow.steps[0].for_each.is_some());

        // Step 2: Text -> Text (summarization)
        assert_eq!(workflow.steps[1].input_modality, Some(Modality::Text));
        assert_eq!(workflow.steps[1].output_modality, Some(Modality::Text));
        assert_eq!(workflow.steps[1].input_from, Some("step1".to_string()));

        // Step 3: Text -> Structured (insights)
        assert_eq!(workflow.steps[2].input_modality, Some(Modality::Text));
        assert_eq!(
            workflow.steps[2].output_modality,
            Some(Modality::Structured)
        );
        assert_eq!(workflow.steps[2].input_from, Some("step2".to_string()));
    }

    #[test]
    fn test_dag_construction_with_loops() {
        // Build DAG with loop nodes
        let step1 = WorkflowStep {
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
                    serde_yaml::Value::String("f1".to_string()),
                    serde_yaml::Value::String("f2".to_string()),
                ]),
                var: "f".to_string(),
                collect_results: true,
                max_parallel: 2,
            }),
            input_modality: Some(Modality::Audio),
            output_modality: Some(Modality::Text),
        };

        let node1 = DagNode {
            id: "step1".to_string(),
            step: step1,
            parents: vec![],
        };

        assert!(node1.step.for_each.is_some());
        assert_eq!(node1.parents.len(), 0);

        let step2 = WorkflowStep {
            run: "Summarizer".to_string(),
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
            output_modality: Some(Modality::Text),
        };

        let node2 = DagNode {
            id: "step2".to_string(),
            step: step2,
            parents: vec!["step1".to_string()],
        };

        assert_eq!(node2.parents.len(), 1);
        assert_eq!(node2.parents[0], "step1");
    }
}
