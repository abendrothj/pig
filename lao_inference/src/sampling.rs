//! Sampling strategies for text generation

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Sampling strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SamplingStrategy {
    /// Greedy decoding - always pick highest probability
    Greedy,
    /// Temperature sampling
    Temperature(f64),
    /// Top-p (nucleus) sampling
    TopP { p: f64, temperature: f64 },
    /// Top-k sampling
    TopK { k: usize, temperature: f64 },
    /// Combined top-k and top-p
    TopKTopP { k: usize, p: f64, temperature: f64 },
    /// Beam search
    BeamSearch { num_beams: usize, length_penalty: f64 },
    /// Contrastive search
    ContrastiveSearch { k: usize, alpha: f64 },
    /// Typical sampling
    TypicalP { p: f64, temperature: f64 },
    /// Mirostat sampling
    Mirostat { tau: f64, eta: f64 },
}

impl Default for SamplingStrategy {
    fn default() -> Self {
        Self::TopKTopP {
            k: 40,
            p: 0.9,
            temperature: 0.7,
        }
    }
}

/// Simple softmax implementation
pub fn softmax(logits: &mut [f32]) {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0f32;

    for logit in logits.iter_mut() {
        *logit = (*logit - max).exp();
        sum += *logit;
    }

    for logit in logits.iter_mut() {
        *logit /= sum;
    }
}

/// Apply temperature to logits
pub fn apply_temperature(logits: &mut [f32], temperature: f64) {
    if temperature <= 0.0 || temperature == 1.0 {
        return;
    }

    let temp = temperature as f32;
    for logit in logits.iter_mut() {
        *logit /= temp;
    }
}

/// Apply top-k filtering
pub fn apply_top_k(logits: &mut [f32], k: usize) {
    if k == 0 || k >= logits.len() {
        return;
    }

    // Find the k-th largest value
    let mut sorted: Vec<f32> = logits.iter().cloned().collect();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let threshold = sorted[k - 1];

    // Zero out values below threshold
    for logit in logits.iter_mut() {
        if *logit < threshold {
            *logit = f32::NEG_INFINITY;
        }
    }
}

/// Apply top-p (nucleus) filtering
pub fn apply_top_p(logits: &mut [f32], p: f64) {
    if p >= 1.0 {
        return;
    }

    // Convert to probabilities
    let mut probs: Vec<(usize, f32)> = logits.iter().cloned().enumerate().collect();
    softmax(&mut probs.iter().map(|(_, v)| *v).collect::<Vec<_>>());

    // Sort by probability descending
    probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Find cutoff
    let mut cumsum = 0.0f32;
    let mut cutoff_idx = probs.len();

    for (i, (_, prob)) in probs.iter().enumerate() {
        cumsum += prob;
        if cumsum > p as f32 {
            cutoff_idx = i + 1;
            break;
        }
    }

    // Create set of allowed indices
    let allowed: std::collections::HashSet<usize> = probs[..cutoff_idx]
        .iter()
        .map(|(idx, _)| *idx)
        .collect();

    // Zero out values not in top-p
    for (i, logit) in logits.iter_mut().enumerate() {
        if !allowed.contains(&i) {
            *logit = f32::NEG_INFINITY;
        }
    }
}

/// Apply repetition penalty
pub fn apply_repetition_penalty(logits: &mut [f32], tokens: &[u32], penalty: f32) {
    if penalty == 1.0 {
        return;
    }

    for &token in tokens {
        let idx = token as usize;
        if idx < logits.len() {
            if logits[idx] > 0.0 {
                logits[idx] /= penalty;
            } else {
                logits[idx] *= penalty;
            }
        }
    }
}

/// Sample from probability distribution
pub fn sample_from_probs(probs: &[f32], rng: &mut impl FnMut() -> f32) -> usize {
    let r = rng();
    let mut cumsum = 0.0f32;

    for (i, &prob) in probs.iter().enumerate() {
        cumsum += prob;
        if r < cumsum {
            return i;
        }
    }

    // Fallback to last token
    probs.len() - 1
}

/// Full sampling pipeline
pub fn sample_token(
    logits: &mut [f32],
    strategy: &SamplingStrategy,
    previous_tokens: &[u32],
    repetition_penalty: f32,
    rng: &mut impl FnMut() -> f32,
) -> u32 {
    // Apply repetition penalty
    apply_repetition_penalty(logits, previous_tokens, repetition_penalty);

    match strategy {
        SamplingStrategy::Greedy => {
            logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i as u32)
                .unwrap_or(0)
        }

        SamplingStrategy::Temperature(temp) => {
            apply_temperature(logits, *temp);
            softmax(logits);
            sample_from_probs(logits, rng) as u32
        }

        SamplingStrategy::TopP { p, temperature } => {
            apply_temperature(logits, *temperature);
            apply_top_p(logits, *p);
            softmax(logits);
            sample_from_probs(logits, rng) as u32
        }

        SamplingStrategy::TopK { k, temperature } => {
            apply_temperature(logits, *temperature);
            apply_top_k(logits, *k);
            softmax(logits);
            sample_from_probs(logits, rng) as u32
        }

        SamplingStrategy::TopKTopP { k, p, temperature } => {
            apply_temperature(logits, *temperature);
            apply_top_k(logits, *k);
            apply_top_p(logits, *p);
            softmax(logits);
            sample_from_probs(logits, rng) as u32
        }

        _ => {
            // Default to greedy for unimplemented strategies
            logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i as u32)
                .unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_softmax() {
        let mut logits = vec![1.0, 2.0, 3.0];
        softmax(&mut logits);

        let sum: f32 = logits.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(logits[2] > logits[1]);
        assert!(logits[1] > logits[0]);
    }

    #[test]
    fn test_top_k() {
        let mut logits = vec![1.0, 5.0, 2.0, 4.0, 3.0];
        apply_top_k(&mut logits, 2);

        // Only top 2 values should remain
        assert!(logits[1] > f32::NEG_INFINITY); // 5.0
        assert!(logits[3] > f32::NEG_INFINITY); // 4.0
        assert!(logits[0] == f32::NEG_INFINITY);
    }

    #[test]
    fn test_repetition_penalty() {
        let mut logits = vec![1.0, 2.0, 3.0];
        let tokens = vec![1];
        apply_repetition_penalty(&mut logits, &tokens, 2.0);

        assert_eq!(logits[0], 1.0);
        assert_eq!(logits[1], 1.0); // Penalized from 2.0
        assert_eq!(logits[2], 3.0);
    }
}
