//! macOS-specific performance utilities for Apple Silicon

#[cfg(target_os = "macos")]
pub mod apple_silicon {
    use std::process::Command;

    /// Detect if running on Apple Silicon
    pub fn is_apple_silicon() -> bool {
        #[cfg(target_arch = "aarch64")]
        {
            true
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            false
        }
    }

    /// Get total unified memory available (GB)
    pub fn get_unified_memory_gb() -> Option<f64> {
        let output = Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
            .ok()?;

        let memsize_str = String::from_utf8(output.stdout).ok()?;
        let memsize_bytes: u64 = memsize_str.trim().parse().ok()?;
        Some(memsize_bytes as f64 / 1_073_741_824.0) // Convert to GB
    }

    /// Get chip model (M1, M2, M3, etc.)
    pub fn get_chip_model() -> Option<String> {
        let output = Command::new("sysctl")
            .arg("-n")
            .arg("machdep.cpu.brand_string")
            .output()
            .ok()?;

        let brand = String::from_utf8(output.stdout).ok()?;
        
        // Extract M1/M2/M3/M4 from brand string
        if brand.contains("M1") {
            Some("M1".to_string())
        } else if brand.contains("M2") {
            Some("M2".to_string())
        } else if brand.contains("M3") {
            Some("M3".to_string())
        } else if brand.contains("M4") {
            Some("M4".to_string())
        } else {
            Some("Unknown Apple Silicon".to_string())
        }
    }

    /// Get number of performance and efficiency cores
    pub fn get_core_counts() -> Option<(usize, usize)> {
        let p_cores = Command::new("sysctl")
            .arg("-n")
            .arg("hw.perflevel0.logicalcpu")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        let e_cores = Command::new("sysctl")
            .arg("-n")
            .arg("hw.perflevel1.logicalcpu")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        if p_cores > 0 || e_cores > 0 {
            Some((p_cores, e_cores))
        } else {
            None
        }
    }

    /// Check if Metal is available
    pub fn is_metal_available() -> bool {
        Command::new("system_profiler")
            .arg("SPDisplaysDataType")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.contains("Metal"))
            .unwrap_or(false)
    }

    /// Check if running under Rosetta 2 translation
    pub fn is_running_under_rosetta() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            // If compiled for x86_64, check if on Apple Silicon hardware
            let output = Command::new("sysctl")
                .arg("-n")
                .arg("sysctl.proc_translated")
                .output();
            
            match output {
                Ok(out) => {
                    let result = String::from_utf8_lossy(&out.stdout);
                    result.trim() == "1"
                }
                Err(_) => false,
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            // Native ARM64 build
            false
        }
    }

    /// Warn user if running under Rosetta 2
    pub fn warn_if_rosetta() {
        if is_running_under_rosetta() {
            eprintln!("\n⚠️  WARNING: Running under Rosetta 2 translation!");
            eprintln!("   Performance is 40-50% slower than native ARM64.");
            eprintln!("   For optimal performance, use the native build:");
            eprintln!("   $ ./scripts/build-apple-silicon.sh\n");
        }
    }

    /// Get performance estimate compared to native
    pub fn get_performance_factor() -> f64 {
        if is_running_under_rosetta() {
            0.5 // Rosetta 2 is ~50% of native speed
        } else if is_apple_silicon() {
            1.0 // Native ARM64
        } else {
            0.8 // Intel Mac
        }
    }

    /// Get recommended thread count for this system
    pub fn get_recommended_threads() -> usize {
        // For Apple Silicon, use all available threads
        // macOS scheduler will place them optimally
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
    }

    /// Get recommended GPU layers for model size
    pub fn get_recommended_gpu_layers(model_size_gb: f64) -> i32 {
        let memory_gb = get_unified_memory_gb().unwrap_or(8.0);
        
        // Conservative estimates to leave room for system
        let available_gb = memory_gb * 0.6; // Use 60% of total
        
        if available_gb >= model_size_gb * 1.5 {
            999 // Full offload
        } else if available_gb >= model_size_gb {
            35 // Partial offload
        } else {
            0 // CPU only
        }
    }

    /// Print system information
    pub fn print_system_info() {
        println!("🍎 Apple Silicon System Information");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        if let Some(chip) = get_chip_model() {
            println!("Chip: {}", chip);
        }
        
        if let Some(memory) = get_unified_memory_gb() {
            println!("Unified Memory: {:.1} GB", memory);
        }
        
        if let Some((p, e)) = get_core_counts() {
            println!("Cores: {} Performance + {} Efficiency", p, e);
        }
        
        println!("Metal Available: {}", if is_metal_available() { "Yes" } else { "No" });
        println!("Recommended Threads: {}", get_recommended_threads());
        println!();
    }

    /// Optimize configuration for Apple Silicon
    pub fn optimize_config(config: &mut serde_json::Value) {
        if !is_apple_silicon() {
            return;
        }

        // Set Metal as default device if available
        if is_metal_available() {
            if let Some(device) = config.get_mut("device") {
                if device.as_str() == Some("cpu") {
                    *device = serde_json::json!("metal");
                }
            } else {
                config["device"] = serde_json::json!("metal");
            }
        }

        // Set recommended thread count
        if !config.get("n_threads").is_some() {
            config["n_threads"] = serde_json::json!(get_recommended_threads());
        }

        // Suggest full GPU offload if enough memory
        if let Some(memory_gb) = get_unified_memory_gb() {
            if memory_gb >= 16.0 && !config.get("n_gpu_layers").is_some() {
                config["n_gpu_layers"] = serde_json::json!(999);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub mod apple_silicon {
    pub fn is_apple_silicon() -> bool {
        false
    }
    
    pub fn print_system_info() {
        println!("Not running on macOS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_apple_silicon_detection() {
        let is_as = apple_silicon::is_apple_silicon();
        #[cfg(target_arch = "aarch64")]
        assert!(is_as);
        #[cfg(not(target_arch = "aarch64"))]
        assert!(!is_as);
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn test_system_info() {
        // Should not panic
        apple_silicon::print_system_info();
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn test_memory_detection() {
        let mem = apple_silicon::get_unified_memory_gb();
        assert!(mem.is_some());
        assert!(mem.unwrap() > 0.0);
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn test_chip_model() {
        let chip = apple_silicon::get_chip_model();
        assert!(chip.is_some());
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn test_metal_availability() {
        // On Apple Silicon, Metal should be available
        assert!(apple_silicon::is_metal_available());
    }

    #[test]
    fn test_recommended_threads() {
        let threads = apple_silicon::get_recommended_threads();
        assert!(threads > 0);
        assert!(threads <= 128); // Sanity check
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_config_optimization() {
        let mut config = serde_json::json!({
            "model_path": "test.gguf"
        });
        
        apple_silicon::optimize_config(&mut config);
        
        // Should have added optimizations
        if apple_silicon::is_apple_silicon() {
            assert!(config.get("device").is_some());
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_rosetta_detection() {
        // Should not panic
        let is_rosetta = apple_silicon::is_running_under_rosetta();
        
        // On native ARM64, should be false
        #[cfg(target_arch = "aarch64")]
        assert!(!is_rosetta);
        
        // Can't reliably test x86_64 on Rosetta without special setup
        // but at least verify it doesn't crash
    }

    #[test]
    fn test_performance_factor() {
        let factor = apple_silicon::get_performance_factor();
        assert!(factor > 0.0);
        assert!(factor <= 1.0);
        
        // Native ARM64 should be 1.0
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(factor, 1.0);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_warn_if_rosetta() {
        // Should not panic
        apple_silicon::warn_if_rosetta();
    }
}
