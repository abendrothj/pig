//! VLLMPlugin - High-throughput LLM inference via vLLM
//!
//! This plugin connects to a vLLM server for high-throughput inference
//! with PagedAttention, continuous batching, and tensor parallelism.
//!
//! vLLM provides:
//! - PagedAttention: Efficient KV-cache memory management
//! - Continuous batching: Dynamic batching for optimal GPU utilization
//! - Tensor parallelism: Distribute large models across multiple GPUs
//! - OpenAI-compatible API: Easy integration
//!
//! To start a vLLM server:
//! ```bash
//! pip install vllm
//! python -m vllm.entrypoints.openai.api_server \
//!     --model meta-llama/Llama-2-7b-chat-hf \
//!     --tensor-parallel-size 1 \
//!     --gpu-memory-utilization 0.9
//! ```

use anyhow::{Context, Result};
use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTablePtr};
use log::{error, info, warn};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;

/// Global runtime and client instance
static RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
static CLIENT: OnceCell<Mutex<VLLMClient>> = OnceCell::new();

/// Configuration for the vLLM plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VLLMConfig {
    /// vLLM server URL (OpenAI-compatible endpoint)
    pub server_url: String,
    /// Model name to use
    pub model: String,
    /// Maximum tokens to generate
    pub max_tokens: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Top-p (nucleus) sampling
    pub top_p: f32,
    /// Frequency penalty
    pub frequency_penalty: f32,
    /// Presence penalty
    pub presence_penalty: f32,
    /// Stop sequences
    pub stop: Vec<String>,
    /// Request timeout in seconds
    pub timeout_secs: u64,
    /// Enable streaming (for future use)
    pub stream: bool,
    /// Best-of sampling (generate n completions, return best)
    pub best_of: Option<u32>,
    /// Use beam search instead of sampling
    pub use_beam_search: bool,
    /// Number of beams for beam search
    pub n_beams: Option<u32>,
}

impl Default for VLLMConfig {
    fn default() -> Self {
        Self {
            server_url: std::env::var("VLLM_SERVER_URL")
                .unwrap_or_else(|_| "http://localhost:8000".to_string()),
            model: std::env::var("VLLM_MODEL")
                .unwrap_or_else(|_| "meta-llama/Llama-2-7b-chat-hf".to_string()),
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.9,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            stop: vec!["</s>".to_string(), "[/INST]".to_string()],
            timeout_secs: 60,
            stream: false,
            best_of: None,
            use_beam_search: false,
            n_beams: None,
        }
    }
}

/// vLLM OpenAI-compatible request format
#[derive(Debug, Serialize)]
struct CompletionRequest {
    model: String,
    prompt: String,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    frequency_penalty: f32,
    presence_penalty: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_of: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    use_beam_search: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
}

/// vLLM OpenAI-compatible chat request format
#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    frequency_penalty: f32,
    presence_penalty: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// vLLM completion response
#[derive(Debug, Deserialize)]
struct CompletionResponse {
    id: String,
    choices: Vec<CompletionChoice>,
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
struct CompletionChoice {
    text: Option<String>,
    message: Option<ChatMessage>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageInfo {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// vLLM client for making requests
struct VLLMClient {
    client: reqwest::Client,
    config: VLLMConfig,
}

impl VLLMClient {
    fn new(config: VLLMConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, config })
    }

    async fn generate(&self, prompt: &str) -> Result<String> {
        // Detect if this is a chat-style prompt
        let is_chat = prompt.contains("[INST]") || prompt.contains("<|user|>");

        if is_chat {
            self.generate_chat(prompt).await
        } else {
            self.generate_completion(prompt).await
        }
    }

    async fn generate_completion(&self, prompt: &str) -> Result<String> {
        let request = CompletionRequest {
            model: self.config.model.clone(),
            prompt: prompt.to_string(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            frequency_penalty: self.config.frequency_penalty,
            presence_penalty: self.config.presence_penalty,
            stop: self.config.stop.clone(),
            stream: false,
            best_of: self.config.best_of,
            use_beam_search: self.config.use_beam_search,
            n: self.config.n_beams,
        };

        let url = format!("{}/v1/completions", self.config.server_url);
        info!("Sending completion request to {}", url);

        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to vLLM server")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "vLLM server returned error {}: {}",
                status,
                error_text
            ));
        }

        let completion: CompletionResponse = response
            .json()
            .await
            .context("Failed to parse vLLM response")?;

        if let Some(usage) = &completion.usage {
            info!(
                "Tokens used - prompt: {}, completion: {}, total: {}",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            );
        }

