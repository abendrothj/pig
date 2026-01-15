# LAO Prompt Library

A collection of sample prompts and their expected YAML workflow outputs for testing, fine-tuning, and validation.

**Note**: The JSON file (`prompt_library.json`) is the source of truth. This markdown file is for human readability.

## Examples

### Summarize this Markdown doc and extract key ideas
```yaml
workflow: "Markdown Summary"
steps:
  - run: EchoPlugin
    input: "doc.md"
  - run: SummarizerPlugin
    input_from: step1
```

### Audit this config file for security risks
```yaml
workflow: "Config Audit"
steps:
  - run: EchoPlugin
    input: "config.yaml"
  - run: EchoPlugin
    input_from: step1
  - run: EchoPlugin
    input_from: step2
```

### Refactor this Rust file and add comments
```yaml
workflow: "Rust Refactor"
steps:
  - run: EchoPlugin
    input: "main.rs"
  - run: SummarizerPlugin
    input_from: step1
```

### Summarize this audio and create a todo list
```yaml
workflow: "Audio Todo"
steps:
  - run: WhisperPlugin
    input: "meeting.wav"
  - run: SummarizerPlugin
    input_from: step1
```

### Process video file and extract audio for transcription
```yaml
workflow: "Video Transcription"
steps:
  - run: EchoPlugin
    input: "presentation.mp4"
  - run: EchoPlugin
    input_from: step1
  - run: WhisperPlugin
    input_from: step2
  - run: SummarizerPlugin
    input_from: step3
```

## Key Patterns

All workflows follow these rules:
- Use step references (`step1`, `step2`) for `input_from` and `depends_on`
- Use only available plugins: `EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`, `PromptDispatcherPlugin`
- Step references are based on position (first step = step1, second = step2, etc.) 