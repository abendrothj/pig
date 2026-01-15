# WhisperPlugin

A plugin that transcribes audio files to text using a local Whisper model via whisper.cpp.

## Prerequisites

You need to have `whisper.cpp` installed on your system. The plugin will look for it in:
1. Environment variable `WHISPER_CPP_PATH` (highest priority)
2. Current directory: `./whisper.cpp` or `./whisper-cpp`
3. System PATH: `whisper.cpp` or `whisper-cpp`
4. Common system locations: `/usr/local/bin/whisper.cpp`, `/usr/bin/whisper.cpp`, `~/.local/bin/whisper.cpp`

### Installing whisper.cpp

**macOS:**
```bash
brew install whisper.cpp
# Or build from source: https://github.com/ggerganov/whisper.cpp
```

**Linux:**
```bash
# Build from source
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
make
# The binary will be in the current directory
```

**Set custom path:**
```bash
export WHISPER_CPP_PATH=/path/to/whisper.cpp
```

## Input
- `input` (string): Path to the audio file to transcribe.

## Output
- (string): The transcribed text.

## Example Workflow
```yaml
workflow: "Audio Transcription"
steps:
  - run: WhisperPlugin
    input: "meeting.wav"
  - run: SummarizerPlugin
    input_from: step1
```

## Usage
Reference the plugin by name in your workflow YAML as shown above. The output can be used as input for downstream plugins (e.g., SummarizerPlugin).

## Troubleshooting

If you see "whisper.cpp binary not found":
1. Install whisper.cpp (see Prerequisites above)
2. Ensure it's in your PATH, or set `WHISPER_CPP_PATH` environment variable
3. Verify the binary is executable: `chmod +x /path/to/whisper.cpp` 