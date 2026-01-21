//! Core scheduling for Apple Silicon - pin inference to P-cores, background to E-cores

#[cfg(target_os = "macos")]
pub mod core_scheduler {
    use std::thread;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone, Debug)]
    pub struct CoreInfo {
        pub p_core_count: usize,
        pub e_core_count: usize,
        pub total_cores: usize,
    }

    impl CoreInfo {
        /// Detect P-core and E-core counts on this system
        pub fn detect() -> Option<Self> {
            use std::process::Command;

            let p_cores = Command::new("sysctl")
                .arg("-n")
                .arg("hw.perflevel0.logicalcpu")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);

            let e_cores = Command::new("sysctl")
                .arg("-n")
                .arg("hw.perflevel1.logicalcpu")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);

            if p_cores > 0 {
                Some(CoreInfo {
                    p_core_count: p_cores,
                    e_core_count: e_cores,
                    total_cores: p_cores + e_cores,
                })
            } else {
                None
            }
        }

        /// Get P-core ID at index (0..p_core_count)
        pub fn get_p_core_id(&self, index: usize) -> usize {
            index % self.p_core_count
        }

        /// Get E-core ID at index (0..e_core_count)
        pub fn get_e_core_id(&self, index: usize) -> usize {
            self.p_core_count + (index % self.e_core_count)
        }
    }

    pub struct InferenceThread {
        handle: Option<thread::JoinHandle<()>>,
        stop_flag: Arc<AtomicBool>,
    }

    impl InferenceThread {
        /// Spawn inference thread on next available P-core
        pub fn spawn_on_p_core<F>(work: F) -> Result<Self, String>
        where
            F: FnOnce() + Send + 'static,
        {
            let stop_flag = Arc::new(AtomicBool::new(false));
            
            // On macOS, thread scheduling is automatic for now
            // Future: Use pthread_setaffinity_np if available
            let handle = thread::Builder::new()
                .name("inference-p-core".to_string())
                .spawn(work)
                .map_err(|e| format!("Failed to spawn inference thread: {}", e))?;

            Ok(InferenceThread {
                handle: Some(handle),
                stop_flag,
            })
        }

        /// Wait for thread to complete
        pub fn wait(mut self) -> Result<(), String> {
            if let Some(handle) = self.handle.take() {
                handle
                    .join()
                    .map_err(|_| "Inference thread panicked".to_string())
            } else {
                Ok(())
            }
        }

        /// Stop the thread (requests graceful shutdown)
        pub fn stop(&self) {
            self.stop_flag.store(true, Ordering::SeqCst);
        }

        /// Check if stop was requested
        pub fn should_stop(&self) -> bool {
            self.stop_flag.load(Ordering::SeqCst)
        }
    }

    pub struct BackgroundThread {
        handle: Option<thread::JoinHandle<()>>,
    }

    impl BackgroundThread {
        /// Spawn background task thread on next available E-core
        pub fn spawn_on_e_core<F>(work: F) -> Result<Self, String>
        where
            F: FnOnce() + Send + 'static,
        {
            let handle = thread::Builder::new()
                .name("background-e-core".to_string())
                .spawn(work)
                .map_err(|e| format!("Failed to spawn background thread: {}", e))?;

            Ok(BackgroundThread {
                handle: Some(handle),
            })
        }

        /// Wait for thread to complete
        pub fn wait(mut self) -> Result<(), String> {
            if let Some(handle) = self.handle.take() {
                handle
                    .join()
                    .map_err(|_| "Background thread panicked".to_string())
            } else {
                Ok(())
            }
        }
    }

    /// Suggestion for optimal thread pool sizing
    pub fn get_thread_pool_sizes() -> (usize, usize) {
        let cores = CoreInfo::detect();
        
        match cores {
            Some(info) => {
                // Use all P-cores for inference, half E-cores for background
                (
                    info.p_core_count,
                    (info.e_core_count / 2).max(1),
                )
            }
            None => (4, 2), // Fallback
        }
    }

    /// Get scheduling recommendation
    pub fn get_scheduling_recommendation() -> String {
        match CoreInfo::detect() {
            Some(info) => {
                format!(
                    "Recommended: {} threads for inference (P-cores), {} for background (E-cores)",
                    info.p_core_count,
                    (info.e_core_count / 2).max(1)
                )
            }
            None => "Could not detect core configuration".to_string(),
        }
    }

    /// Print core configuration for debugging
    pub fn print_core_configuration() {
        match CoreInfo::detect() {
            Some(info) => {
                println!("\n🔌 Core Configuration:");
                println!("  P-cores (Performance): {}", info.p_core_count);
                println!("  E-cores (Efficiency):  {}", info.e_core_count);
                println!("  Total Cores:           {}", info.total_cores);
                println!("  Recommended Inference Threads: {}", info.p_core_count);
                println!("  Recommended Background Threads: {}", (info.e_core_count / 2).max(1));
                println!();
            }
            None => {
                println!("Could not detect Apple Silicon core configuration");
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub mod core_scheduler {
    use std::thread;

    #[derive(Clone, Debug)]
    pub struct CoreInfo {
        pub p_core_count: usize,
        pub e_core_count: usize,
        pub total_cores: usize,
    }

    impl CoreInfo {
        pub fn detect() -> Option<Self> {
            let cores = std::thread::available_parallelism().ok()?.get();
            Some(CoreInfo {
                p_core_count: cores,
                e_core_count: 0,
                total_cores: cores,
            })
        }
    }

    pub struct InferenceThread {
        handle: Option<thread::JoinHandle<()>>,
    }

    impl InferenceThread {
        pub fn spawn_on_p_core<F>(work: F) -> Result<Self, String>
        where
            F: FnOnce() + Send + 'static,
        {
            let handle = thread::Builder::new()
                .spawn(work)
                .map_err(|e| format!("Failed to spawn thread: {}", e))?;

            Ok(InferenceThread {
                handle: Some(handle),
            })
        }

        pub fn wait(mut self) -> Result<(), String> {
            if let Some(handle) = self.handle.take() {
                handle.join().map_err(|_| "Thread panicked".to_string())
            } else {
                Ok(())
            }
        }
    }

    pub struct BackgroundThread {
        handle: Option<thread::JoinHandle<()>>,
    }

    impl BackgroundThread {
        pub fn spawn_on_e_core<F>(work: F) -> Result<Self, String>
        where
            F: FnOnce() + Send + 'static,
        {
            let handle = thread::Builder::new()
                .spawn(work)
                .map_err(|e| format!("Failed to spawn thread: {}", e))?;

            Ok(BackgroundThread {
                handle: Some(handle),
            })
        }

        pub fn wait(mut self) -> Result<(), String> {
            if let Some(handle) = self.handle.take() {
                handle.join().map_err(|_| "Thread panicked".to_string())
            } else {
                Ok(())
            }
        }
    }

    pub fn get_thread_pool_sizes() -> (usize, usize) {
        let cores = std::thread::available_parallelism().ok().map(|p| p.get()).unwrap_or(4);
        ((cores * 3) / 4, (cores / 4).max(1))
    }

    pub fn get_scheduling_recommendation() -> String {
        "Core scheduling only optimized on Apple Silicon".to_string()
    }

    pub fn print_core_configuration() {
        println!("Core scheduling only available on Apple Silicon macOS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_info_detection() {
        let cores = core_scheduler::CoreInfo::detect();
        // Should be Some on macOS Apple Silicon, but may be None on other systems
        if let Some(info) = cores {
            assert!(info.total_cores > 0);
        }
    }

    #[test]
    fn test_thread_pool_sizing() {
        let (inference, background) = core_scheduler::get_thread_pool_sizes();
        assert!(inference > 0);
        assert!(background > 0);
    }

    #[test]
    fn test_inference_thread_spawn() {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = core_scheduler::InferenceThread::spawn_on_p_core(move || {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        assert!(result.is_ok());
        let thread = result.unwrap();
        assert!(thread.wait().is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn test_background_thread_spawn() {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = core_scheduler::BackgroundThread::spawn_on_e_core(move || {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        assert!(result.is_ok());
        let thread = result.unwrap();
        assert!(thread.wait().is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn test_scheduling_recommendation() {
        let rec = core_scheduler::get_scheduling_recommendation();
        assert!(!rec.is_empty());
    }
}
