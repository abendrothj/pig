# LlamaCppPlugin

Direct llama.cpp inference with automatic CUDA/Metal/CPU acceleration detection.

## Features

- **Automatic GPU Detection**: Uses CUDA on NVIDIA GPUs, Metal on Apple Silicon
- **Full GGUF Support**: Load any GGUF quantized model
- **Flexible GPU Offloading**: Control how many layers run on GPU
- **Battle-Tested**: Based on production-grade llama.cpp
- **Ollama Compatible**: Works with Ollama-downloaded models

## GPU Acceleration

### Automatic Backend Selection

llama.cpp automatically detects and uses the best available backend:

1. **Metal** (Apple Silicon - M1/M2/M3/M4)
   - Automatically detected on macOS
   - Set `n_gpu_layers > 0` to enable
   - No configuration needed

2. **CUDA** (NVIDIA GPUs)
   - Automatically detected on Linux/Windows
   - Set `n_gpu_layers > 0` to enable
   - Requires NVIDIA drivers

3. **CPU** (Fallback)
   - Used when `n_gpu_layers = 0`
   - Or when no GPU detected

### GPU Layer Configuration

- `n_gpu_layers: 0` - CPU only
- `n_gpu_layers: 20` - Offload 20 layers to GPU
- `n_gpu_layers: 35` - Offload 35 layers (default)
- `n_gpu_layers: 999` - Offload all layers (recommended)

Higher values = more GPU usage = faster inference

## Configuration

```json
{
  "model_path": "models/llama-2-7b-chat.Q4_K_M.gguf",
  "n_gpu_layers": 999,
  "n_ctx": 4096,
  "n_threads": 8,
  "n_batch": 512,
  "temperature": 0.7,
  "top_p": 0.9,
  "top_k": 40,
  "repeat_penalty": 1.1,
  "max_tokens": 512
}
```

### Environment Variables

- `LLAMA_MODEL_PATH`: Path to GGUF model file
- `LLAMA_GPU_LAYERS`: Number of layers to offload (default: 35)

## Example Workflows

### Apple Silicon (Metal)
```yaml
workflow: "Metal-Accelerated Generation"
steps:
  - run: LlamaCppPlugin
    input: |
      {
        "prompt": "Write a haiku about coding",
        "config": {
          "n_gpu_layers": 999,
          "temperature": 0.8
        }
      }
```

### NVIDIA GPU (CUDA)
```yaml
workflow: "CUDA-Accelerated Generation"
steps:
  - run: LlamaCppPlugin
    input: |
      {
        "prompt": "Explain machine learning",
        "config": {
          "n_gpu_layers": 999,
          "n_ctx": 8192,
          "temperature": 0.7
        }
      }
```

### CPU Only
```yaml
workflow: "CPU Generation"
steps:
  - run: LlamaCppPlugin
    input: |
      {
        "prompt": "Short answer: what is AI?",
        "config": {
          "n_gpu_layers": 0,
          "n_threads": 16,
          "max_tokens": 100
        }
      }
```

## Using Ollama Models

This plugin works with models downloaded via Ollama:

### Find Ollama Models

**macOS/Linux:**
```bash
ls ~/.ollama/models/blobs/
```

**Windows:**
```powershell
dir %USERPROFILE%\.ollama\models\blobs\
```

### Use in Workflow

```yaml
workflow: "Ollama Model Usage"
steps:
  - run: LlamaCppPlugin
    input: |
      {
        "prompt": "Hello, world!",
        "config": {
          "model_path": "~/.ollama/models/blobs/sha256-abc123...",
          "n_gpu_layers": 999
        }
      }
```

## Performance Tips

### Apple Silicon (Metal)
- **M3 Max/Ultra**: Use `n_gpu_layers: 999` for full offload
- **M1/M2**: Start with `n_gpu_layers: 35`, increase if memory permits
- **Memory**: Unified memory shared between CPU/GPU
- **Models**: 7B models run well, 13B+ requires 32GB+ RAM

### NVIDIA GPU (CUDA)
- **RTX 4090/4080**: Full offload for up to 70B models
- **RTX 3090/3080**: 30B models with full offload
- **RTX 3060 (12GB)**: 13B models work well
- **Older GPUs**: Reduce `n_gpu_layers` or use CPU

### CPU Optimization
- Set `n_threads` to CPU core count
- Use quantized models (Q4_K_M recommended)
- Increase `n_batch` for throughput over latency

## Quantization Formats

- `Q4_0` - Smallest, fastest, lowest quality
- `Q4_K_M` - Best quality/size tradeoff (recommended)
- `Q5_K_M` - Higher quality, larger size
- `Q8_0` - Very high quality, nearly FP16 size

## Troubleshooting

### Metal Not Using GPU on macOS
```bash
# Check GPU is available
system_profiler SPDisplaysDataType | grep Metal

# Set explicit GPU layers
export LLAMA_GPU_LAYERS=999
```

### CUDA Not Detected
```bash
# Verify CUDA installation
nvidia-smi

# Check driver version
cat /proc/driver/nvidia/version
```

### Out of Memory
1. Reduce `n_gpu_layers`
2. Use smaller quantization (Q4_0 instead of Q4_K_M)
3. Reduce `n_ctx` (context size)
4. Use smaller model

### Slow Performance
- Increase `n_gpu_layers` if GPU available
- Use Metal on Apple Silicon (automatic)
- Check `n_threads` matches CPU cores
- Verify GPU is actually being used (Activity Monitor/nvidia-smi)