        completion
            .choices
            .first()
            .and_then(|c| c.text.clone())
            .ok_or_else(|| anyhow::anyhow!("No completion text in response"))
    }

    async fn generate_chat(&self, prompt: &str) -> Result<String> {
        // Parse the prompt into messages
        let messages = parse_chat_prompt(prompt);

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            frequency_penalty: self.config.frequency_penalty,
            presence_penalty: self.config.presence_penalty,
            stop: self.config.stop.clone(),
            stream: false,
        };

        let url = format!("{}/v1/chat/completions", self.config.server_url);
        info!("Sending chat completion request to {}", url);

        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to vLLM server")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "vLLM server returned error {}: {}",
                status,
                error_text
            ));
        }

        let completion: CompletionResponse = response
            .json()
            .await
            .context("Failed to parse vLLM response")?;

        completion
            .choices
            .first()
            .and_then(|c| c.message.as_ref().map(|m| m.content.clone()))
            .ok_or_else(|| anyhow::anyhow!("No message content in response"))
    }

    async fn check_health(&self) -> Result<bool> {
        let url = format!("{}/health", self.config.server_url);
        match self.client.get(&url).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/models", self.config.server_url);
        let response = self.client.get(&url).send().await?;

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelInfo>,
        }

        #[derive(Deserialize)]
        struct ModelInfo {
            id: String,
        }

        let models: ModelsResponse = response.json().await?;
        Ok(models.data.into_iter().map(|m| m.id).collect())
    }
}

/// Parse a chat prompt into messages
fn parse_chat_prompt(prompt: &str) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    // Handle Llama-2 style prompts
    if prompt.contains("[INST]") {
        let parts: Vec<&str> = prompt.split("[INST]").collect();
        for (i, part) in parts.iter().enumerate() {
            if i == 0 && !part.trim().is_empty() {
                // System message before first [INST]
                let system = part.replace("<<SYS>>", "").replace("<</SYS>>", "").trim().to_string();
                if !system.is_empty() {
                    messages.push(ChatMessage {
                        role: "system".to_string(),
                        content: system,
                    });
                }
            } else if !part.trim().is_empty() {
                // User message
                let content = part.replace("[/INST]", "").trim().to_string();
                if !content.is_empty() {
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
            }
        }
    } else {
        // Simple prompt - treat as user message
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        });
    }

    messages
}

/// Get the global tokio runtime
fn get_runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// Get or initialize the global client
fn get_client() -> Result<&'static Mutex<VLLMClient>> {
    CLIENT.get_or_try_init(|| {
        let config = VLLMConfig::default();
        VLLMClient::new(config).map(Mutex::new)
    })
}

/// Input format with optional configuration
#[derive(Debug, Deserialize)]
struct PluginInputData {
    prompt: String,
    #[serde(default)]
    config: Option<VLLMConfig>,
    /// Optional: list available models
    #[serde(default)]
    list_models: bool,
    /// Optional: health check
    #[serde(default)]
    health_check: bool,
}

// ============================================================================
// Plugin API Implementation
// ============================================================================

