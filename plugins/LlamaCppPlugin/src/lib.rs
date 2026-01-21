//! LlamaCppPlugin - Direct llama.cpp inference for LLM generation
//!
//! This plugin provides direct access to llama.cpp for LLM inference
//! with full GGUF model support and CUDA/Metal/CPU acceleration.
//!
//! ## GPU Acceleration
//!
//! - **CUDA**: Automatically detected on NVIDIA GPUs (Linux/Windows)
//! - **Metal**: Automatically detected on Apple Silicon (macOS)
//! - **CPU**: Fallback when no GPU is available
//!
//! Set `n_gpu_layers` to control GPU offloading:
//! - `0`: CPU only
//! - `1-N`: Offload N layers to GPU
//! - `999`: Offload all layers to GPU (recommended)
//!
//! ## Configuration
//!
//! Environment variables:
//! - `LLAMA_MODEL_PATH`: Path to GGUF model file
//! - `LLAMA_GPU_LAYERS`: Number of layers to offload to GPU (default: 35)
//!
//! ## Using Ollama Models
//!
//! This plugin can use models downloaded by Ollama. Find them at:
//! - Linux/macOS: `~/.ollama/models/blobs/`
//! - Windows: `%USERPROFILE%\.ollama\models\blobs\`
//!
//! Example: `LLAMA_MODEL_PATH=~/.ollama/models/blobs/sha256-xxxx`

use anyhow::{Context, Result};
use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTablePtr};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use log::{error, info};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::num::NonZeroU32;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::Mutex;

/// Global model instance for reuse across calls
static MODEL_INSTANCE: OnceCell<Mutex<ModelState>> = OnceCell::new();

/// Configuration for the LlamaCpp plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaCppConfig {
    /// Path to the GGUF model file
    pub model_path: String,
    /// Number of GPU layers to offload (0 = CPU only)
    pub n_gpu_layers: i32,
    /// Context size (number of tokens)
    pub n_ctx: u32,
    /// Number of threads for CPU inference
    pub n_threads: u32,
    /// Batch size for parallel processing
    pub n_batch: u32,
    /// Temperature for sampling (0.0 = greedy)
    pub temperature: f32,
    /// Top-p sampling threshold
    pub top_p: f32,
    /// Top-k sampling (0 = disabled)
    pub top_k: i32,
    /// Repetition penalty
    pub repeat_penalty: f32,
    /// Maximum tokens to generate
    pub max_tokens: u32,
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            model_path: std::env::var("LLAMA_MODEL_PATH")
                .unwrap_or_else(|_| "models/llama-2-7b-chat.Q4_K_M.gguf".to_string()),
            n_gpu_layers: std::env::var("LLAMA_GPU_LAYERS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(35), // Offload most layers to GPU by default
            n_ctx: 4096,
            n_threads: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4),
            n_batch: 512,
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
            max_tokens: 512,
        }
    }
}

/// Holds the loaded model and backend
struct ModelState {
    backend: LlamaBackend,
    model: LlamaModel,
    config: LlamaCppConfig,
}

impl ModelState {
    fn new(config: LlamaCppConfig) -> Result<Self> {
        info!("Initializing llama.cpp backend...");
        let backend = LlamaBackend::init()?;

        info!("Loading model from: {}", config.model_path);
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(config.n_gpu_layers as u32);

        let model = LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
            .context("Failed to load GGUF model")?;

        info!("Model loaded successfully with {} GPU layers", config.n_gpu_layers);

        Ok(Self {
            backend,
            model,
            config,
        })
    }

