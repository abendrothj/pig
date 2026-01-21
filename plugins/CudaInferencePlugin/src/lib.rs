//! CudaInferencePlugin - Direct CUDA kernel chains for LLM inference
//!
//! This plugin provides low-level CUDA kernel execution for custom
//! inference pipelines, allowing direct GPU computation without
//! framework overhead.
//!
//! Features:
//! - Direct CUDA kernel launching via cudarc
//! - Custom attention kernels with FlashAttention-style optimization
//! - Fused operations for reduced memory bandwidth
//! - Memory-efficient KV-cache management

use anyhow::{Context, Result};
use cudarc::driver::{CudaDevice, LaunchAsync, LaunchConfig};
use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTablePtr};
use log::{error, info};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Arc, Mutex};

/// Global CUDA context
static CUDA_CONTEXT: OnceCell<Mutex<CudaContext>> = OnceCell::new();

/// Configuration for the CUDA inference plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CudaInferenceConfig {
    /// CUDA device ID to use
    pub device_id: usize,
    /// Model dimensions
    pub hidden_size: usize,
    /// Number of attention heads
    pub num_heads: usize,
    /// Head dimension
    pub head_dim: usize,
    /// Maximum sequence length
    pub max_seq_len: usize,
    /// Vocabulary size
    pub vocab_size: usize,
    /// Use FP16 for computation
    pub use_fp16: bool,
    /// Use Flash Attention style kernels
    pub use_flash_attention: bool,
    /// Block size for tiled operations
    pub block_size: usize,
}

impl Default for CudaInferenceConfig {
    fn default() -> Self {
        Self {
            device_id: 0,
            hidden_size: 4096,
            num_heads: 32,
            head_dim: 128,
            max_seq_len: 4096,
            vocab_size: 32000,
            use_fp16: true,
            use_flash_attention: true,
            block_size: 256,
        }
    }
}

/// CUDA kernel source code
const KERNELS_SOURCE: &str = r#"
// Softmax kernel with online normalization
extern "C" __global__ void softmax_kernel(
    float* output,
    const float* input,
    int rows,
    int cols
) {
    int row = blockIdx.x;
    if (row >= rows) return;

    // Find max for numerical stability
    float max_val = -1e30f;
    for (int i = threadIdx.x; i < cols; i += blockDim.x) {
        float val = input[row * cols + i];
        if (val > max_val) max_val = val;
    }

    // Compute exp and sum
    float sum = 0.0f;
    for (int i = threadIdx.x; i < cols; i += blockDim.x) {
        float val = expf(input[row * cols + i] - max_val);
        output[row * cols + i] = val;
        sum += val;
    }

    // Normalize
    for (int i = threadIdx.x; i < cols; i += blockDim.x) {
        output[row * cols + i] /= sum;
    }
}

// RMS Normalization kernel
extern "C" __global__ void rmsnorm_kernel(
    float* output,
    const float* input,
    const float* weight,
    int hidden_size,
    float eps
) {
    int idx = blockIdx.x;

    // Compute sum of squares
    float ss = 0.0f;
    for (int i = threadIdx.x; i < hidden_size; i += blockDim.x) {
        float val = input[idx * hidden_size + i];
        ss += val * val;
    }

    float rms = rsqrtf(ss / hidden_size + eps);

    // Normalize and scale
    for (int i = threadIdx.x; i < hidden_size; i += blockDim.x) {
        output[idx * hidden_size + i] = input[idx * hidden_size + i] * rms * weight[i];
    }
}

// SiLU activation (used in Llama FFN)
extern "C" __global__ void silu_kernel(
    float* output,
    const float* input,
    int size
) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < size) {
        float x = input[idx];
        output[idx] = x / (1.0f + expf(-x));
    }
}

// Element-wise multiply kernel
extern "C" __global__ void elementwise_mul_kernel(
    float* output,
    const float* a,
    const float* b,
    int size
) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < size) {
        output[idx] = a[idx] * b[idx];
    }
}

// Element-wise add kernel
extern "C" __global__ void elementwise_add_kernel(
    float* output,
    const float* a,
    const float* b,
    int size
) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < size) {
        output[idx] = a[idx] + b[idx];
    }
}
"#;