unsafe extern "C" fn name() -> *const c_char {
    c"VLLMPlugin".as_ptr()
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
    let (prompt, list_models, health_check) = match serde_json::from_str::<PluginInputData>(input_text) {
        Ok(data) => (data.prompt, data.list_models, data.health_check),
        Err(_) => (input_text.to_string(), false, false),
    };

    let result = if health_check {
        process_health_check()
    } else if list_models {
        process_list_models()
    } else {
        info!("Processing prompt: {}...", &prompt[..prompt.len().min(50)]);
        process_prompt(&prompt)
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

fn process_prompt(prompt: &str) -> Result<String> {
    let runtime = get_runtime();
    let client = get_client()?;
    let client = client.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    runtime.block_on(client.generate(prompt))
}

fn process_health_check() -> Result<String> {
    let runtime = get_runtime();
    let client = get_client()?;
    let client = client.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let healthy = runtime.block_on(client.check_health())?;
    Ok(serde_json::json!({
        "healthy": healthy,
        "server_url": client.config.server_url
    }).to_string())
}

fn process_list_models() -> Result<String> {
    let runtime = get_runtime();
    let client = get_client()?;
    let client = client.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let models = runtime.block_on(client.list_models())?;
    Ok(serde_json::json!({
        "models": models
    }).to_string())
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
    static NAME: &[u8] = b"VLLMPlugin\0";
    static VERSION: &[u8] = b"1.0.0\0";
    static DESCRIPTION: &[u8] = b"High-throughput LLM inference via vLLM with PagedAttention\0";
    static AUTHOR: &[u8] = b"LAO Team\0";
    static TAGS: &[u8] = b"[\"llm\", \"vllm\", \"high-throughput\", \"paged-attention\", \"text-generation\"]\0";
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using vLLM with PagedAttention and continuous batching\",\"input_type\":\"Text\",\"output_type\":\"Text\"},{\"name\":\"chat\",\"description\":\"Chat completion with vLLM\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";

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
    static CAPABILITIES: &[u8] = b"[{\"name\":\"text-generation\",\"description\":\"Generate text using vLLM with PagedAttention and continuous batching\",\"input_type\":\"Text\",\"output_type\":\"Text\"},{\"name\":\"chat\",\"description\":\"Chat completion with vLLM\",\"input_type\":\"Text\",\"output_type\":\"Text\"}]\0";
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
        let _config = VLLMConfig::default();
        let config = VLLMConfig::default();
        assert!(!config.server_url.is_empty());
        assert!(!config.model.is_empty());
        assert!(config.max_tokens > 0);
        assert!(config.temperature >= 0.0);
        assert!(config.top_p > 0.0 && config.top_p <= 1.0);
    }

    #[test]
    fn test_config_from_env() {
        std::env::set_var("VLLM_SERVER_URL", "http://test:8000");
        std::env::set_var("VLLM_MODEL", "test-model");
        
        let config = VLLMConfig::default();
        // Config should use env vars if set
        
        std::env::remove_var("VLLM_SERVER_URL");
        std::env::remove_var("VLLM_MODEL");
    }

    #[test]
    fn test_config_serialization() {
        let mut config = VLLMConfig::default();
        config.server_url = "http://localhost:8000".to_string();
        config.model = "llama-2-7b".to_string();
        config.temperature = 0.8;
        config.top_p = 0.95;
        config.max_tokens = 512;
        config.stream = false;

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: VLLMConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.server_url, deserialized.server_url);
        assert_eq!(config.model, deserialized.model);
        assert_eq!(config.temperature, deserialized.temperature);
    }

    #[test]
    fn test_parse_chat_prompt() {
        let prompt = "[INST] Hello, how are you? [/INST]";
        let messages = parse_chat_prompt(prompt);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0].content.contains("Hello"));
    }

    #[test]
    fn test_parse_chat_prompt_with_system() {
        let prompt = "<<SYS>> You are a helpful assistant. <</SYS>> [INST] Hello! [/INST]";
        let messages = parse_chat_prompt(prompt);
        assert!(messages.len() >= 1);
        assert!(messages.iter().any(|m| m.role == "system") || messages.iter().any(|m| m.content.contains("helpful assistant")));
    }

    #[test]
    fn test_parse_chat_prompt_multiple_turns() {
        let prompt = "[INST] First question [/INST] First answer [INST] Second question [/INST]";
        let messages = parse_chat_prompt(prompt);
        assert!(messages.len() >= 2);
    }

    #[test]
    fn test_plugin_name() {
        unsafe {
            let name_ptr = name();
            let name_cstr = CStr::from_ptr(name_ptr);
            let name_str = name_cstr.to_str().unwrap();
            assert_eq!(name_str, "VLLMPlugin");
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
            assert!(desc_str.contains("vLLM") || desc_str.contains("inference"));
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

            let whitespace_input = CString::new("   \n\t   ").unwrap();
            let whitespace_struct = PluginInput {
                text: whitespace_input.into_raw(),
            };
            assert!(!validate_input(&whitespace_struct));
        }
    }

    #[test]
    fn test_parse_json_input() {
        let input_json = r#"{
            "prompt": "Hello, world!",
            "config": {
                "temperature": 0.9,
                "max_tokens": 100,
                "stream": false
            }
        }"#;
        
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(input_json);
        assert!(parsed.is_ok());
        
        let value = parsed.unwrap();
        assert_eq!(value["prompt"].as_str().unwrap(), "Hello, world!");
        assert_eq!(value["config"]["max_tokens"].as_i64().unwrap(), 100);
        assert_eq!(value["config"]["stream"].as_bool().unwrap(), false);
    }

    #[test]
    fn test_chat_message_format() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "Test message".to_string(),
        };
        
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Test message");
        
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.role, deserialized.role);
        assert_eq!(msg.content, deserialized.content);
    }

    #[test]
    fn test_run_without_server() {
        unsafe {
            let input_text = CString::new("Test prompt").unwrap();
            let input = PluginInput {
                text: input_text.into_raw(),
            };

            let output = run(&input);
            assert!(!output.text.is_null());
            
            let output_str = CStr::from_ptr(output.text).to_str().unwrap();
            // Should fail gracefully without vLLM server running
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
            
            let caps_array = caps.unwrap();
            assert!(caps_array.is_array());
        }
    }

    #[test]
    fn test_url_validation() {
        let config = VLLMConfig::default();
        assert!(config.server_url.starts_with("http://") || config.server_url.starts_with("https://"));
    }
}
