# Mac Silicon Optimization Guide

This guide covers optimizations specific to Apple Silicon (M1/M2/M3/M4) Macs.

## Architecture-Specific Features

### 1. Unified Memory Architecture
Apple Silicon uses unified memory shared between CPU and GPU:
- **No data copying** between CPU and GPU
- **Lower latency** for model loading
- **More available VRAM** (uses system RAM)

**Optimization**: Load larger models than traditional GPUs allow
```yaml
# Example: 13B model on M1 Max with 32GB RAM
config:
  model_path: "llama-2-13b.gguf"
  device: "metal"
  n_gpu_layers: 999  # Full GPU offload
```

### 2. Metal Performance Shaders (MPS)
Native GPU acceleration for ML workloads:
- **Automatic optimization** by Apple's compiler
- **Power efficient** compared to discrete GPUs
- **Thermal management** built-in

**Status**: ✅ Enabled in GGUFPlugin and LlamaCppPlugin

### 3. Accelerate Framework
Apple's optimized BLAS/LAPACK implementation:
- **5-10x faster** matrix operations vs generic BLAS
- **NEON SIMD** instructions for ARM64
- **Cache optimized** for Apple CPU architecture

**Enable**: Build with `accelerate` feature
```bash
cd plugins/GGUFPlugin
cargo build --release --features metal,accelerate
```

### 4. High Performance vs Efficiency Cores
M-series chips have performance (P) and efficiency (E) cores:
- **M1**: 4P + 4E cores
- **M2**: 4P + 4E cores
- **M3 Max**: 12P + 4E cores
- **M4 Max**: 14P + 10E cores

**Optimization**: Let macOS scheduler handle thread placement
```rust
// Set threads to total core count
config.n_threads = std::thread::available_parallelism()
    .map(|n| n.get() as u32)
    .unwrap_or(8);
```

## Recommended Configurations

### M1/M2 (8GB RAM)
```yaml
config:
  device: "metal"
  n_gpu_layers: 35  # Partial offload
  max_tokens: 256
  # Use 7B Q4_K_M models
```

### M1/M2 Pro/Max (16-32GB RAM)
```yaml
config:
  device: "metal"
  n_gpu_layers: 999  # Full offload
  max_tokens: 512
  # Can run 13B Q4_K_M models
```

### M3 Max/Ultra (64-128GB RAM)
```yaml
config:
  device: "metal"
  n_gpu_layers: 999
  max_tokens: 2048
  # Can run 30B+ models or higher quantization
```

## Performance Benchmarks

### Token Generation Speed (tokens/sec)

| Model | M1 (Metal) | M2 (Metal) | M3 Max | RTX 4090 (CUDA) |
|-------|-----------|-----------|---------|-----------------|
| Llama-2-7B Q4 | 25-35 | 35-45 | 50-65 | 120-150 |
| Llama-2-13B Q4 | 12-18 | 18-25 | 30-40 | 80-100 |
| Llama-2-70B Q4 | N/A | N/A | 8-12 | 40-50 |

## Battery Optimization

### For MacBooks
Metal is significantly more power-efficient than CPU:

```yaml
# Power-saving config
config:
  device: "metal"
  n_gpu_layers: 20  # Partial offload reduces power
  max_tokens: 128
  temperature: 0.7
```

**Tips**:
- Use Metal for 2-4x better battery life vs CPU
- Lower `n_gpu_layers` if laptop gets hot
- Reduce `max_tokens` for faster completion
- Use Q4_0 quantization (smaller, faster)

## Build Optimizations

### Native ARM64 Build
Ensure you're building natively (not via Rosetta):
```bash
rustc --version  # Should show aarch64-apple-darwin
file target/release/lao-cli  # Should show arm64
```

### Optimized Release Build
```bash
# Full optimization
RUSTFLAGS="-C target-cpu=native" cargo build --release

# With link-time optimization
RUSTFLAGS="-C target-cpu=native -C lto=thin" cargo build --release
```

### Enable All Apple Features
```bash
cd plugins/GGUFPlugin
cargo build --release --features metal,accelerate

cd ../LlamaCppPlugin
# llama.cpp auto-detects Metal
cargo build --release
```

## Monitoring Performance

### Check Metal Usage
```bash
# Monitor GPU utilization
sudo powermetrics --samplers gpu_power -i 1000

# Or use Activity Monitor > Window > GPU History
```

### Check Memory Pressure
```bash
# Unified memory usage
memory_pressure
```

### Profile with Instruments
```bash
# Xcode's profiler works with Rust
xcrun xctrace record --template "Time Profiler" \
  --launch ./target/release/lao-cli
```

## Troubleshooting

### Metal Not Detected
```bash
# Check Metal support
system_profiler SPDisplaysDataType | grep Metal

# Force Metal backend
export GGUF_DEVICE=metal
```

### Poor Performance
1. **Verify native build**: `file target/release/lao-cli` → should show arm64
2. **Check thermal throttling**: Activity Monitor → CPU History
3. **Disable Rosetta**: System Settings → General → Rosetta (should be off)
4. **Free memory**: Close other apps, Metal uses system RAM

### Model Too Large
```yaml
# Hybrid CPU/GPU approach
config:
  device: "metal"
  n_gpu_layers: 20  # Reduce layers
  n_threads: 8      # Use CPU cores for remaining layers
```

## Future Optimizations

### Planned Features
- [ ] CoreML integration for Neural Engine support
- [ ] Apple Silicon-specific quantization formats
- [ ] Unified Memory zero-copy optimizations
- [ ] Metal Performance Shaders custom kernels
- [ ] Multi-model concurrent execution
- [ ] Memory compression for larger models

### Neural Engine (ANE) Support
Apple's dedicated ML accelerator (16 TOPS on M1):
- Currently not exposed in stable APIs
- CoreML can use ANE for certain operations
- Future plugin planned: `CoreMLPlugin`

## Best Practices

1. **Always use Metal** on Apple Silicon (2-5x faster than CPU)
2. **Enable Accelerate** for CPU fallback operations
3. **Match model size to RAM** (7B for 8GB, 13B for 16GB+)
4. **Use Q4_K_M quantization** (best quality/speed balance)
5. **Full GPU offload** (`n_gpu_layers: 999`) when possible
6. **Monitor temperature** on MacBooks, reduce layers if hot
7. **Close other apps** before large model loads
8. **Use power adapter** for intensive workloads

## Comparison: Metal vs Other Backends

### Metal Advantages
- ✅ Native Apple Silicon integration
- ✅ Power efficient (2-4x better battery)
- ✅ Unified memory (no CPU↔GPU copies)
- ✅ Thermal management built-in
- ✅ Works out-of-box, no drivers needed

### Metal Limitations
- ❌ Slower than high-end NVIDIA GPUs (RTX 4090)
- ❌ Limited to macOS
- ❌ Memory shared with system (not dedicated VRAM)

### When to Use Each
- **Metal**: All Apple Silicon Macs (best choice)
- **CPU**: Only for testing or very small models
- **CUDA**: Linux/Windows with NVIDIA GPUs

## Resources

- [Apple Metal Documentation](https://developer.apple.com/metal/)
- [Accelerate Framework](https://developer.apple.com/documentation/accelerate)
- [llama.cpp Metal Backend](https://github.com/ggerganov/llama.cpp#metal-build)
- [Candle Metal Support](https://github.com/huggingface/candle/tree/main/candle-metal-kernels)
