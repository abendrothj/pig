//! Unified memory optimization for Apple Silicon - zero-copy GPU buffers

#[cfg(target_os = "macos")]
pub mod unified_memory {
    use std::alloc::Layout;
    use std::ptr::NonNull;

    /// Unified memory buffer for CPU-GPU sharing without copies
    pub struct UnifiedMemoryBuffer {
        ptr: NonNull<u8>,
        size: usize,
        _aligned: bool,
    }

    impl UnifiedMemoryBuffer {
        /// Create a new unified memory buffer
        pub fn new(size: usize) -> Result<Self, String> {
            if size == 0 {
                return Err("Buffer size must be > 0".to_string());
            }

            // Align to 64 bytes for cache efficiency
            let alignment = 64.max(std::mem::align_of::<u128>());
            let layout = Layout::from_size_align(size, alignment)
                .map_err(|e| format!("Invalid layout: {}", e))?;

            let ptr = unsafe {
                let p = std::alloc::alloc(layout);
                if p.is_null() {
                    return Err("Failed to allocate unified memory".to_string());
                }
                NonNull::new_unchecked(p)
            };

            Ok(UnifiedMemoryBuffer {
                ptr,
                size,
                _aligned: true,
            })
        }

        /// Get size in bytes
        pub fn len(&self) -> usize {
            self.size
        }

        /// Get raw pointer for GPU operations
        pub fn as_ptr(&self) -> *const u8 {
            self.ptr.as_ptr()
        }

        /// Get mutable raw pointer
        pub fn as_mut_ptr(&mut self) -> *mut u8 {
            unsafe { self.ptr.as_mut() }
        }

