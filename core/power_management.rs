//! Power management for macOS - battery detection and thermal optimization

#[cfg(target_os = "macos")]
pub mod power_management {
    use std::process::Command;
    use serde_json::json;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PowerState {
        OnBattery,
        PluggedIn,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub enum ThermalState {
        Normal,    // 0
        Nominal,   // 1
        Fair,      // 2
        Serious,   // 3
        Critical,  // 4
    }

    /// Get current power state (battery or plugged in)
    pub fn get_power_state() -> PowerState {
        let output = Command::new("pmset")
            .arg("-g")
            .arg("batt")
            .output();

        match output {
            Ok(out) => {
                let result = String::from_utf8_lossy(&out.stdout);
                if result.contains("AC Power") {
                    PowerState::PluggedIn
                } else if result.contains("Battery Power") {
                    PowerState::OnBattery
                } else {
                    PowerState::Unknown
                }
            }
            Err(_) => PowerState::Unknown,
        }
    }

    /// Get battery percentage (0-100)
    pub fn get_battery_percentage() -> Option<u32> {
        let output = Command::new("pmset")
            .arg("-g")
            .arg("batt")
            .output()
            .ok()?;

        let result = String::from_utf8(output.stdout).ok()?;
        
        // Parse "123%" from battery output
        result
            .lines()
            .find(|line| line.contains("%"))
            .and_then(|line| {
                line.split('%')
                    .next()
                    .and_then(|s| s.trim().split_whitespace().last())
                    .and_then(|num| num.parse::<u32>().ok())
            })
    }

    /// Get system thermal state (requires TCC privilege on macOS 12+)
    pub fn get_thermal_state() -> ThermalState {
        // Try to get thermal pressure info
        let output = Command::new("sysctl")
            .arg("-n")
            .arg("kern.thermalstatus")
            .output();

        match output {
            Ok(out) => {
                let result = String::from_utf8_lossy(&out.stdout);
                match result.trim().parse::<u32>() {
                    Ok(0) => ThermalState::Normal,
                    Ok(1) => ThermalState::Nominal,
                    Ok(2) => ThermalState::Fair,
                    Ok(3) => ThermalState::Serious,
                    Ok(4) => ThermalState::Critical,
                    _ => ThermalState::Normal,
                }
            }
            Err(_) => ThermalState::Normal,
        }
    }

    /// Get CPU temperature (approximate)
    pub fn get_cpu_temp_celsius() -> Option<f64> {
        let output = Command::new("sysctl")
            .arg("-n")
            .arg("hw.cputemp")
            .output()
            .ok()?;

        let result = String::from_utf8_lossy(&output.stdout);
        result.trim().parse::<f64>().ok()
    }

    /// Get optimized inference config based on power state
    pub fn get_optimized_config(power_state: PowerState, thermal: ThermalState) -> serde_json::Value {
        match (power_state, thermal) {
            // Battery + Normal/Nominal = Battery saver
            (PowerState::OnBattery, ThermalState::Normal) |
            (PowerState::OnBattery, ThermalState::Nominal) => {
                json!({
                    "device": "ane",  // Neural Engine (ultra low power)
                    "use_gpu": false,
                    "max_threads": 4,
                    "batch_size": 1,
                    "optimization": "battery_saver",
                    "expected_power_draw_w": 0.5,
                })
            }

            // Battery + Fair/Serious/Critical = Ultra power saver
            (PowerState::OnBattery, _) => {
                json!({
                    "device": "cpu",  // CPU only
                    "use_gpu": false,
                    "max_threads": 2,
                    "batch_size": 1,
                    "quantization": "int8",
                    "optimization": "ultra_battery_saver",
                    "expected_power_draw_w": 0.2,
                })
            }

            // Plugged in + Normal = High performance
            (PowerState::PluggedIn, ThermalState::Normal) => {
                json!({
                    "device": "metal",
                    "use_gpu": true,
                    "max_threads": 8,
                    "batch_size": 4,
                    "n_gpu_layers": 999,
                    "optimization": "performance",
                    "expected_power_draw_w": 25.0,
                })
            }

            // Plugged in + Nominal = Balanced
            (PowerState::PluggedIn, ThermalState::Nominal) => {
                json!({
                    "device": "metal",
                    "use_gpu": true,
                    "max_threads": 6,
                    "batch_size": 2,
                    "n_gpu_layers": 35,
                    "optimization": "balanced",
                    "expected_power_draw_w": 18.0,
                })
            }

            // Plugged in + Fair/Serious = Thermal throttle mode
            (PowerState::PluggedIn, ThermalState::Fair) |
            (PowerState::PluggedIn, ThermalState::Serious) |
            (PowerState::PluggedIn, ThermalState::Critical) => {
                json!({
                    "device": "cpu",
                    "use_gpu": false,
                    "max_threads": 4,
                    "batch_size": 1,
                    "optimization": "thermal_throttle",
                    "expected_power_draw_w": 8.0,
                })
            }

            // Unknown state = Conservative
            (PowerState::Unknown, _) => {
                json!({
                    "device": "cpu",
                    "use_gpu": false,
                    "max_threads": 4,
                    "batch_size": 1,
                    "optimization": "conservative",
                })
            }
        }
    }

    /// Get estimated time for full inference at current power state
    pub fn estimate_inference_time_seconds(
        model_tokens: u32,
        tokens_per_second: f64,
    ) -> f64 {
        model_tokens as f64 / tokens_per_second
    }

    /// Check if system should defer heavy workloads (low power or thermal throttling)
    pub fn should_defer_workloads() -> bool {
        let power = get_power_state();
        let thermal = get_thermal_state();

        match (power, thermal) {
            (PowerState::OnBattery, _) => true,
            (_, ThermalState::Serious) | (_, ThermalState::Critical) => true,
            _ => false,
        }
    }

    /// Print power status for debugging
    pub fn print_power_status() {
        let power = get_power_state();
        let thermal = get_thermal_state();
        let battery = get_battery_percentage();
        let temp = get_cpu_temp_celsius();

        println!("\n📊 Power Status:");
        println!("  Power State:    {:?}", power);
        if let Some(pct) = battery {
            println!("  Battery:        {}%", pct);
        }
        println!("  Thermal State:  {:?}", thermal);
        if let Some(t) = temp {
            println!("  CPU Temp:       {:.1}°C", t);
        }

        let config = get_optimized_config(power, thermal);
        if let Some(opt) = config.get("optimization") {
            println!("  Optimization:   {}", opt);
        }
        println!();
    }
}

#[cfg(not(target_os = "macos"))]
pub mod power_management {
    use serde_json::json;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PowerState {
        OnBattery,
        PluggedIn,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ThermalState {
        Normal,
        Nominal,
        Fair,
        Serious,
        Critical,
    }

    pub fn get_power_state() -> PowerState {
        PowerState::Unknown
    }

    pub fn get_battery_percentage() -> Option<u32> {
        None
    }

    pub fn get_thermal_state() -> ThermalState {
        ThermalState::Normal
    }

    pub fn get_cpu_temp_celsius() -> Option<f64> {
        None
    }

    pub fn get_optimized_config(_power_state: PowerState, _thermal: ThermalState) -> serde_json::Value {
        json!({
            "device": "cpu",
            "optimization": "default"
        })
    }

    pub fn should_defer_workloads() -> bool {
        false
    }

    pub fn print_power_status() {
        println!("Power management only available on macOS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_power_state_detection() {
        let state = power_management::get_power_state();
        // Should be either battery or plugged in
        assert!(state == power_management::PowerState::OnBattery 
                || state == power_management::PowerState::PluggedIn);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_thermal_state() {
        let thermal = power_management::get_thermal_state();
        // Should always return some valid state
        assert!(thermal <= power_management::ThermalState::Critical);
    }

    #[test]
    fn test_config_generation() {
        let config = power_management::get_optimized_config(
            power_management::PowerState::PluggedIn,
            power_management::ThermalState::Normal,
        );
        
        assert!(config.get("device").is_some());
        assert!(config.get("optimization").is_some());
    }

    #[test]
    fn test_deferred_workload_check() {
        // Should not panic
        let _should_defer = power_management::should_defer_workloads();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_power_status_print() {
        // Should not panic
        power_management::print_power_status();
    }
}
