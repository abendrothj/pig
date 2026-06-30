# SummarizerPlugin

A plugin that summarizes input text. Useful for condensing transcripts, documents, or meeting notes.

## Input
- `input` (string): The text to summarize.

## Output
- (string): The summary of the input text.

## Example Workflow
```yaml
workflow: "Summarize Meeting"
steps:
  - run: WhisperPlugin
    input: "meeting.wav"
  - run: SummarizerPlugin
    input_from: step1
```

## Usage
Reference the plugin by name in your workflow YAML as shown above. The input can come from a previous step (for example, `WhisperPlugin`) or be provided directly. 