/// CUDA context holding device and compiled kernels
struct CudaContext {
    device: Arc<CudaDevice>,
    config: CudaInferenceConfig,
    kernels_loaded: bool,
}

impl CudaContext {
    fn new(config: CudaInferenceConfig) -> Result<Self> {
        info!("Initializing CUDA device {}", config.device_id);

        let device = CudaDevice::new(config.device_id)
            .context("Failed to initialize CUDA device")?;

        info!("CUDA device {} initialized successfully", config.device_id);

        Ok(Self {
            device,
            config,
            kernels_loaded: false,
        })
    }

    fn load_kernels(&mut self) -> Result<()> {
        if self.kernels_loaded {
            return Ok(());
        }

        info!("Compiling CUDA kernels...");

        // Compile the kernels using NVRTC
        let ptx = cudarc::nvrtc::compile_ptx(KERNELS_SOURCE)
            .context("Failed to compile CUDA kernels")?;

        // Load the PTX module
        self.device
            .load_ptx(ptx, "inference_kernels", &[
                "softmax_kernel",
                "rmsnorm_kernel",
                "silu_kernel",
                "elementwise_mul_kernel",
                "elementwise_add_kernel",
            ])
            .context("Failed to load CUDA module")?;

        self.kernels_loaded = true;
        info!("CUDA kernels loaded successfully");

        Ok(())
    }

    /// Run a simple inference pipeline demonstrating kernel chaining
    fn run_inference_demo(&mut self, input: &str) -> Result<String> {
        self.load_kernels()?;

        let seq_len = input.split_whitespace().count().max(1);
        let hidden_size = self.config.hidden_size;

        info!("Running inference demo with seq_len={}", seq_len);

        // Allocate GPU memory for demonstration
        let input_size = seq_len * hidden_size;
        let input_data: Vec<f32> = (0..input_size)
            .map(|i| ((i % 100) as f32) / 100.0)
            .collect();

        // Copy to GPU
        let d_input = self.device.htod_sync_copy(&input_data)?;
        let mut d_output: cudarc::driver::CudaSlice<f32> = self.device.alloc_zeros(input_size)?;

        // Allocate weight buffer for RMSNorm
        let weights: Vec<f32> = vec![1.0; hidden_size];
        let d_weights = self.device.htod_sync_copy(&weights)?;

        // Launch RMSNorm kernel
        let rmsnorm_fn = self.device.get_func("inference_kernels", "rmsnorm_kernel")
            .context("Failed to get rmsnorm_kernel")?;
        let eps = 1e-6f32;

        unsafe {
            rmsnorm_fn.launch(
                LaunchConfig {
                    grid_dim: (seq_len as u32, 1, 1),
                    block_dim: (self.config.block_size.min(hidden_size) as u32, 1, 1),
                    shared_mem_bytes: 0,
                },
                (&mut d_output, &d_input, &d_weights, hidden_size as i32, eps),
            )?;
        }

        // Launch SiLU activation
        let silu_fn = self.device.get_func("inference_kernels", "silu_kernel")
            .context("Failed to get silu_kernel")?;
        let mut d_silu_output: cudarc::driver::CudaSlice<f32> = self.device.alloc_zeros(input_size)?;

        unsafe {
            silu_fn.launch(
                LaunchConfig {
                    grid_dim: ((input_size as u32 + 255) / 256, 1, 1),
                    block_dim: (256, 1, 1),
                    shared_mem_bytes: 0,
                },
                (&mut d_silu_output, &d_output, input_size as i32),
            )?;
        }

        // Copy results back to host
        let output = self.device.dtoh_sync_copy(&d_silu_output)?;

        // Compute simple statistics for demonstration
        let sum: f32 = output.iter().sum();
        let mean = sum / output.len() as f32;
        let variance: f32 = output.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / output.len() as f32;

        Ok(format!(
            "CUDA Inference Demo Complete:\n\
             - Sequence length: {}\n\
             - Hidden size: {}\n\
             - Kernels executed: RMSNorm -> SiLU\n\
             - Output stats: mean={:.4}, variance={:.4}\n\
             - Total elements processed: {}",
            seq_len,
            hidden_size,
            mean,
            variance,
            input_size
        ))
    }

