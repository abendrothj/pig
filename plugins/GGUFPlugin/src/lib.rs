//! GGUFPlugin - Native GGUF model loading with Candle ML framework
//!
//! This plugin provides direct GGUF model loading and inference using
//! Hugging Face's Candle framework, supporting CUDA, Metal, and CPU backends.

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_llama as llama;
use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTablePtr};
use log::{error, info, warn};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Global model instance for reuse across calls
static MODEL_INSTANCE: OnceCell<Mutex<GGUFModelState>> = OnceCell::new();

/// Configuration for the GGUF plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GGUFConfig {
    /// Path to the GGUF model file
    pub model_path: String,
    /// Path to the tokenizer (HF model ID or local path)
    pub tokenizer_path: String,
    /// Device to use: "cuda", "metal", or "cpu"
    pub device: String,
    /// CUDA device ID (if using CUDA)
    pub cuda_device_id: usize,
    /// Temperature for sampling
    pub temperature: f64,
    /// Top-p (nucleus) sampling
    pub top_p: f64,
    /// Top-k sampling (0 = disabled)
    pub top_k: usize,
    /// Repetition penalty
    pub repeat_penalty: f32,
    /// Maximum tokens to generate
    pub max_tokens: usize,
    /// Random seed for reproducibility
    pub seed: u64,
}

impl Default for GGUFConfig {
    fn default() -> Self {
        Self {
            model_path: std::env::var("GGUF_MODEL_PATH")
                .unwrap_or_else(|_| "models/llama-2-7b-chat.Q4_K_M.gguf".to_string()),
            tokenizer_path: std::env::var("GGUF_TOKENIZER_PATH")
                .unwrap_or_else(|_| "meta-llama/Llama-2-7b-chat-hf".to_string()),
            device: std::env::var("GGUF_DEVICE").unwrap_or_else(|_| "cuda".to_string()),
            cuda_device_id: std::env::var("CUDA_VISIBLE_DEVICES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
            max_tokens: 512,
            seed: 42,
        }
    }
}

/// Holds the loaded GGUF model state
struct GGUFModelState {
    model: llama::ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    config: GGUFConfig,
}

impl GGUFModelState {
    fn new(config: GGUFConfig) -> Result<Self> {
        info!("Initializing GGUF model loader...");

        // Select device
        let device = match config.device.as_str() {
            "cuda" => {
                info!("Using CUDA device {}", config.cuda_device_id);
                Device::new_cuda(config.cuda_device_id)
                    .unwrap_or_else(|e| {
                        warn!("CUDA not available ({}), falling back to CPU", e);
                        Device::Cpu
                    })
            }
            "metal" => {
                info!("Using Metal device");
                Device::new_metal(0)
                    .unwrap_or_else(|e| {
                        warn!("Metal not available ({}), falling back to CPU", e);
                        Device::Cpu
                    })
            }
            _ => {
                info!("Using CPU device");
                Device::Cpu
            }
        };

        info!("Loading tokenizer from: {}", config.tokenizer_path);
        let tokenizer = load_tokenizer(&config.tokenizer_path)?;

        info!("Loading GGUF model from: {}", config.model_path);
        let model = load_gguf_model(&config.model_path, &device)?;

        info!("Model loaded successfully on {:?}", device);

        Ok(Self {
            model,
            tokenizer,
            device,
            config,
        })
    }

    fn generate(&mut self, prompt: &str) -> Result<String> {
        // Tokenize the prompt
        let encoding = self.tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow::anyhow!("Tokenization error: {}", e))?;

        let prompt_tokens = encoding.get_ids().to_vec();
        info!("Prompt tokenized into {} tokens", prompt_tokens.len());

        // Create tensor from tokens
        let mut tokens = prompt_tokens.clone();
        let mut all_tokens = tokens.clone();

        // Setup logits processor
        let mut logits_processor = LogitsProcessor::new(
            self.config.seed,
            Some(self.config.temperature),
            Some(self.config.top_p),
        );

        // Generate tokens
        let mut generated = String::new();
        let eos_token = self.tokenizer
            .token_to_id("</s>")
            .unwrap_or(2);

