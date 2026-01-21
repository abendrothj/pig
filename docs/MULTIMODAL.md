# Multimodal Workflow Support

LAO now supports **multimodal workflows** - workflows that handle multiple types of data (text, audio, images, video) with automatic modality detection and type-safe connections between steps.

## Features

### 🔄 Modality Types
- **Text**: Plain text, Markdown, JSON
- **Audio**: MP3, WAV, OGG, FLAC, AAC, M4A
- **Image**: PNG, JPG, GIF, WebP, BMP, SVG
- **Video**: MP4, AVI, MOV, MKV, WebM, FLV
- **Structured**: JSON/YAML formatted data
- **Binary**: Raw binary data
- **Mixed**: Multiple modalities

### 🎯 File Attachment System
- Drag-and-drop file support in graph editor
- Automatic modality detection from:
  - File extensions
  - MIME types
  - User-specified modality
- Modality badges with visual indicators (🔊 audio, 🖼️ image, 🎬 video, 📄 text)

### 🔗 Modality-aware Connections
- Specify `input_modality` and `output_modality` in workflow steps
- Visual flow diagram showing modality transformations
- Validation of modality compatibility

### 📊 Multimodal Pipeline Examples
All examples use modality hints to process multiple input types.

## Workflow Syntax

### Basic Multimodal Step

```yaml
workflow: audio_to_insights
steps:
  - run: WhisperPlugin
    input_modality: audio
    output_modality: text
    params:
      model: "base"
      language: "auto"
    
  - run: SummarizerPlugin
    input_from: step1
    input_modality: text
    output_modality: text
    params:
      max_length: 200
    depends_on:
      - step1
```

### Modality Specifications

In `WorkflowStep`:
- `input_modality`: Specifies the type of data the step expects
- `output_modality`: Specifies what type of data the step produces

Valid values:
- `text` - Plain text or structured text
- `audio` - Audio data (transcriptions, speech)
- `image` - Image data (photos, diagrams)
- `video` - Video data
- `structured` - JSON/YAML structured data
- `binary` - Raw binary data
- `mixed` - Multiple modality types

## File Handling in UI

### Modality Detection
Files are automatically analyzed for modality:

```
file.mp3  → 🔊 audio
file.jpg  → 🖼️ image  
file.mp4  → 🎬 video
file.txt  → 📄 text
file.json → 📊 structured
```

### Drag-and-Drop
1. Select a workflow node in the graph editor
2. Drag files from your filesystem to the node
3. Files are automatically categorized by modality
4. Modality icons show the file type

### File Browser
Click "📁 Browse..." to select files from the file picker dialog. File attachments are stored per node.

## UI Panels

### Modality Flow Panel (Cmd+O)
Shows the modality transformation pipeline:
- Input modality
- Step-by-step transformations
- Output modality
- Visual warnings for complex conversions

### Example Output
```
🔄 Modality Flow

Input: 🔊 Audio

Pipeline:
1. step1 → 📝 Text
2. step2 → 📊 JSON
3. step3 → 📝 Text

Output: 📝 Text
```

## Example Workflows

### 1. Audio Analysis Pipeline
**File**: `workflows/multimodal_analysis.yaml`

Transcribes audio, summarizes, and extracts insights:
```
Audio → Transcription → Summary → Insights (JSON)
```

### 2. Format Conversion
**File**: `workflows/multimodal_pipeline.yaml`

Demonstrates audio-to-text pipeline with caching:
```
Audio → Text (cached) → Summary → Structured Output
```

## Plugin Support

### Built-in Multimodal Plugins
- **WhisperPlugin**: Audio → Text (speech-to-text)
- **SummarizerPlugin**: Text → Text (condensed)
- **PromptDispatcherPlugin**: Text → Structured (LLM processing)
- **MultimodalPlugin**: Format conversion and modality detection

### Creating Multimodal Plugins
Plugins declare capabilities through metadata:

```rust
// In plugin implementation
capabilities: ["audio_to_text", "image_to_text", "format_conversion"]
input_schema: {"type": "object", "modality": "string"}
output_schema: {"type": "object", "modality": "string"}
```

## CLI Usage

### Validate Multimodal Workflow
```bash
cd core
../target/release/lao-cli validate ../workflows/multimodal_analysis.yaml
```

### Run Multimodal Workflow
```bash
cd core
../target/release/lao-cli run ../workflows/multimodal_analysis.yaml
```

### Plugin Capabilities
```bash
cd core
../target/release/lao-cli plugin-list
# Shows each plugin's input/output modalities
```

## Best Practices

### 1. Always Specify Modalities
```yaml
steps:
  - run: MyPlugin
    input_modality: audio      # ✅ Explicit
    output_modality: text      # ✅ Explicit
```

### 2. Use Caching for Expensive Conversions
```yaml
steps:
  - run: WhisperPlugin
    input_modality: audio
    output_modality: text
    cache_key: "transcription"  # Cache audio transcriptions
    params:
      model: "large"            # More expensive models
```

### 3. Loop Over Multiple Modalities
```yaml
steps:
  - run: AudioProcessor
    input_modality: audio
    output_modality: text
    for_each:
      items: 
        - "file1.mp3"
        - "file2.mp3"
        - "file3.mp3"
      var: "audio_file"
      max_parallel: 2           # Process 2 files in parallel
```

### 4. Visualize Modality Flow
Use the modality panel (Cmd+O) to verify transformations before running:
- Check for unexpected conversions
- Identify information loss points
- Validate plugin compatibility

## Performance Considerations

### Modality-aware Caching
- Transcriptions (audio→text) are cached by file hash
- Image analysis results cached separately
- Different modalities have different cache lifetimes

### Parallel Processing
When processing multiple files of the same modality:
```yaml
for_each:
  items: ["audio1.mp3", "audio2.mp3", "audio3.mp3"]
  max_parallel: 4  # Adjust based on system capacity
```

### Memory Usage
Video processing can be memory-intensive:
- Chunk large videos into frames
- Use `max_parallel: 1` for video workflows
- Monitor memory in Metrics Dashboard (Cmd+M)

## Troubleshooting

### Plugin Not Found
Ensure the plugin supports the specified modality:
```bash
cd core
../target/release/lao-cli plugin-list
# Check each plugin's capabilities
```

### Modality Mismatch
Error: "Cannot convert audio to binary"
- Check step's `input_modality` matches previous step's `output_modality`
- Use `MultimodalPlugin` for explicit conversions

### File Attachment Issues
- Ensure file path is absolute
- Check file permissions (readable)
- Verify modality is correctly detected in UI

## Future Enhancements

- [ ] 3D model support (GLB, USDZ, OBJ)
- [ ] Point cloud processing (LAS, PLY)
- [ ] Time series data support
- [ ] Custom modality types
- [ ] Modality-specific retry strategies
- [ ] Intelligent codec selection for video
- [ ] Cross-modal search and retrieval