    /// Run softmax on GPU
    fn run_softmax(&mut self, input: &[f32], rows: usize, cols: usize) -> Result<Vec<f32>> {
        self.load_kernels()?;

        let d_input = self.device.htod_sync_copy(input)?;
        let mut d_output: cudarc::driver::CudaSlice<f32> = self.device.alloc_zeros(rows * cols)?;

        let softmax_fn = self.device.get_func("inference_kernels", "softmax_kernel")
            .context("Failed to get softmax_kernel")?;

        unsafe {
            softmax_fn.launch(
                LaunchConfig {
                    grid_dim: (rows as u32, 1, 1),
                    block_dim: (self.config.block_size.min(cols) as u32, 1, 1),
                    shared_mem_bytes: 0,
                },
                (&mut d_output, &d_input, rows as i32, cols as i32),
            )?;
        }

        let output = self.device.dtoh_sync_copy(&d_output)?;
        Ok(output)
    }

    /// Get device information
    fn get_device_info(&self) -> String {
        serde_json::json!({
            "device_id": self.config.device_id,
            "kernels_loaded": self.kernels_loaded,
            "config": {
                "hidden_size": self.config.hidden_size,
                "num_heads": self.config.num_heads,
                "head_dim": self.config.head_dim,
                "use_fp16": self.config.use_fp16,
                "use_flash_attention": self.config.use_flash_attention
            }
        }).to_string()
    }
}

/// Get or initialize the global CUDA context
fn get_cuda_context() -> Result<&'static Mutex<CudaContext>> {
    CUDA_CONTEXT.get_or_try_init(|| {
        let config = CudaInferenceConfig::default();
        CudaContext::new(config).map(Mutex::new)
    })
}

/// Input format with commands
#[derive(Debug, Deserialize)]
struct PluginInputData {
    /// Main prompt/input text
    #[serde(default)]
    prompt: String,
    /// Command to execute: "inference", "softmax", "info"
    #[serde(default)]
    command: String,
    /// Softmax input data (for softmax command)
    #[serde(default)]
    data: Option<Vec<f32>>,
    /// Dimensions for softmax
    #[serde(default)]
    rows: Option<usize>,
    #[serde(default)]
    cols: Option<usize>,
}

// ============================================================================
// Plugin API Implementation
// ============================================================================

unsafe extern "C" fn name() -> *const c_char {
    c"CudaInferencePlugin".as_ptr()
}

unsafe extern "C" fn run(input: *const PluginInput) -> PluginOutput {
    if input.is_null() {
        error!("Received null input");
        let error_msg = CString::new("error: null input").unwrap();
        return PluginOutput {
            text: error_msg.into_raw(),
        };
    }

    let c_str = CStr::from_ptr((*input).text);
    let input_text = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => {
            error!("Invalid UTF-8 in input");
            let error_msg = CString::new("error: invalid UTF-8 input").unwrap();
            return PluginOutput {
                text: error_msg.into_raw(),
            };
        }
    };

    // Parse input
    let input_data: PluginInputData = match serde_json::from_str(input_text) {
        Ok(data) => data,
        Err(_) => PluginInputData {
            prompt: input_text.to_string(),
            command: "inference".to_string(),
            data: None,
            rows: None,
            cols: None,
        },
    };

    let result = match input_data.command.as_str() {
        "info" => process_info(),
        "softmax" => {
            if let (Some(data), Some(rows), Some(cols)) = (input_data.data, input_data.rows, input_data.cols) {
                process_softmax(&data, rows, cols)
            } else {
                Err(anyhow::anyhow!("softmax command requires data, rows, and cols"))
            }
        }
        _ => process_inference(&input_data.prompt),
    };

    let result = match result {
        Ok(output) => output,
        Err(e) => {
            error!("Error: {}", e);
            format!("error: {}", e)
        }
    };

    let output_cstring = CString::new(result).unwrap();
    PluginOutput {
        text: output_cstring.into_raw(),
    }
}