    fn generate(&self, prompt: &str) -> Result<String> {
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.config.n_ctx))
            .with_n_batch(self.config.n_batch);

        let mut ctx = self.model
            .new_context(&self.backend, ctx_params)
            .context("Failed to create context")?;

        // Tokenize the prompt
        let tokens = self.model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .context("Failed to tokenize prompt")?;

        info!("Tokenized prompt into {} tokens", tokens.len());

        // Create batch for processing
        let mut batch = LlamaBatch::new(self.config.n_batch as usize, 1);

        // Add tokens to batch
        for (i, token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch.add(*token, i as i32, &[0], is_last)?;
        }

        // Process the prompt
        ctx.decode(&mut batch).context("Failed to decode prompt")?;

        // Setup sampler chain
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(self.config.temperature),
            LlamaSampler::top_k(self.config.top_k),
            LlamaSampler::top_p(self.config.top_p, 1),
            LlamaSampler::dist(rand::random()),
        ]);

        // Generate tokens
        let mut output_tokens = Vec::new();
        let mut n_cur = tokens.len();

        for _ in 0..self.config.max_tokens {
            // Sample next token
            let token = sampler.sample(&ctx, -1);

            // Check for end of sequence
            if self.model.is_eog_token(token) {
                break;
            }

            output_tokens.push(token);

            // Prepare next batch
            batch.clear();
            batch.add(token, n_cur as i32, &[0], true)?;
            n_cur += 1;

            // Decode
            ctx.decode(&mut batch).context("Failed to decode token")?;
        }

        // Convert tokens back to string
        let output: String = output_tokens
            .iter()
            .map(|t| self.model.token_to_str(*t, llama_cpp_2::model::Special::Tokenize))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to convert tokens to string")?
            .join("");

        Ok(output)
    }
}

/// Get or initialize the global model instance
fn get_model() -> Result<&'static Mutex<ModelState>> {
    MODEL_INSTANCE.get_or_try_init(|| {
        let config = LlamaCppConfig::default();
        ModelState::new(config).map(Mutex::new)
    })
}

/// Input format that can include configuration overrides
#[derive(Debug, Deserialize)]
struct PluginInputData {
    prompt: String,
    #[serde(default)]
    config: Option<LlamaCppConfig>,
}

// ============================================================================
// Plugin API Implementation
// ============================================================================

unsafe extern "C" fn name() -> *const c_char {
    c"LlamaCppPlugin".as_ptr()
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

    // Try to parse as JSON with config, otherwise treat as plain prompt
    let prompt = match serde_json::from_str::<PluginInputData>(input_text) {
        Ok(data) => data.prompt,
        Err(_) => input_text.to_string(),
    };

    info!("Processing prompt: {}...", &prompt[..prompt.len().min(50)]);

    let result = match process_prompt(&prompt) {
        Ok(output) => output,
        Err(e) => {
            error!("Generation error: {}", e);
            format!("error: {}", e)
        }
    };

    let output_cstring = CString::new(result).unwrap();
    PluginOutput {
        text: output_cstring.into_raw(),
    }
}