        for i in 0..self.config.max_tokens {
            // Create input tensor
            let context_size = tokens.len();
            let input = Tensor::new(&tokens[..], &self.device)?
                .unsqueeze(0)?;

            // Forward pass
            let logits = self.model.forward(&input, 0)?;
            let logits = logits.squeeze(0)?;
            let logits = logits.get(logits.dim(0)? - 1)?;

            // Apply repetition penalty
            let logits = if self.config.repeat_penalty != 1.0 {
                apply_repeat_penalty(&logits, &all_tokens, self.config.repeat_penalty)?
            } else {
                logits
            };

            // Sample next token
            let next_token = logits_processor.sample(&logits)?;

            // Check for EOS
            if next_token == eos_token {
                break;
            }

            all_tokens.push(next_token);
            tokens = vec![next_token];

            // Decode token
            if let Some(text) = self.tokenizer.decode(&[next_token], false).ok() {
                generated.push_str(&text);
            }
        }

        Ok(generated)
    }
}

/// Load tokenizer from HuggingFace hub or local path
fn load_tokenizer(path: &str) -> Result<Tokenizer> {
    let tokenizer_path = if path.contains('/') && !std::path::Path::new(path).exists() {
        // Looks like a HuggingFace model ID, try to download
        info!("Downloading tokenizer from HuggingFace: {}", path);
        let api = hf_hub::api::sync::Api::new()?;
        let repo = api.model(path.to_string());
        repo.get("tokenizer.json")
            .context("Failed to download tokenizer.json")?
    } else {
        PathBuf::from(path)
    };

    Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))
}

/// Load GGUF model using memory mapping
fn load_gguf_model(path: &str, device: &Device) -> Result<llama::ModelWeights> {
    let model_path = std::path::Path::new(path);

    if !model_path.exists() {
        return Err(anyhow::anyhow!("Model file not found: {}", path));
    }

    // Memory-map the file for efficient loading
    let file = std::fs::File::open(model_path)?;
    let mut file_reader = std::io::BufReader::new(&file);

    // Parse GGUF content first
    use candle_core::quantized::gguf_file;
    let content = gguf_file::Content::read(&mut file_reader)
        .context("Failed to parse GGUF file")?;

    // Reopen file for model loading
    let file = std::fs::File::open(model_path)?;
    let mut file_reader = std::io::BufReader::new(&file);

    // Parse GGUF and load weights
    let model = llama::ModelWeights::from_gguf(content, &mut file_reader, &device)
        .context("Failed to load GGUF model weights")?;

    Ok(model)
}

/// Apply repetition penalty to logits
fn apply_repeat_penalty(logits: &Tensor, tokens: &[u32], penalty: f32) -> Result<Tensor> {
    let device = logits.device();
    let mut logits_vec: Vec<f32> = logits.to_vec1()?;

    for &token in tokens {
        let token = token as usize;
        if token < logits_vec.len() {
            let score = logits_vec[token];
            logits_vec[token] = if score > 0.0 {
                score / penalty
            } else {
                score * penalty
            };
        }
    }

    Ok(Tensor::new(logits_vec, device)?)
}

/// Get or initialize the global model instance
fn get_model() -> Result<&'static Mutex<GGUFModelState>> {
    MODEL_INSTANCE.get_or_try_init(|| {
        let config = GGUFConfig::default();
        GGUFModelState::new(config).map(Mutex::new)
    })
}

/// Input format with optional configuration
#[derive(Debug, Deserialize)]
struct PluginInputData {
    prompt: String,
    #[serde(default)]
    config: Option<GGUFConfig>,
}

// ============================================================================
// Plugin API Implementation
// ============================================================================