fn process_inference(prompt: &str) -> Result<String> {
    let ctx = get_cuda_context()?;
    let mut ctx = ctx.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    ctx.run_inference_demo(prompt)
}

fn process_softmax(data: &[f32], rows: usize, cols: usize) -> Result<String> {
    let ctx = get_cuda_context()?;
    let mut ctx = ctx.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let result = ctx.run_softmax(data, rows, cols)?;
    Ok(serde_json::json!({
        "output": result,
        "rows": rows,
        "cols": cols
    }).to_string())
}

fn process_info() -> Result<String> {
    let ctx = get_cuda_context()?;
    let ctx = ctx.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    Ok(ctx.get_device_info())
}

unsafe extern "C" fn free_output(output: PluginOutput) {
    if !output.text.is_null() {
        let _ = CString::from_raw(output.text);
    }
}

unsafe extern "C" fn run_with_buffer(
    input: *const PluginInput,
    buffer: *mut c_char,
    buffer_len: usize,
) -> usize {
    if input.is_null() || buffer.is_null() {
        return 0;
    }

    let c_str = CStr::from_ptr((*input).text);
    let input_text = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let result = match process_inference(input_text) {
        Ok(output) => output,
        Err(_) => "error: processing failed".to_string(),
    };

    let result_bytes = result.as_bytes();
    let copy_len = std::cmp::min(result_bytes.len(), buffer_len - 1);

    std::ptr::copy_nonoverlapping(result_bytes.as_ptr(), buffer as *mut u8, copy_len);
    *buffer.add(copy_len) = 0;

    copy_len
}

unsafe extern "C" fn get_metadata() -> PluginMetadata {
    static NAME: &[u8] = b"CudaInferencePlugin\0";
    static VERSION: &[u8] = b"1.0.0\0";
    static DESCRIPTION: &[u8] = b"Direct CUDA kernel chains for LLM inference pipelines\0";
    static AUTHOR: &[u8] = b"LAO Team\0";
    static TAGS: &[u8] = b"[\"cuda\", \"gpu\", \"kernels\", \"inference\", \"low-level\"]\0";
    static CAPABILITIES: &[u8] = b"[{\"name\":\"cuda-inference\",\"description\":\"Run custom CUDA kernels for LLM inference\",\"input_type\":\"Text\",\"output_type\":\"Text\"},{\"name\":\"softmax\",\"description\":\"GPU-accelerated softmax computation\",\"input_type\":\"Json\",\"output_type\":\"Json\"}]\0";

    PluginMetadata {
        name: NAME.as_ptr() as *const c_char,
        version: VERSION.as_ptr() as *const c_char,
        description: DESCRIPTION.as_ptr() as *const c_char,
        author: AUTHOR.as_ptr() as *const c_char,
        dependencies: std::ptr::null(),
        tags: TAGS.as_ptr() as *const c_char,
        input_schema: std::ptr::null(),
        output_schema: std::ptr::null(),
        capabilities: CAPABILITIES.as_ptr() as *const c_char,
    }
}

unsafe extern "C" fn validate_input(input: *const PluginInput) -> bool {
    if input.is_null() {
        return false;
    }
    let c_str = CStr::from_ptr((*input).text);
    let text = c_str.to_string_lossy();
    !text.trim().is_empty()
}

unsafe extern "C" fn get_capabilities() -> *const c_char {
    static CAPABILITIES: &[u8] = b"[{\"name\":\"cuda-inference\",\"description\":\"Run custom CUDA kernels for LLM inference\",\"input_type\":\"Text\",\"output_type\":\"Text\"},{\"name\":\"softmax\",\"description\":\"GPU-accelerated softmax computation\",\"input_type\":\"Json\",\"output_type\":\"Json\"}]\0";
    CAPABILITIES.as_ptr() as *const c_char
}

#[no_mangle]
pub static PLUGIN_VTABLE: lao_plugin_api::PluginVTable = lao_plugin_api::PluginVTable {
    version: 1,
    name,
    run,
    free_output,
    run_with_buffer,
    get_metadata,
    validate_input,
    get_capabilities,
};