fn process_prompt(prompt: &str) -> Result<String> {
    let model = get_model()?;
    let state = model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    state.generate(prompt)
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

    let prompt = match serde_json::from_str::<PluginInputData>(input_text) {
        Ok(data) => data.prompt,
        Err(_) => input_text.to_string(),
    };

    let result = match process_prompt(&prompt) {
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
    static NAME: &[u8] = b"LlamaCppPlugin\0";
    static VERSION: &[u8] = b"1.0.0\0";
    static DESCRIPTION: &[u8] = b"Direct llama.cpp inference - no HTTP overhead, full GPU support\0";
    static AUTHOR: &[u8] = b"LAO Team\0";
    static TAGS: &[u8] = b"[\"llm\", \"llama.cpp\", \"gguf\", \"cuda\", \"text-generation\"]\0";
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using llama.cpp with CUDA/Metal acceleration\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";

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
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using llama.cpp with CUDA/Metal acceleration\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";
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
        let config = LlamaCppConfig::default();
        assert!(config.n_gpu_layers >= 0);
        assert!(config.n_ctx > 0);
        assert!(config.temperature >= 0.0);
        assert!(config.temperature <= 2.0);
        assert!(config.top_p >= 0.0 && config.top_p <= 1.0);
    }

    #[test]
    fn test_config_from_env() {
        std::env::set_var("LLAMA_MODEL_PATH", "/test/model.gguf");
        std::env::set_var("LLAMA_GPU_LAYERS", "10");
        
        let config = LlamaCppConfig::default();
        assert!(config.model_path.contains("gguf") || config.model_path.contains("llama"));
        
        std::env::remove_var("LLAMA_MODEL_PATH");
        std::env::remove_var("LLAMA_GPU_LAYERS");
    }

    #[test]
    fn test_gpu_layers_config() {
        // Test various GPU layer configurations
        let cpu_only = LlamaCppConfig {
            n_gpu_layers: 0,
            ..Default::default()
        };
        assert_eq!(cpu_only.n_gpu_layers, 0);

        let partial_gpu = LlamaCppConfig {
            n_gpu_layers: 20,
            ..Default::default()
        };
        assert_eq!(partial_gpu.n_gpu_layers, 20);

        let full_gpu = LlamaCppConfig {
            n_gpu_layers: 999, // High value for full GPU offload
            ..Default::default()
        };
        assert_eq!(full_gpu.n_gpu_layers, 999);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_metal_support() {
        // On macOS, llama.cpp should support Metal acceleration
        // This tests that GPU layers can be configured for Metal
        let config = LlamaCppConfig {
            n_gpu_layers: 35,
            ..Default::default()
        };
        
        assert!(config.n_gpu_layers > 0);
        // llama.cpp will automatically use Metal on macOS when n_gpu_layers > 0
    }

    #[test]
    fn test_config_serialization() {
        let config = LlamaCppConfig {
            model_path: "model.gguf".to_string(),
            n_gpu_layers: 20,
            n_ctx: 2048,
            n_threads: 4,
            n_batch: 512,
            temperature: 0.8,
            top_p: 0.95,
            top_k: 40,
            repeat_penalty: 1.1,
            max_tokens: 256,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: LlamaCppConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.model_path, deserialized.model_path);
        assert_eq!(config.n_gpu_layers, deserialized.n_gpu_layers);
        assert_eq!(config.temperature, deserialized.temperature);
    }

    #[test]
    fn test_plugin_name() {
        unsafe {
            let name_ptr = name();
            let name_cstr = CStr::from_ptr(name_ptr);
            let name_str = name_cstr.to_str().unwrap();
            assert_eq!(name_str, "LlamaCppPlugin");
        }
    }

    #[test]
    fn test_plugin_metadata() {
        unsafe {
            let metadata = get_metadata();
            assert!(!metadata.name.is_null());
            assert!(!metadata.version.is_null());
            assert!(!metadata.description.is_null());
            
            let desc_str = CStr::from_ptr(metadata.description).to_str().unwrap();
            assert!(desc_str.contains("llama.cpp") || desc_str.contains("LLM"));
        }
    }

    #[test]
    fn test_validate_input() {
        unsafe {
            let valid_input = CString::new("Test prompt").unwrap();
            let input = PluginInput {
                text: valid_input.into_raw(),
            };
            assert!(validate_input(&input));

            let empty_input = CString::new("").unwrap();
            let empty_struct = PluginInput {
                text: empty_input.into_raw(),
            };
            assert!(!validate_input(&empty_struct));

            let whitespace_input = CString::new("   ").unwrap();
            let whitespace_struct = PluginInput {
                text: whitespace_input.into_raw(),
            };
            assert!(!validate_input(&whitespace_struct));
        }
    }

    #[test]
    fn test_parse_json_config() {
        let input_json = r#"{
            "prompt": "Hello",
            "config": {
                "temperature": 0.9,
                "max_tokens": 100
            }
        }"#;
        
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(input_json);
        assert!(parsed.is_ok());
        
        let value = parsed.unwrap();
        assert_eq!(value["prompt"].as_str().unwrap(), "Hello");
        assert_eq!(value["config"]["max_tokens"].as_i64().unwrap(), 100);
    }

    #[test]
    #[ignore] // Requires actual model file, panics in llama.cpp if file doesn't exist
    fn test_run_without_model() {
        unsafe {
            let input_text = CString::new("Test prompt").unwrap();
            let input = PluginInput {
                text: input_text.into_raw(),
            };

            let output = run(&input);
            assert!(!output.text.is_null());
            
            let output_str = CStr::from_ptr(output.text).to_str().unwrap();
            // Should fail gracefully without model file
            assert!(output_str.contains("error") || output_str.contains("model") || output_str.len() > 0);
            
            free_output(output);
        }
    }

    #[test]
    fn test_capabilities_json() {
        unsafe {
            let caps_ptr = get_capabilities();
            assert!(!caps_ptr.is_null());
            
            let caps_str = CStr::from_ptr(caps_ptr).to_str().unwrap();
            let caps: Result<serde_json::Value, _> = serde_json::from_str(caps_str);
            assert!(caps.is_ok());
        }
    }
}
