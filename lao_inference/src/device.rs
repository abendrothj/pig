//! Device abstraction for inference

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Device type for inference
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeviceType {
    /// CPU execution
    Cpu,
    /// NVIDIA CUDA GPU
    Cuda(usize),
    /// Apple Metal GPU
    Metal(usize),
    /// AMD ROCm GPU
    Rocm(usize),
}

impl Default for DeviceType {
    fn default() -> Self {
        // Try to detect best available device
        if cfg!(feature = "cuda") {
            Self::Cuda(0)
        } else if cfg!(target_os = "macos") {
            Self::Metal(0)
        } else {
            Self::Cpu
        }
    }
}

impl DeviceType {
    /// Check if this is a GPU device
    pub fn is_gpu(&self) -> bool {
        !matches!(self, Self::Cpu)
    }

    /// Get device ID (0 for CPU)
    pub fn device_id(&self) -> usize {
        match self {
            Self::Cpu => 0,
            Self::Cuda(id) | Self::Metal(id) | Self::Rocm(id) => *id,
        }
    }
}

/// Device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_type: DeviceType,
    pub name: String,
    pub total_memory: u64,
    pub free_memory: u64,
    pub compute_capability: Option<(u32, u32)>,
}

/// Detect available devices
pub fn detect_devices() -> Vec<DeviceInfo> {
    let mut devices = vec![DeviceInfo {
        device_type: DeviceType::Cpu,
        name: "CPU".to_string(),
        total_memory: get_system_memory(),
        free_memory: get_available_memory(),
        compute_capability: None,
    }];

    #[cfg(feature = "cuda")]
    {
        if let Ok(cuda_devices) = detect_cuda_devices() {
            devices.extend(cuda_devices);
        }
    }

    devices
}

fn get_system_memory() -> u64 {
    // Platform-specific memory detection
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }

    // Default fallback
    16 * 1024 * 1024 * 1024 // 16 GB
}

fn get_available_memory() -> u64 {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemAvailable:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }

    get_system_memory() / 2 // Assume half available
}

#[cfg(feature = "cuda")]
fn detect_cuda_devices() -> Result<Vec<DeviceInfo>> {
    use cudarc::driver::CudaDevice;

    let mut devices = Vec::new();
    let mut device_id = 0;

    while let Ok(device) = CudaDevice::new(device_id) {
        let (free, total) = device.memory_info()?;

        devices.push(DeviceInfo {
            device_type: DeviceType::Cuda(device_id),
            name: format!("CUDA Device {}", device_id),
            total_memory: total as u64,
            free_memory: free as u64,
            compute_capability: None, // Would need additional CUDA calls
        });

        device_id += 1;

        // Limit search to reasonable number
        if device_id >= 16 {
            break;
        }
    }

    Ok(devices)
}

#[cfg(not(feature = "cuda"))]
fn detect_cuda_devices() -> Result<Vec<DeviceInfo>> {
    Ok(Vec::new())
}