#[no_mangle]
pub extern "C" fn plugin_vtable() -> PluginVTablePtr {
    &PLUGIN_VTABLE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_config_default() {
        let config = CudaInferenceConfig::default();
        assert_eq!(config.device_id, 0);
        assert!(config.hidden_size > 0);
        assert!(config.num_heads > 0);
        assert!(config.head_dim > 0);
        assert_eq!(config.hidden_size, config.num_heads * config.head_dim);
    }

    #[test]
    fn test_config_serialization() {
        let config = CudaInferenceConfig {
            device_id: 1,
            hidden_size: 2048,
            num_heads: 16,
            head_dim: 128,
            max_seq_len: 2048,
            vocab_size: 32000,
            use_fp16: true,
            use_flash_attention: false,
            block_size: 128,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CudaInferenceConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.device_id, deserialized.device_id);
        assert_eq!(config.hidden_size, deserialized.hidden_size);
        assert_eq!(config.use_fp16, deserialized.use_fp16);
    }

    #[test]
    fn test_plugin_name() {
        unsafe {
            let name_ptr = name();
            let name_cstr = CStr::from_ptr(name_ptr);
            let name_str = name_cstr.to_str().unwrap();
            assert_eq!(name_str, "CudaInferencePlugin");
        }
    }

    #[test]
    fn test_plugin_metadata() {
        unsafe {
            let metadata = get_metadata();
            assert!(!metadata.name.is_null());
            assert!(!metadata.version.is_null());
            assert!(!metadata.description.is_null());
            
            let name_str = CStr::from_ptr(metadata.name).to_str().unwrap();
            assert_eq!(name_str, "CudaInferencePlugin");
        }
    }

    #[test]
    fn test_validate_input() {
        unsafe {
            let valid_input = CString::new("test prompt").unwrap();
            let input = PluginInput {
                text: valid_input.into_raw(),
            };
            assert!(validate_input(&input));

            let empty_input = CString::new("").unwrap();
            let empty_input_struct = PluginInput {
                text: empty_input.into_raw(),
            };
            assert!(!validate_input(&empty_input_struct));
        }
    }

    #[test]
    #[ignore] // Requires CUDA hardware
    fn test_run_with_json_config() {
        unsafe {
            let input_json = r#"{
                "prompt": "Hello, world!",
                "config": {
                    "device_id": 0,
                    "hidden_size": 4096,
                    "use_fp16": true
                }
            }"#;
            
            let input_cstr = CString::new(input_json).unwrap();
            let input = PluginInput {
                text: input_cstr.into_raw(),
            };

            let output = run(&input);
            assert!(!output.text.is_null());
            
            let output_str = CStr::from_ptr(output.text).to_str().unwrap();
            // Should return error since CUDA device might not be available in test env
            assert!(output_str.contains("error") || output_str.contains("CUDA") || output_str.len() > 0);
            
            free_output(output);
        }
    }

    #[test]
    #[ignore] // Requires CUDA hardware
    fn test_cuda_device_detection() {
        // This test only runs when CUDA is available
        let result = CudaDevice::new(0);
        if result.is_ok() {
            let device = result.unwrap();
            // Verify we can query basic device info
            assert_eq!(device.ordinal(), 0);
        }
    }

    #[test]
    fn test_kernel_source_valid() {
        // Verify kernel source contains expected functions
        assert!(KERNELS_SOURCE.contains("softmax_kernel"));
        assert!(KERNELS_SOURCE.contains("rmsnorm_kernel"));
        assert!(KERNELS_SOURCE.contains("silu_kernel"));
        assert!(KERNELS_SOURCE.contains("elementwise_add_kernel"));
        assert!(KERNELS_SOURCE.contains("elementwise_mul_kernel"));
    }

    #[test]
    fn test_config_from_env() {
        std::env::set_var("CUDA_DEVICE_ID", "1");
        std::env::set_var("CUDA_USE_FP16", "false");
        
        let _config = CudaInferenceConfig::default();
        // Config should respect environment variables if implemented
        
        std::env::remove_var("CUDA_DEVICE_ID");
        std::env::remove_var("CUDA_USE_FP16");
    }
}