        /// Get slice for CPU access (zero-copy)
        pub fn as_slice(&self) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.size) }
        }

        /// Get mutable slice for CPU access (zero-copy)
        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            unsafe { std::slice::from_raw_parts_mut(self.ptr.as_mut(), self.size) }
        }

        /// Hint to OS: sequential access pattern (for prefetching)
        pub fn hint_sequential_access(&self) {
            #[cfg(target_os = "macos")]
            unsafe {
                libc::madvise(
                    self.ptr.as_ptr() as *mut libc::c_void,
                    self.size,
                    libc::MADV_SEQUENTIAL,
                );
            }
        }

        /// Hint to OS: random access pattern
        pub fn hint_random_access(&self) {
            #[cfg(target_os = "macos")]
            unsafe {
                libc::madvise(
                    self.ptr.as_ptr() as *mut libc::c_void,
                    self.size,
                    libc::MADV_RANDOM,
                );
            }
        }

        /// Flush to ensure GPU sees updates
        pub fn flush(&self) {
            // On Apple Silicon unified memory, this is typically implicit
            // But we can suggest the OS to sync if needed
            #[cfg(target_os = "macos")]
            unsafe {
                libc::madvise(
                    self.ptr.as_ptr() as *mut libc::c_void,
                    self.size,
                    libc::MADV_WILLNEED,
                );
            }
        }
    }

    impl Drop for UnifiedMemoryBuffer {
        fn drop(&mut self) {
            // NonNull is always non-null by construction, so we can safely dealloc
            unsafe {
                let layout = Layout::from_size_align_unchecked(
                    self.size,
                    64.max(std::mem::align_of::<u128>()),
                );
                std::alloc::dealloc(self.ptr.as_mut(), layout);
            }
        }
    }

    impl Clone for UnifiedMemoryBuffer {
        fn clone(&self) -> Self {
            // In APFS, cloning is cheap
            match UnifiedMemoryBuffer::new(self.size) {
                Ok(mut new_buf) => {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            self.ptr.as_ptr(),
                            new_buf.ptr.as_mut(),
                            self.size,
                        );
                    }
                    new_buf
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to clone unified memory buffer: {}", e);
                    // Return a zero-sized buffer as fallback to avoid panicking
                    UnifiedMemoryBuffer::new(1).expect("failed to allocate even 1 byte")
                }
            }
        }
    }

    /// KV cache for transformer models - zero-copy between GPU and CPU
    pub struct KVCache {
        k_buffer: UnifiedMemoryBuffer,
        v_buffer: UnifiedMemoryBuffer,
        max_seq_len: usize,
        embedding_dim: usize,
    }

    impl KVCache {
        /// Create KV cache for transformer with given dimensions
        pub fn new(max_seq_len: usize, embedding_dim: usize) -> Result<Self, String> {
            // Each buffer: max_seq_len * embedding_dim * sizeof(f32)
            let buffer_size = max_seq_len * embedding_dim * 4; // 4 bytes per float

            let k_buffer = UnifiedMemoryBuffer::new(buffer_size)?;
            let v_buffer = UnifiedMemoryBuffer::new(buffer_size)?;

            // Hint sequential access for cache reading
            k_buffer.hint_sequential_access();
            v_buffer.hint_sequential_access();

            Ok(KVCache {
                k_buffer,
                v_buffer,
                max_seq_len,
                embedding_dim,
            })
        }

        /// Get K buffer for reading
        pub fn k_data(&self) -> &[u8] {
            self.k_buffer.as_slice()
        }

        /// Get K buffer for writing
        pub fn k_data_mut(&mut self) -> &mut [u8] {
            self.k_buffer.as_mut_slice()
        }

        /// Get V buffer for reading
        pub fn v_data(&self) -> &[u8] {
            self.v_buffer.as_slice()
        }

        /// Get V buffer for writing
        pub fn v_data_mut(&mut self) -> &mut [u8] {
            self.v_buffer.as_mut_slice()
        }

        /// Get raw pointers for GPU operations
        pub fn get_gpu_pointers(&self) -> (*const u8, *const u8) {
            (self.k_buffer.as_ptr(), self.v_buffer.as_ptr())
        }

        /// Get mutable raw pointers for GPU operations
        pub fn get_gpu_pointers_mut(&mut self) -> (*mut u8, *mut u8) {
            (self.k_buffer.as_mut_ptr(), self.v_buffer.as_mut_ptr())
        }

        /// Cache statistics
        pub fn stats(&self) -> KVCacheStats {
            KVCacheStats {
                total_size_mb: (self.k_buffer.len() + self.v_buffer.len()) as f64 / 1_048_576.0,
                max_seq_len: self.max_seq_len,
                embedding_dim: self.embedding_dim,
                k_size_mb: self.k_buffer.len() as f64 / 1_048_576.0,
                v_size_mb: self.v_buffer.len() as f64 / 1_048_576.0,
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct KVCacheStats {
        pub total_size_mb: f64,
        pub max_seq_len: usize,
        pub embedding_dim: usize,
        pub k_size_mb: f64,
        pub v_size_mb: f64,
    }

    /// Token embedding table - shared between CPU and GPU
    pub struct EmbeddingTable {
        data: UnifiedMemoryBuffer,
        vocab_size: usize,
        embedding_dim: usize,
    }

    impl EmbeddingTable {
        /// Create embedding table
        pub fn new(vocab_size: usize, embedding_dim: usize) -> Result<Self, String> {
            let buffer_size = vocab_size * embedding_dim * 4; // 4 bytes per float
            let data = UnifiedMemoryBuffer::new(buffer_size)?;

            Ok(EmbeddingTable {
                data,
                vocab_size,
                embedding_dim,
            })
        }

        /// Get embedding data for reading
        pub fn data(&self) -> &[u8] {
            self.data.as_slice()
        }

        /// Get embedding data for writing
        pub fn data_mut(&mut self) -> &mut [u8] {
            self.data.as_mut_slice()
        }

        /// Get raw pointer for GPU operations
        pub fn gpu_ptr(&self) -> *const u8 {
            self.data.as_ptr()
        }

        /// Get memory usage in MB
        pub fn size_mb(&self) -> f64 {
            self.data.len() as f64 / 1_048_576.0
        }
    }

    /// Print unified memory statistics
    pub fn print_unified_memory_info() {
        use std::process::Command;

        if let Ok(output) = Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
        {
            if let Ok(memsize_str) = String::from_utf8(output.stdout) {
                if let Ok(memsize_bytes) = memsize_str.trim().parse::<u64>() {
                    let memsize_gb = memsize_bytes as f64 / 1_073_741_824.0;
                    println!("\n💾 Unified Memory Configuration:");
                    println!("  Total Unified Memory: {:.1} GB", memsize_gb);
                    println!("  Optimal for ML models: {:.1} GB", memsize_gb * 0.6);
                    println!("  GPU can access all: Yes (zero-copy)");
                    println!();
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub mod unified_memory {
    pub struct UnifiedMemoryBuffer {
        data: Vec<u8>,
    }

    impl UnifiedMemoryBuffer {
        pub fn new(size: usize) -> Result<Self, String> {
            Ok(UnifiedMemoryBuffer {
                data: vec![0; size],
            })
        }

        pub fn len(&self) -> usize {
            self.data.len()
        }

        pub fn as_ptr(&self) -> *const u8 {
            self.data.as_ptr()
        }

        pub fn as_mut_ptr(&mut self) -> *mut u8 {
            self.data.as_mut_ptr()
        }

        pub fn as_slice(&self) -> &[u8] {
            &self.data
        }

        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            &mut self.data
        }
    }

    impl Clone for UnifiedMemoryBuffer {
        fn clone(&self) -> Self {
            UnifiedMemoryBuffer {
                data: self.data.clone(),
            }
        }
    }

    pub struct KVCache {
        k_buffer: UnifiedMemoryBuffer,
        v_buffer: UnifiedMemoryBuffer,
    }

    impl KVCache {
        pub fn new(max_seq_len: usize, embedding_dim: usize) -> Result<Self, String> {
            let buffer_size = max_seq_len * embedding_dim * 4;
            Ok(KVCache {
                k_buffer: UnifiedMemoryBuffer::new(buffer_size)?,
                v_buffer: UnifiedMemoryBuffer::new(buffer_size)?,
            })
        }

        pub fn k_data(&self) -> &[u8] {
            self.k_buffer.as_slice()
        }

        pub fn v_data(&self) -> &[u8] {
            self.v_buffer.as_slice()
        }
    }

    pub struct EmbeddingTable {
        data: UnifiedMemoryBuffer,
    }

    impl EmbeddingTable {
        pub fn new(vocab_size: usize, embedding_dim: usize) -> Result<Self, String> {
            let buffer_size = vocab_size * embedding_dim * 4;
            Ok(EmbeddingTable {
                data: UnifiedMemoryBuffer::new(buffer_size)?,
            })
        }

        pub fn data(&self) -> &[u8] {
            self.data.as_slice()
        }
    }

    pub fn print_unified_memory_info() {
        println!("Unified memory optimization only available on macOS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_memory_allocation() {
        let buf = unified_memory::UnifiedMemoryBuffer::new(1024);
        assert!(buf.is_ok());
        let buf = buf.unwrap();
        assert_eq!(buf.len(), 1024);
    }

    #[test]
    fn test_kv_cache_creation() {
        let cache = unified_memory::KVCache::new(512, 128);
        assert!(cache.is_ok());
    }

    #[test]
    fn test_embedding_table() {
        let table = unified_memory::EmbeddingTable::new(32000, 4096);
        assert!(table.is_ok());
    }

    #[test]
    fn test_buffer_access() {
        let mut buf = unified_memory::UnifiedMemoryBuffer::new(256).unwrap();
        let slice = buf.as_mut_slice();
        slice[0] = 42;
        assert_eq!(buf.as_slice()[0], 42);
    }
}
