# LAO Multimodal Processing Guide

**Complete guide to handling images, audio, video, and mixed media in LAO workflows**

---

## Overview

LAO supports **7 modality types** with automatic detection and conversion:

| Modality | File Extensions | MIME Types | Use Cases |
|----------|----------------|------------|-----------|
| **Audio** | `.mp3`, `.wav`, `.ogg`, `.flac`, `.m4a` | `audio/*` | Transcription, speech analysis, music |
| **Image** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.bmp`, `.svg` | `image/*` | OCR, object detection, classification |
| **Video** | `.mp4`, `.avi`, `.mov`, `.mkv`, `.webm` | `video/*` | Frame extraction, scene analysis |
| **Text** | `.txt`, `.md`, `.json`, `.yaml`, `.csv` | `text/*` | NLP, summarization, analysis |
| **Binary** | `.pdf`, `.docx`, `.xlsx` | `application/*` | Document parsing |
| **Structured** | `.json`, `.yaml`, `.xml` | `application/json` | Data processing |
| **Unknown** | Other extensions | - | Fallback handling |

---

## Image Processing

### Basic Image Analysis

```yaml
name: "simple_image_ocr"
steps:
  - id: extract_text
    runner: MultimodalPlugin
    input:
      file: "invoice.png"
      task: "ocr"
    input_modality: Image
    output_modality: Text
```

### Batch Image Processing with Loops

```yaml
name: "batch_images"
steps:
  - id: process_photos
    runner: MultimodalPlugin
    input:
      task: "analyze"
      for_each:
        items: ["photo1.jpg", "photo2.png", "photo3.gif"]
        var: "photo"
        max_parallel: 3
      file: "${photo}"
    input_modality: Image
    output_modality: Structured
```

### Image Object Detection

```yaml
name: "detect_objects"
steps:
  - id: find_objects
    runner: MultimodalPlugin
    input:
      file: "street_scene.jpg"
      task: "detect_objects"
      confidence: 0.7
    input_modality: Image
    output_modality: Structured
    
  - id: describe_objects
    runner: OllamaPlugin
    input:
      prompt: "Describe the detected objects"
      objects: "$find_objects.output"
    input_modality: Structured
    output_modality: Text
```

---

## Video Processing

### Extract Frames from Video

```yaml
name: "video_frames"
steps:
  - id: extract
    runner: MultimodalPlugin
    input:
      file: "video.mp4"
      task: "extract_frames"
      fps: 1  # 1 frame per second
    input_modality: Video
    output_modality: Image
```

### Video Scene Analysis

```yaml
name: "video_analysis"
steps:
  - id: extract_frames
    runner: MultimodalPlugin
    input:
      file: "presentation.mp4"
      task: "extract_frames"
      fps: 0.5
    input_modality: Video
    output_modality: Image
    
  - id: analyze_scenes
    runner: MultimodalPlugin
    input:
      for_each:
        items: "${extract_frames.frames}"
        var: "frame"
        max_parallel: 4
      file: "${frame}"
      task: "analyze"
    input_modality: Image
    output_modality: Structured
    
  - id: summarize
    runner: SummarizerPlugin
    input:
      scenes: "$analyze_scenes.output"
    input_modality: Structured
    output_modality: Text
```

---

## Audio Processing

### Audio Transcription

```yaml
name: "transcribe_audio"
steps:
  - id: transcribe
    runner: WhisperPlugin
    input:
      file: "recording.mp3"
      language: "en"
    input_modality: Audio
    output_modality: Text
```

### Batch Audio Processing

```yaml
name: "batch_audio"
steps:
  - id: transcribe_all
    runner: WhisperPlugin
    input:
      for_each:
        items:
          - "meeting1.mp3"
          - "meeting2.wav"
          - "interview.ogg"
        var: "audio_file"
        max_parallel: 2
      file: "${audio_file}"
    input_modality: Audio
    output_modality: Text
```

---

## Mixed Media Processing

### Automatic Modality Detection

LAO automatically detects modality from file extensions:

```yaml
name: "auto_detect"
steps:
  - id: process_files
    runner: MultimodalPlugin
    input:
      for_each:
        items:
          - "photo.jpg"      # Auto-detected as Image
          - "audio.mp3"      # Auto-detected as Audio
          - "video.mp4"      # Auto-detected as Video
          - "document.pdf"   # Auto-detected as Binary
        var: "file"
        max_parallel: 2
      file: "${file}"
      auto_detect: true
    # No input_modality = auto-detect
    output_modality: Structured
```

### Cross-Media Analysis

```yaml
name: "mixed_media"
steps:
  - id: process_all
    runner: MultimodalPlugin
    input:
      for_each:
        items: ["image.png", "audio.mp3", "notes.txt"]
        var: "media"
        collect_results: true
      file: "${media}"
      auto_detect: true
    output_modality: Structured
    
  - id: analyze_together
    runner: OllamaPlugin
    input:
      prompt: "Find common themes across all media"
      data: "$process_all.output"
    input_modality: Structured
    output_modality: Text
```

---

## Modality Transformations

### Common Transformation Patterns

| From | To | Use Case | Example |
|------|-----|----------|---------|
| Image → Text | OCR text extraction | `task: "ocr"` |
| Image → Structured | Object detection | `task: "detect_objects"` |
| Video → Image | Frame extraction | `task: "extract_frames"` |
| Audio → Text | Transcription | `runner: WhisperPlugin` |
| Text → Structured | NLP analysis | `task: "analyze"` |
| Structured → Text | Report generation | `runner: SummarizerPlugin` |

### Chaining Modality Conversions

```yaml
name: "multi_transform"
steps:
  # Video → Image
  - id: frames
    runner: MultimodalPlugin
    input:
      file: "video.mp4"
      task: "extract_frames"
    input_modality: Video
    output_modality: Image
    
  # Image → Structured
  - id: detect
    runner: MultimodalPlugin
    input:
      frames: "$frames.output"
      task: "detect_objects"
    input_modality: Image
    output_modality: Structured
    
  # Structured → Text
  - id: describe
    runner: OllamaPlugin
    input:
      data: "$detect.output"
    input_modality: Structured
    output_modality: Text
```

---

## UI Integration

### File Upload with Modality Detection

The LAO UI automatically detects modality when you drag-drop files:

```rust
// Automatic modality detection in UI
let modality = Modality::from_file_extension(&file_path);

// Visual indicators:
// 🎵 Audio files (blue)
// 🖼️ Image files (green)
// 🎬 Video files (purple)
// 📄 Text files (gray)
```

### Modality Flow Visualization

Press **Cmd+O** to view the multimodal flow diagram:
- Shows modality transformations between steps
- Color-coded data flow
- Highlights incompatible connections

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| **Cmd+O** | Toggle Modality Flow Panel |
| **Cmd+M** | Toggle Metrics Dashboard |
| **Cmd+T** | Toggle Timeline View |

---

## Example Workflows

### 1. Image Analysis Workflow

**File**: [workflows/image_analysis.yaml](../workflows/image_analysis.yaml)
- OCR text extraction
- Object detection
- AI description generation
- Summary report

**Run**:
```bash
cd core
../target/release/lao-cli run ../workflows/image_analysis.yaml
```

### 2. Batch Image Processing

**File**: [workflows/image_batch_processing.yaml](../workflows/image_batch_processing.yaml)
- Process 5 images in parallel
- Generate captions for each
- Create markdown report

**Run**:
```bash
cd core
../target/release/lao-cli run ../workflows/image_batch_processing.yaml
```

### 3. Video Frame Analysis

**File**: [workflows/video_to_images.yaml](../workflows/video_to_images.yaml)
- Extract frames from video (1 FPS)
- Analyze each frame
- Generate scene timeline

**Run**:
```bash
cd core
../target/release/lao-cli run ../workflows/video_to_images.yaml
```

### 4. Mixed Media Processing

**File**: [workflows/mixed_media_processing.yaml](../workflows/mixed_media_processing.yaml)
- Process PDF, images, audio, video, text
- Automatic modality detection
- Cross-media analysis
- Comprehensive report

**Run**:
```bash
cd core
../target/release/lao-cli run ../workflows/mixed_media_processing.yaml
```

---

## Plugin Reference

### MultimodalPlugin

**Capabilities**:
- Format conversion
- Modality detection
- OCR (image → text)
- Object detection (image → structured)
- Frame extraction (video → image)

**Usage**:
```yaml
- id: convert
  runner: MultimodalPlugin
  input:
    file: "input.png"
    task: "ocr"  # or "detect_objects", "extract_frames"
  input_modality: Image
  output_modality: Text
```

### WhisperPlugin

**Capabilities**:
- Audio transcription
- Multi-language support
- Timestamp generation

**Usage**:
```yaml
- id: transcribe
  runner: WhisperPlugin
  input:
    file: "audio.mp3"
    language: "en"
  input_modality: Audio
  output_modality: Text
```

---

## Testing

### Modality Detection Tests

Run unit tests for modality detection:
```bash
cargo test -p lao-orchestrator-core --test features_test -- test_modality
```

**Tests**:
- ✅ Audio extension detection (mp3, wav, ogg)
- ✅ Image extension detection (png, jpg, gif)
- ✅ Video extension detection (mp4, avi, mov)
- ✅ MIME type detection for all modalities

### Integration Tests

Run multimodal integration tests:
```bash
cargo test -p lao-orchestrator-core --test integration_test
```

**Tests**:
- ✅ Image analysis with loops
- ✅ Video frame extraction pipeline
- ✅ Mixed media processing
- ✅ Modality transformation chains

---

## Best Practices

### 1. Always Specify Modalities

```yaml
# ✅ Good - explicit modalities
- id: process
  runner: MultimodalPlugin
  input_modality: Image
  output_modality: Text

# ⚠️ Okay - auto-detect from extension
- id: process
  runner: MultimodalPlugin
  input:
    file: "photo.jpg"  # Auto-detected as Image
```

### 2. Use Loops for Batch Processing

```yaml
# ✅ Good - parallel processing
- id: process_images
  input:
    for_each:
      items: ["img1.jpg", "img2.png", "img3.gif"]
      max_parallel: 3
  input_modality: Image
```

### 3. Chain Transformations Logically

```yaml
# ✅ Good - clear transformation chain
Video → Image → Structured → Text

# ❌ Bad - incompatible modalities
Video → Text (skips frame extraction)
```

### 4. Handle Errors with Retries

```yaml
# ✅ Good - resilient processing
- id: process
  input_modality: Image
  retries: 3
  retry_delay: 2000
```

---

## Performance Tips

### 1. Parallel Processing

Use `max_parallel` to process multiple files concurrently:
```yaml
for_each:
  items: [file1, file2, file3, file4, file5]
  max_parallel: 3  # Process 3 at a time
```

### 2. Frame Rate Optimization

For videos, extract frames at lower FPS:
```yaml
input:
  task: "extract_frames"
  fps: 0.5  # Every 2 seconds instead of every frame
```

### 3. Cache Results

Enable caching for expensive operations:
```yaml
- id: expensive_ocr
  cache_key: "image_${filename}_ocr"
  input_modality: Image
```

---

## Troubleshooting

### Issue: "Modality mismatch"

**Cause**: Output modality doesn't match next step's input modality

**Solution**: Add conversion step
```yaml
- id: convert
  runner: MultimodalPlugin
  input:
    task: "to_text"
    data: "$previous.output"
  input_modality: Structured
  output_modality: Text
```

### Issue: "File not found"

**Cause**: File path not accessible to plugin

**Solution**: Use absolute paths or verify file location
```yaml
input:
  file: "/absolute/path/to/file.jpg"
```

### Issue: "Unknown modality"

**Cause**: File extension not recognized

**Solution**: Explicitly set `input_modality`
```yaml
input:
  file: "data.custom"
  input_modality: Binary  # Force specific modality
```

---

## API Reference

### Modality Enum

```rust
pub enum Modality {
    Audio,      // MP3, WAV, OGG, FLAC
    Image,      // PNG, JPG, GIF, BMP
    Video,      // MP4, AVI, MOV, MKV
    Text,       // TXT, MD, JSON, YAML
    Binary,     // PDF, DOCX, XLSX
    Structured, // JSON, YAML, XML data
    Unknown,    // Fallback
}
```

### Detection Methods

```rust
// From file extension
Modality::from_file_extension("photo.jpg") // → Image

// From MIME type
Modality::from_mime_type("image/png") // → Image

// String representation
Modality::Audio.as_str() // → "audio"
```

---

## Additional Resources

- **Architecture**: [docs/architecture.md](architecture.md)
- **Plugin Development**: [docs/plugins.md](plugins.md)
- **Workflow Syntax**: [docs/workflows.md](workflows.md)
- **Test Report**: [TEST_REPORT.md](../TEST_REPORT.md)
- **Integration Status**: [INTEGRATION_STATUS.md](../INTEGRATION_STATUS.md)

---

**LAO Multimodal Guide** | Version 1.0 | 2026
