# GGUFPlugin

Native GGUF model loading and inference using Hugging Face's Candle framework with full GPU acceleration support.

## Features

- **Multi-Backend Support**: CUDA (NVIDIA), Metal (Apple Silicon), and CPU
- **GGUF Format**: Direct loading of quantized GGUF models
- **Flexible Sampling**: Temperature, top-p, top-k, and repetition penalty
- **Tokenizer Support**: HuggingFace tokenizers for any model

## GPU Acceleration

### CUDA (NVIDIA GPUs)
- Automatically detected on Linux/Windows with NVIDIA drivers
- Set `device: "cuda"` in config
- Supports multiple GPUs via `cuda_device_id`

### Metal (Apple Silicon)
- Automatically detected on macOS with M1/M2/M3 chips
- Set `device: "metal"` in config
- Optimized for Apple GPU architecture
- No additional drivers needed

### CPU Fallback
- Automatically used when GPU unavailable
- Set `device: "cpu"` for explicit CPU usage

## Configuration

```json
{
  "model_path": "models/llama-2-7b-chat.Q4_K_M.gguf",
  "tokenizer_path": "meta-llama/Llama-2-7b-chat-hf",
  "device": "metal",
  "cuda_device_id": 0,
  "temperature": 0.7,
  "top_p": 0.9,
  "top_k": 40,
  "repeat_penalty": 1.1,
  "max_tokens": 512,
  "seed": 42
}
```

### Environment Variables

- `GGUF_MODEL_PATH`: Path to GGUF model file
- `GGUF_TOKENIZER_PATH`: HuggingFace model ID or local path
- `GGUF_DEVICE`: Device to use (`cuda`, `metal`, or `cpu`)
- `CUDA_VISIBLE_DEVICES`: CUDA device ID to use

## Example Workflows

### Apple Silicon (Metal)
```yaml
workflow: "Metal LLM Inference"
steps:
  - run: GGUFPlugin
    input: |
      {
        "prompt": "Explain quantum computing in simple terms",
        "config": {
          "device": "metal",
          "temperature": 0.7,
          "max_tokens": 256
        }
      }
```

### NVIDIA GPU (CUDA)
```yaml
workflow: "CUDA LLM Inference"
steps:
  - run: GGUFPlugin
    input: |
      {
        "prompt": "Write a Python function to sort a list",
        "config": {
          "device": "cuda",
          "cuda_device_id": 0,
          "temperature": 0.5,
          "max_tokens": 512
        }
      }
```

### CPU Only
```yaml
workflow: "CPU LLM Inference"
steps:
  - run: GGUFPlugin
    input: |
      {
        "prompt": "Summarize this text",
        "config": {
          "device": "cpu",
          "max_tokens": 128
        }
      }
```

## Model Compatibility

Supports all GGUF quantized models:
- Llama 2/3
- Mistral
- Mixtral
- CodeLlama
- Any model with HuggingFace tokenizer

## Performance Tips

1. **Metal on Apple Silicon**: Near-native performance with M-series chips
2. **CUDA on NVIDIA**: Best performance with RTX 30xx/40xx series
3. **Quantization**: Q4_K_M offers best quality/speed tradeoff
4. **Context Size**: Larger context = more memory but better coherence
5. **Batch Size**: Adjust based on available VRAM/RAM

## Troubleshooting

### Metal Not Available on macOS
- Ensure macOS 12.3+ (required for Metal acceleration)
- Check Activity Monitor for GPU usage
- Falls back to CPU automatically

### CUDA Errors
- Verify NVIDIA drivers installed: `nvidia-smi`
- Check CUDA version compatibility
- Falls back to CPU automatically

### Out of Memory
- Use smaller quantization (Q4_0, Q4_K_S)
- Reduce `max_tokens`
- Switch to CPU with `device: "cpu"`
