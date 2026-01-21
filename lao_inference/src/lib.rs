//! LAO Inference Library
//!
//! Shared utilities and abstractions for LLM inference across LAO plugins.
//!
//! This library provides:
//! - Common configuration structures
//! - Device abstraction (CPU, CUDA, Metal)
//! - Tokenization utilities
//! - Sampling strategies
//! - Model loading helpers

pub mod config;
pub mod device;
pub mod sampling;
pub mod tokenizer;

#[cfg(feature = "candle")]
pub mod candle_utils;

#[cfg(feature = "llama-cpp")]
pub mod llama_utils;

#[cfg(feature = "cuda")]
pub mod cuda_utils;

pub use config::*;
pub use device::*;
pub use sampling::*;