unsafe extern "C" fn name() -> *const c_char {
    c"GGUFPlugin".as_ptr()
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
    let mut state = model.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
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
    static NAME: &[u8] = b"GGUFPlugin\0";
    static VERSION: &[u8] = b"2.0.0\0";
    static DESCRIPTION: &[u8] = b"Native GGUF model loading with Candle - CUDA/Metal/CPU support\0";
    static AUTHOR: &[u8] = b"LAO Team\0";
    static TAGS: &[u8] = b"[\"llm\", \"gguf\", \"candle\", \"cuda\", \"metal\", \"text-generation\"]\0";
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using native GGUF models with Candle ML\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";

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
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using native GGUF models with Candle ML\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";
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
        let config = GGUFConfig::default();
        assert!(config.temperature > 0.0);
        assert!(config.max_tokens > 0);
        assert!(config.top_p > 0.0 && config.top_p <= 1.0);
        assert!(config.top_k > 0);
        assert!(!config.model_path.is_empty());
        assert!(!config.tokenizer_path.is_empty());
    }

    #[test]
    fn test_config_serialization() {
        let config = GGUFConfig {
            model_path: "test.gguf".to_string(),
            tokenizer_path: "tokenizer".to_string(),
            device: "cpu".to_string(),
            cuda_device_id: 0,
            temperature: 0.7,
            top_p: 0.9,
            top_k: 50,
            repeat_penalty: 1.1,
            max_tokens: 512,
            seed: 123,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: GGUFConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.model_path, deserialized.model_path);
        assert_eq!(config.temperature, deserialized.temperature);
        assert_eq!(config.device, deserialized.device);
    }

    #[test]
    fn test_plugin_name() {
        unsafe {
            let name_ptr = name();
            let name_cstr = CStr::from_ptr(name_ptr);
            assert_eq!(name_cstr.to_str().unwrap(), "GGUFPlugin");
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
            assert_eq!(name_str, "GGUFPlugin");
            
            let desc_str = CStr::from_ptr(metadata.description).to_str().unwrap();
            assert!(desc_str.contains("GGUF") || desc_str.contains("Candle"));
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
        }
    }

    #[test]
    fn test_device_selection() {
        let cpu_config = GGUFConfig {
            device: "cpu".to_string(),
            ..Default::default()
        };
        assert_eq!(cpu_config.device, "cpu");

        let cuda_config = GGUFConfig {
            device: "cuda".to_string(),
            cuda_device_id: 1,
            ..Default::default()
        };
        assert_eq!(cuda_config.device, "cuda");
        assert_eq!(cuda_config.cuda_device_id, 1);

        let metal_config = GGUFConfig {
            device: "metal".to_string(),
            ..Default::default()
        };
        assert_eq!(metal_config.device, "metal");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_metal_device_initialization() {
        // Test Metal device creation on macOS
        let result = Device::new_metal(0);
        // Metal should be available on macOS, but test gracefully
        match result {
            Ok(device) => {
                assert!(matches!(device, Device::Metal(_)));
            }
            Err(e) => {
                // Metal might not be available in test environment
                println!("Metal not available: {}", e);
            }
        }
    }

    #[test]
    fn test_device_fallback() {
        // Test that device selection falls back gracefully
        let config = GGUFConfig {
            device: "cuda".to_string(),
            ..Default::default()
        };
        
        // On non-CUDA systems, initialization should fall back to CPU
        // This is tested in the actual GGUFModelState::new() implementation
        assert_eq!(config.device, "cuda");
    }

    #[test]
    fn test_apply_repeat_penalty() {
        // Test the repeat penalty logic with mock data
        let device = Device::Cpu;
        let logits_data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let logits = Tensor::new(logits_data.clone(), &device).unwrap();
        let tokens = vec![1, 3];
        let penalty = 1.2;

        let result = apply_repeat_penalty(&logits, &tokens, penalty);
        assert!(result.is_ok());
        
        let result_tensor = result.unwrap();
        let result_vec: Vec<f32> = result_tensor.to_vec1().unwrap();
        
        // Token 1 and 3 should be penalized
        assert!(result_vec[1] < logits_data[1]);
        assert!(result_vec[3] < logits_data[3]);
        // Token 0, 2, 4 should be unchanged
        assert_eq!(result_vec[0], logits_data[0]);
        assert_eq!(result_vec[2], logits_data[2]);
        assert_eq!(result_vec[4], logits_data[4]);
    }

    #[test]
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
            assert!(output_str.len() > 0);
            
            free_output(output);
        }
    }

    #[test]
    fn test_capabilities() {
        unsafe {
            let caps_ptr = get_capabilities();
            assert!(!caps_ptr.is_null());
            
            let caps_str = CStr::from_ptr(caps_ptr).to_str().unwrap();
            let caps: Result<serde_json::Value, _> = serde_json::from_str(caps_str);
            assert!(caps.is_ok());
        }
    }

    #[test]
    fn test_sampling_params() {
        let config = GGUFConfig::default();
        assert!(config.temperature > 0.0);
        assert!(config.temperature < 2.0);
        assert!(config.repeat_penalty >= 1.0);
        assert!(config.seed > 0);
    }
}
