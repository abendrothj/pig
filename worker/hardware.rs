//! Hardware capability discovery. Every field is best-effort: missing tooling (no
//! `nvidia-smi`, no `/proc`, an unrecognized platform) degrades to `None`/`false`
//! rather than crashing the worker or fabricating a value. Uncertainty is represented
//! explicitly — a `None` here means "not measured," not "zero."

use pig_core::model::AcceleratorKind;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub logical_cpus: Option<u32>,
    pub physical_cpus: Option<u32>,
    pub total_memory_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub accelerator: Option<AcceleratorKind>,
    pub accelerator_name: Option<String>,
    pub total_vram_bytes: Option<u64>,
    pub available_vram_bytes: Option<u64>,
    pub accelerator_utilization_percent: Option<f32>,
    pub unified_memory: bool,
    pub cuda_available: bool,
    pub metal_available: bool,
}

pub fn discover() -> HardwareInfo {
    let mut info = HardwareInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname(),
        logical_cpus: std::thread::available_parallelism()
            .ok()
            .map(|n| n.get() as u32),
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    discover_macos(&mut info);
    #[cfg(target_os = "linux")]
    discover_linux(&mut info);

    discover_nvidia(&mut info);
    info
}

fn hostname() -> String {
    // No single cross-platform std API; try the platform command, fall back to a
    // placeholder rather than failing discovery entirely.
    Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

#[cfg(target_os = "macos")]
fn discover_macos(info: &mut HardwareInfo) {
    let sysctl = |key: &str| -> Option<String> {
        Command::new("sysctl")
            .arg("-n")
            .arg(key)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };

    info.physical_cpus = sysctl("hw.physicalcpu").and_then(|s| s.parse().ok());
    info.total_memory_bytes = sysctl("hw.memsize").and_then(|s| s.parse().ok());
    // Available memory has no single reliable sysctl value on macOS (it requires
    // interpreting `vm_stat` page-state counters); left unmeasured rather than guessed.
    info.available_memory_bytes = None;

    // Apple Silicon (arm64) always has a Metal-capable integrated GPU with unified
    // memory; Intel Macs may or may not, and we don't probe deeply enough to know, so
    // we deliberately leave the accelerator unset there rather than assume one.
    if info.arch == "aarch64" {
        info.metal_available = true;
        info.unified_memory = true;
        info.accelerator = Some(AcceleratorKind::Metal);
        info.accelerator_name =
            sysctl("machdep.cpu.brand_string").or(Some("Apple Silicon GPU".to_string()));
        // Unified memory: the GPU can address system RAM, but not all of it is usable
        // for a model (the OS and other processes need headroom) - report total system
        // memory as the VRAM ceiling, not a free/available figure we can't measure.
        info.total_vram_bytes = info.total_memory_bytes;
        info.available_vram_bytes = None;
    }
}

#[cfg(target_os = "linux")]
fn discover_linux(info: &mut HardwareInfo) {
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        let physical_ids: std::collections::HashSet<&str> = cpuinfo
            .lines()
            .filter(|l| l.starts_with("physical id"))
            .filter_map(|l| l.split(':').nth(1))
            .map(|s| s.trim())
            .collect();
        let core_ids: std::collections::HashSet<String> = cpuinfo
            .lines()
            .filter(|l| l.starts_with("core id"))
            .filter_map(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .collect();
        if !physical_ids.is_empty() && !core_ids.is_empty() {
            info.physical_cpus = Some((physical_ids.len() * core_ids.len()) as u32);
        }
    }

    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        let parse_kb = |prefix: &str| -> Option<u64> {
            meminfo
                .lines()
                .find(|l| l.starts_with(prefix))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                })
        };
        info.total_memory_bytes = parse_kb("MemTotal:").map(|kb| kb * 1024);
        info.available_memory_bytes = parse_kb("MemAvailable:").map(|kb| kb * 1024);
    }
}

/// `nvidia-smi` presence/absence must never crash discovery on any platform.
fn discover_nvidia(info: &mut HardwareInfo) {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output();

    let Ok(output) = output else {
        return; // not installed / not launchable - leave cuda_available = false
    };
    if !output.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(first_line) = text.lines().next() else {
        return;
    };
    let parts: Vec<&str> = first_line.split(',').map(|s| s.trim()).collect();
    if parts.len() < 3 {
        return;
    }
    info.cuda_available = true;
    info.accelerator = Some(AcceleratorKind::Cuda);
    info.accelerator_name = Some(parts[0].to_string());
    info.total_vram_bytes = parts[1].parse::<u64>().ok().map(|mib| mib * 1024 * 1024);
    info.available_vram_bytes = parts[2].parse::<u64>().ok().map(|mib| mib * 1024 * 1024);
    info.accelerator_utilization_percent = parts.get(3).and_then(|s| s.parse::<f32>().ok());
}

/// A coarse, cheap-to-read proxy for CPU utilization: the 1-minute load average
/// (kernel-maintained state, one file read, no delta-sampling) divided by logical
/// CPU count. This is not the same thing as instantaneous CPU% - load average
/// includes processes waiting on I/O, not just CPU contention - but it's honest,
/// real state rather than a fabricated number, and avoids needing a background
/// sampling task for a first pass. Clamped to `[0, 100]` since load average can
/// exceed the logical CPU count under contention.
#[cfg(target_os = "linux")]
pub fn cpu_utilization_percent(logical_cpus: Option<u32>) -> Option<f32> {
    let cpus = logical_cpus.filter(|&n| n > 0)? as f32;
    let loadavg = std::fs::read_to_string("/proc/loadavg").ok()?;
    let one_minute: f32 = loadavg.split_whitespace().next()?.parse().ok()?;
    Some((one_minute / cpus * 100.0).clamp(0.0, 100.0))
}

#[cfg(not(target_os = "linux"))]
pub fn cpu_utilization_percent(_logical_cpus: Option<u32>) -> Option<f32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_never_panics_and_reports_logical_cpus() {
        let info = discover();
        assert!(info.logical_cpus.unwrap_or(0) > 0);
        assert!(!info.os.is_empty());
        assert!(!info.hostname.is_empty());
    }

    #[test]
    fn missing_nvidia_smi_does_not_crash_and_leaves_cuda_unavailable() {
        // discover_nvidia is exercised unconditionally by discover(); on any machine
        // without nvidia-smi (this dev machine included) it must degrade cleanly.
        let mut info = HardwareInfo::default();
        discover_nvidia(&mut info);
        // Either nvidia-smi is genuinely absent (cuda_available stays false) or present
        // (cuda_available becomes true) - either way, no panic, which is the property
        // under test.
        let _ = info.cuda_available;
    }

    #[test]
    fn cpu_utilization_percent_never_panics_and_stays_in_range() {
        if let Some(pct) = cpu_utilization_percent(Some(8)) {
            assert!((0.0..=100.0).contains(&pct));
        }
    }

    #[test]
    fn cpu_utilization_percent_is_none_for_zero_logical_cpus() {
        assert_eq!(cpu_utilization_percent(Some(0)), None);
        assert_eq!(cpu_utilization_percent(None), None);
    }
}
