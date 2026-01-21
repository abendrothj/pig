//! Common configuration structures for inference

use serde::{Deserialize, Serialize};

/// Common generation parameters used across all backends
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    /// Maximum number of tokens to generate
    pub max_tokens: usize,
    /// Temperature for sampling (0.0 = greedy)
    pub temperature: f64,
    /// Top-p (nucleus) sampling threshold
    pub top_p: f64,
    /// Top-k sampling (0 = disabled)
    pub top_k: usize,
    /// Repetition penalty (1.0 = no penalty)
    pub repetition_penalty: f32,
    /// Frequency penalty
    pub frequency_penalty: f32,
    /// Presence penalty
    pub presence_penalty: f32,
    /// Stop sequences
    pub stop_sequences: Vec<String>,
    /// Random seed for reproducibility (None = random)
    pub seed: Option<u64>,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repetition_penalty: 1.1,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            stop_sequences: vec!["</s>".to_string()],
            seed: None,
        }
    }
}

impl GenerationConfig {
    /// Create a greedy decoding config
    pub fn greedy() -> Self {
        Self {
            temperature: 0.0,
            top_k: 1,
            ..Default::default()
        }
    }

    /// Create a creative/diverse config
    pub fn creative() -> Self {
        Self {
            temperature: 1.0,
            top_p: 0.95,
            top_k: 0,
            ..Default::default()
        }
    }

    /// Create a balanced config
    pub fn balanced() -> Self {
        Self::default()
    }
}

/// Model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Path to the model file or HuggingFace model ID
    pub model_path: String,
    /// Path to tokenizer (if separate from model)
    pub tokenizer_path: Option<String>,
    /// Model architecture type
    pub architecture: ModelArchitecture,
    /// Quantization type
    pub quantization: Option<QuantizationType>,
    /// Context length
    pub context_length: usize,
    /// Hidden size
    pub hidden_size: usize,
    /// Number of attention heads
    pub num_attention_heads: usize,
    /// Number of layers
    pub num_layers: usize,
    /// Vocabulary size
    pub vocab_size: usize,
}

/// Supported model architectures
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelArchitecture {
    Llama,
    Llama2,
    Llama3,
    Mistral,
    Mixtral,
    Phi,
    Phi2,
    Phi3,
    Qwen,
    Qwen2,
    Gemma,
    Gemma2,
    StableLM,
    Falcon,
    MPT,
    Custom,
}

/// Quantization types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum QuantizationType {
    /// No quantization (FP32)
    None,
    /// FP16
    F16,
    /// BF16
    BF16,
    /// 8-bit quantization
    Q8_0,
    /// 4-bit quantization (K-quants)
    Q4_K_M,
    Q4_K_S,
    Q5_K_M,
    Q5_K_S,
    Q6_K,
    /// 2-bit quantization
    Q2_K,
    /// GPTQ quantization
    GPTQ,
    /// AWQ quantization
    AWQ,
}

/// Backend selection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InferenceBackend {
    /// Direct llama.cpp integration
    LlamaCpp,
    /// Candle (Hugging Face Rust ML)
    Candle,
    /// vLLM server
    VLLM,
    /// Custom CUDA kernels
    CudaCustom,
    /// Ollama (HTTP API)
    Ollama,
    /// LM Studio (HTTP API)
    LMStudio,
}
