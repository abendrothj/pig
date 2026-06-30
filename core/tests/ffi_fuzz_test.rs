//! Custom FFI Fuzzer for LAO Plugin System
//!
//! Tests the FFI boundary with structured adversarial inputs to detect:
//! - Null pointer dereferences
//! - Buffer overflows/overruns
//! - Panics from malformed data
//! - Invalid UTF-8 handling
//! - Malformed JSON parsing
//! - Memory lifecycle issues
//!
//! Run modes:
//!   cargo test --test ffi_fuzz_test                          # Standard
//!   cargo +nightly miri test --test ffi_fuzz_test            # Miri (pure-Rust harnesses only)
//!   RUSTFLAGS="-Zsanitizer=address" cargo +nightly test \
//!     --test ffi_fuzz_test --target aarch64-apple-darwin     # ASan (all harnesses incl. FFI)
//!
//! Reproducibility: set LAO_FUZZ_SEED=<seed> to replay a specific run.

#[cfg(not(miri))]
use lao_orchestrator_core::cross_platform::PathUtils;
#[cfg(not(miri))]
use lao_orchestrator_core::plugins::PluginRegistry;
use lao_plugin_api::*;
#[cfg(not(miri))]
use serial_test::serial;
use std::ffi::{c_char, CStr, CString};
use std::panic::{self, AssertUnwindSafe};

// ---------------------------------------------------------------------------
// PRNG — xorshift64, zero dependencies
// ---------------------------------------------------------------------------

struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn next_usize(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        (self.next_u64() as usize) % max
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn next_byte(&mut self) -> u8 {
        self.next_u64() as u8
    }

    fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next_usize(items.len())]
    }
}

fn get_seed() -> u64 {
    if let Ok(s) = std::env::var("LAO_FUZZ_SEED") {
        return s.parse().unwrap_or(42);
    }
    // Under Miri, SystemTime::now() is unavailable without -Zmiri-disable-isolation.
    // Fall back to a fixed seed so Miri runs are deterministic.
    #[cfg(miri)]
    {
        42
    }
    #[cfg(not(miri))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42)
    }
}

// ---------------------------------------------------------------------------
// Result tracking
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum FuzzResult {
    Ok,
    Panic(String),
    ValidationFailure(String),
}

#[derive(Default)]
struct FuzzStats {
    total: usize,
    ok: usize,
    panics: usize,
    validation_failures: usize,
    failure_details: Vec<String>,
}

impl FuzzStats {
    fn record(&mut self, result: FuzzResult) {
        self.total += 1;
        match result {
            FuzzResult::Ok => self.ok += 1,
            FuzzResult::Panic(msg) => {
                self.panics += 1;
                if self.failure_details.len() < 10 {
                    self.failure_details.push(format!("PANIC: {}", msg));
                }
            }
            FuzzResult::ValidationFailure(msg) => {
                self.validation_failures += 1;
                if self.failure_details.len() < 10 {
                    self.failure_details.push(format!("VALIDATION: {}", msg));
                }
            }
        }
    }

    fn print_summary(&self, target: &str) {
        println!("=== Fuzz Summary: {} ===", target);
        println!("  Total iterations: {}", self.total);
        println!("  OK: {}", self.ok);
        println!("  Panics: {}", self.panics);
        println!("  Validation failures: {}", self.validation_failures);
        if !self.failure_details.is_empty() {
            println!("  Failure details:");
            for detail in &self.failure_details {
                println!("    - {}", detail);
            }
        }
    }

    fn has_failures(&self) -> bool {
        self.panics > 0 || self.validation_failures > 0
    }
}

// ---------------------------------------------------------------------------
// Generators — structured adversarial inputs
// ---------------------------------------------------------------------------

/// Holds owned CStrings so pointers remain valid for the duration of a test iteration.
struct OwnedStrings {
    strings: Vec<CString>,
    binary: Vec<Vec<u8>>,
}

impl OwnedStrings {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            binary: Vec::new(),
        }
    }

    /// Store a CString and return its raw pointer.
    fn store(&mut self, s: CString) -> *mut c_char {
        let ptr = s.into_raw();
        // We'll reconstruct it for cleanup in Drop
        self.strings.push(unsafe { CString::from_raw(ptr) });
        self.strings.last().unwrap().as_ptr() as *mut c_char
    }

    /// Store a CString and return a const pointer.
    fn store_const(&mut self, s: CString) -> *const c_char {
        self.store(s) as *const c_char
    }

    /// Store binary data and return a pointer + size.
    fn store_binary(&mut self, data: Vec<u8>) -> (*mut u8, usize) {
        let size = data.len();
        self.binary.push(data);
        let last = self.binary.last_mut().unwrap();
        (last.as_mut_ptr(), size)
    }
}

/// Generate a CString with various adversarial characteristics.
/// Returns None to signal "use a null pointer".
fn gen_c_string(rng: &mut FuzzRng) -> Option<CString> {
    let category = rng.next_usize(9);
    match category {
        // Null — caller should use null pointer
        0 => None,
        // Empty string
        1 => Some(CString::new("").unwrap()),
        // Short ASCII
        2 => {
            let len = 1 + rng.next_usize(64);
            let s: Vec<u8> = (0..len).map(|_| 32 + (rng.next_byte() % 95)).collect();
            Some(CString::new(s).unwrap())
        }
        // Long string (64KB)
        3 => {
            let len = 65536;
            let s = vec![b'A'; len];
            Some(CString::new(s).unwrap())
        }
        // Invalid UTF-8 bytes (but valid C string — no interior nulls)
        4 => {
            let len = 4 + rng.next_usize(32);
            let mut bytes: Vec<u8> = Vec::with_capacity(len);
            for _ in 0..len {
                let b = 0x80 + (rng.next_byte() % 0x7F);
                // Avoid null bytes (CString rejects them)
                let b = if b == 0 { 0x80 } else { b };
                bytes.push(b);
            }
            Some(CString::new(bytes).unwrap())
        }
        // Control characters
        5 => {
            let s = "hello\tworld\nfoo\rbar";
            Some(CString::new(s).unwrap())
        }
        // JSON-like strings (to exercise JSON parsing paths)
        6 => {
            let choices = [
                r#"{"key": "value"}"#,
                r#"[1, 2, 3]"#,
                r#"{{{invalid"#,
                r#"["unclosed"#,
                r#"null"#,
                r#"{"deeply": {"nested": {"object": true}}}"#,
            ];
            Some(CString::new(*rng.choose(&choices)).unwrap())
        }
        // Plugin-specific rejection patterns (EchoPlugin rejects these)
        7 => {
            let choices = ["not: allowed", "{ braces }", "} close", "not:"];
            Some(CString::new(*rng.choose(&choices)).unwrap())
        }
        // Very long string (256KB)
        8 => {
            let len = 262144;
            let s: Vec<u8> = (0..len).map(|i| 32 + ((i as u8) % 95)).collect();
            Some(CString::new(s).unwrap())
        }
        _ => unreachable!(),
    }
}

/// Generate a PluginInput with adversarial text field.
#[cfg(not(miri))]
fn gen_plugin_input(rng: &mut FuzzRng, owned: &mut OwnedStrings) -> PluginInput {
    match gen_c_string(rng) {
        Some(s) => PluginInput {
            text: owned.store(s),
        },
        None => PluginInput {
            text: std::ptr::null_mut(),
        },
    }
}

/// Generate a MultiModalInput with adversarial fields.
fn gen_multimodal_input(rng: &mut FuzzRng, owned: &mut OwnedStrings) -> MultiModalInput {
    // input_type: valid range is 0-7, also test invalid discriminants
    let input_type = *rng.choose(&[0u32, 1, 2, 3, 4, 5, 6, 7, 8, 255, u32::MAX]);

    let text_data = match gen_c_string(rng) {
        Some(s) => owned.store(s),
        None => std::ptr::null_mut(),
    };

    let file_path = {
        let choices: &[&str] = &[
            "",
            "/tmp/test.wav",
            "../../../etc/passwd",
            "/nonexistent/path/file.txt",
        ];
        let path_str = *rng.choose(choices);
        if rng.next_bool() {
            owned.store(CString::new(path_str).unwrap())
        } else {
            std::ptr::null_mut()
        }
    };

    let (binary_data, binary_size) = if rng.next_bool() {
        let actual_size = rng.next_usize(1024);
        let data: Vec<u8> = (0..actual_size).map(|_| rng.next_byte()).collect();
        let (ptr, real_size) = owned.store_binary(data);

        // Sometimes lie about the size
        let reported_size = if rng.next_usize(4) == 0 {
            // Mismatched: report larger than actual
            real_size + 100
        } else {
            real_size
        };
        (ptr, reported_size)
    } else {
        (std::ptr::null_mut(), 0)
    };

    let metadata = match gen_c_string(rng) {
        Some(s) => owned.store(s),
        None => std::ptr::null_mut(),
    };

    MultiModalInput {
        input_type,
        text_data,
        file_path,
        binary_data,
        binary_size,
        metadata,
    }
}

/// Generate adversarial JSON strings for metadata fields.
fn gen_json_string(rng: &mut FuzzRng) -> Option<CString> {
    let category = rng.next_usize(10);
    let s = match category {
        0 => return None, // null pointer
        1 => "".to_string(),
        2 => "[]".to_string(),
        // Valid dependency JSON
        3 => r#"[{"name":"dep1","version":"1.0","optional":false}]"#.to_string(),
        // Wrong schema type
        4 => "123".to_string(),
        5 => r#""just_a_string""#.to_string(),
        // Malformed JSON
        6 => "{{{".to_string(),
        7 => r#"[{"unclosed": true"#.to_string(),
        // Deeply nested JSON (100 levels — enough to stress without stack overflow)
        8 => {
            let mut s = String::new();
            for _ in 0..100 {
                s.push_str(r#"{"a":"#);
            }
            s.push_str("1");
            for _ in 0..100 {
                s.push('}');
            }
            s
        }
        // Large array
        9 => {
            let mut s = String::from("[");
            for i in 0..1000 {
                if i > 0 {
                    s.push(',');
                }
                s.push('0');
            }
            s.push(']');
            s
        }
        _ => unreachable!(),
    };
    Some(CString::new(s).unwrap())
}

/// Generate a PluginMetadata with adversarial fields.
fn gen_plugin_metadata(rng: &mut FuzzRng, owned: &mut OwnedStrings) -> PluginMetadata {
    let field = |rng: &mut FuzzRng, owned: &mut OwnedStrings| -> *const c_char {
        match gen_c_string(rng) {
            Some(s) => owned.store_const(s),
            None => std::ptr::null(),
        }
    };

    let json_field = |rng: &mut FuzzRng, owned: &mut OwnedStrings| -> *const c_char {
        match gen_json_string(rng) {
            Some(s) => owned.store_const(s),
            None => std::ptr::null(),
        }
    };

    PluginMetadata {
        name: field(rng, owned),
        version: field(rng, owned),
        description: field(rng, owned),
        author: field(rng, owned),
        dependencies: json_field(rng, owned),
        tags: json_field(rng, owned),
        input_schema: json_field(rng, owned),
        output_schema: json_field(rng, owned),
        capabilities: json_field(rng, owned),
    }
}

// ---------------------------------------------------------------------------
// Dummy VTable functions for synthetic testing
// ---------------------------------------------------------------------------

unsafe extern "C" fn dummy_name() -> *const c_char {
    std::ptr::null()
}

unsafe extern "C" fn dummy_run(_: *const PluginInput) -> PluginOutput {
    PluginOutput {
        text: std::ptr::null_mut(),
    }
}

unsafe extern "C" fn dummy_free_output(_: PluginOutput) {}

unsafe extern "C" fn dummy_run_with_buffer(
    _: *const PluginInput,
    _: *mut c_char,
    _: usize,
) -> usize {
    0
}

unsafe extern "C" fn dummy_validate_input(_: *const PluginInput) -> bool {
    true
}

unsafe extern "C" fn dummy_get_capabilities() -> *const c_char {
    std::ptr::null()
}

unsafe extern "C" fn dummy_run_structured(_: *const PluginInput) -> PluginResult {
    PluginResult {
        status: LAO_STATUS_RUNTIME_ERROR,
        text: std::ptr::null_mut(),
    }
}

unsafe extern "C" fn dummy_free_result(_: PluginResult) {}

// Adversarial get_metadata: returns all-null fields
unsafe extern "C" fn adversarial_get_metadata_all_null() -> PluginMetadata {
    PluginMetadata {
        name: std::ptr::null(),
        version: std::ptr::null(),
        description: std::ptr::null(),
        author: std::ptr::null(),
        dependencies: std::ptr::null(),
        tags: std::ptr::null(),
        input_schema: std::ptr::null(),
        output_schema: std::ptr::null(),
        capabilities: std::ptr::null(),
    }
}

// Adversarial get_metadata: returns invalid UTF-8 in name
unsafe extern "C" fn adversarial_get_metadata_bad_utf8() -> PluginMetadata {
    static BAD_NAME: &[u8] = &[0x80, 0x81, 0xFE, 0xFF, 0x00]; // invalid UTF-8 + null terminator
    PluginMetadata {
        name: BAD_NAME.as_ptr() as *const c_char,
        version: std::ptr::null(),
        description: std::ptr::null(),
        author: std::ptr::null(),
        dependencies: std::ptr::null(),
        tags: std::ptr::null(),
        input_schema: std::ptr::null(),
        output_schema: std::ptr::null(),
        capabilities: std::ptr::null(),
    }
}

// Adversarial get_metadata: returns malformed JSON in dependencies
unsafe extern "C" fn adversarial_get_metadata_bad_json() -> PluginMetadata {
    static NAME: &[u8] = b"BadPlugin\0";
    static VERSION: &[u8] = b"0.0.1\0";
    static BAD_DEPS: &[u8] = b"{{{not json at all\0";
    static BAD_TAGS: &[u8] = b"123\0"; // wrong type (number instead of array)
    static BAD_CAPS: &[u8] = b"[{\"wrong_field\": true}]\0";
    PluginMetadata {
        name: NAME.as_ptr() as *const c_char,
        version: VERSION.as_ptr() as *const c_char,
        description: std::ptr::null(),
        author: std::ptr::null(),
        dependencies: BAD_DEPS.as_ptr() as *const c_char,
        tags: BAD_TAGS.as_ptr() as *const c_char,
        input_schema: std::ptr::null(),
        output_schema: std::ptr::null(),
        capabilities: BAD_CAPS.as_ptr() as *const c_char,
    }
}

// ---------------------------------------------------------------------------
// Fuzz harnesses
// ---------------------------------------------------------------------------

/// Harness 1: Fuzz PluginInfo::from_metadata with adversarial PluginMetadata.
fn fuzz_plugin_info_from_metadata(rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();
    let metadata = gen_plugin_metadata(rng, &mut owned);

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let info = PluginInfo::from_metadata(&metadata);
        // Access all fields to verify no deferred panics
        let _ = info.name.len();
        let _ = info.version.len();
        let _ = info.description.len();
        let _ = info.author.len();
        let _ = info.dependencies.len();
        let _ = info.tags.len();
        let _ = info.capabilities.len();
        let _ = info.input_schema.as_ref().map(|s| s.len());
        let _ = info.output_schema.as_ref().map(|s| s.len());
    }));

    match result {
        Ok(()) => FuzzResult::Ok,
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 2: Fuzz the `run` function through a loaded plugin's VTable.
#[cfg(not(miri))]
fn fuzz_plugin_run(vtable: &PluginVTable, rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();
    let input = gen_plugin_input(rng, &mut owned);

    let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        let output = (vtable.run)(&input);
        // Validate output
        if !output.text.is_null() {
            let c_str = CStr::from_ptr(output.text);
            let _ = c_str.to_string_lossy();
        }
        (vtable.free_output)(output);
    }));

    match result {
        Ok(()) => FuzzResult::Ok,
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 3: Fuzz validate_input through a loaded plugin's VTable.
#[cfg(not(miri))]
fn fuzz_plugin_validate_input(vtable: &PluginVTable, rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();
    let input = gen_plugin_input(rng, &mut owned);

    let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        let valid = (vtable.validate_input)(&input);
        let _ = valid; // just ensure no crash
    }));

    match result {
        Ok(()) => FuzzResult::Ok,
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 4: Fuzz run_with_buffer for buffer overflow detection.
#[cfg(not(miri))]
fn fuzz_plugin_run_with_buffer(vtable: &PluginVTable, rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();
    let input = gen_plugin_input(rng, &mut owned);

    let buf_size = *rng.choose(&[0usize, 1, 2, 16, 256, 4096]);
    let sentinel: u8 = 0xAA;

    let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        if buf_size == 0 {
            // Test with null buffer and zero size
            let written = (vtable.run_with_buffer)(&input, std::ptr::null_mut(), 0);
            if written != 0 {
                return Err(format!(
                    "run_with_buffer returned {} for null buffer",
                    written
                ));
            }
            return Ok(());
        }

        // Allocate buffer with sentinel padding to detect overruns
        let total_alloc = buf_size + 16; // 16 bytes of sentinel after buffer
        let mut buffer: Vec<u8> = vec![sentinel; total_alloc];

        let buf_ptr = buffer.as_mut_ptr() as *mut c_char;
        let written = (vtable.run_with_buffer)(&input, buf_ptr, buf_size);

        // Check: written should be < buf_size (room for null terminator)
        if written >= buf_size {
            return Err(format!(
                "run_with_buffer wrote {} bytes into buffer of size {} (overflow!)",
                written, buf_size
            ));
        }

        // Check: null terminator should be present (only when data was written)
        // When written == 0 the plugin may return early without touching the buffer
        if written > 0 && buffer[written] != 0 {
            return Err(format!(
                "Missing null terminator at position {} (found 0x{:02X})",
                written, buffer[written]
            ));
        }

        // Check: sentinel bytes after buf_size should be untouched
        for i in buf_size..total_alloc {
            if buffer[i] != sentinel {
                return Err(format!(
                    "Buffer overrun detected: byte at offset {} changed from 0x{:02X} to 0x{:02X}",
                    i, sentinel, buffer[i]
                ));
            }
        }

        Ok(())
    }));

    match result {
        Ok(Ok(())) => FuzzResult::Ok,
        Ok(Err(msg)) => FuzzResult::ValidationFailure(msg),
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 5: Fuzz adversarial JSON in metadata fields.
fn fuzz_metadata_json_parsing(rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();

    // Generate metadata with specifically adversarial JSON fields
    let name_s = CString::new("FuzzPlugin").unwrap();
    let version_s = CString::new("0.0.0").unwrap();

    let deps_json = gen_json_string(rng);
    let tags_json = gen_json_string(rng);
    let caps_json = gen_json_string(rng);
    let input_schema_json = gen_json_string(rng);
    let output_schema_json = gen_json_string(rng);

    let metadata = PluginMetadata {
        name: owned.store_const(name_s),
        version: owned.store_const(version_s),
        description: std::ptr::null(),
        author: std::ptr::null(),
        dependencies: deps_json
            .map(|s| owned.store_const(s))
            .unwrap_or(std::ptr::null()),
        tags: tags_json
            .map(|s| owned.store_const(s))
            .unwrap_or(std::ptr::null()),
        input_schema: input_schema_json
            .map(|s| owned.store_const(s))
            .unwrap_or(std::ptr::null()),
        output_schema: output_schema_json
            .map(|s| owned.store_const(s))
            .unwrap_or(std::ptr::null()),
        capabilities: caps_json
            .map(|s| owned.store_const(s))
            .unwrap_or(std::ptr::null()),
    };

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let info = PluginInfo::from_metadata(&metadata);
        // Verify no panics when accessing parsed fields
        let _ = info.dependencies.len();
        let _ = info.tags.len();
        let _ = info.capabilities.len();
        let _ = info.input_schema;
        let _ = info.output_schema;
    }));

    match result {
        Ok(()) => FuzzResult::Ok,
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 6: Test synthetic vtables with adversarial get_metadata implementations.
fn fuzz_synthetic_vtable_metadata() -> Vec<FuzzResult> {
    let adversarial_fns: Vec<(&str, unsafe extern "C" fn() -> PluginMetadata)> = vec![
        ("all_null", adversarial_get_metadata_all_null),
        ("bad_utf8", adversarial_get_metadata_bad_utf8),
        ("bad_json", adversarial_get_metadata_bad_json),
    ];

    let mut results = Vec::new();

    for (label, get_meta_fn) in &adversarial_fns {
        let vtable = PluginVTable {
            version: 2,
            name: dummy_name,
            run: dummy_run,
            free_output: dummy_free_output,
            run_with_buffer: dummy_run_with_buffer,
            get_metadata: *get_meta_fn,
            validate_input: dummy_validate_input,
            get_capabilities: dummy_get_capabilities,
            run_structured: dummy_run_structured,
            free_result: dummy_free_result,
        };

        let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            let metadata = (vtable.get_metadata)();
            let info = PluginInfo::from_metadata(&metadata);
            // Access all fields
            let _ = format!(
                "name={} version={} deps={} tags={} caps={}",
                info.name,
                info.version,
                info.dependencies.len(),
                info.tags.len(),
                info.capabilities.len()
            );
        }));

        match result {
            Ok(()) => {
                println!("  [PASS] synthetic vtable '{}': handled gracefully", label);
                results.push(FuzzResult::Ok);
            }
            Err(e) => {
                println!("  [FAIL] synthetic vtable '{}': panic {:?}", label, e);
                results.push(FuzzResult::Panic(format!("{}: {:?}", label, e)));
            }
        }
    }

    results
}

/// Harness 7: Test null text pointer handling across all VTable functions.
#[cfg(not(miri))]
fn fuzz_null_text_detection(vtable: &PluginVTable) -> FuzzResult {
    let input = PluginInput {
        text: std::ptr::null_mut(),
    };

    let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        // validate_input should return false for null text
        let valid = (vtable.validate_input)(&input);
        assert!(!valid, "validate_input should return false for null text");

        // run should return null output for null text
        let output = (vtable.run)(&input);
        assert!(
            output.text.is_null(),
            "run should return null output for null text"
        );
        (vtable.free_output)(output);

        // run_with_buffer should return 0 for null text
        let mut buffer = vec![0u8; 256];
        let written =
            (vtable.run_with_buffer)(&input, buffer.as_mut_ptr() as *mut c_char, buffer.len());
        assert_eq!(written, 0, "run_with_buffer should return 0 for null text");
    }));

    match result {
        Ok(()) => {
            println!("  [PASS] Null text pointer: all VTable functions handle it safely");
            FuzzResult::Ok
        }
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

/// Harness 8: Fuzz MultiModalInput struct construction and field access.
fn fuzz_multimodal_input_construction(rng: &mut FuzzRng) -> FuzzResult {
    let mut owned = OwnedStrings::new();
    let mmi = gen_multimodal_input(rng, &mut owned);

    let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        // Verify fields are accessible without crash
        let _ = mmi.input_type;
        let _ = mmi.binary_size;

        // Validate text_data if non-null
        if !mmi.text_data.is_null() {
            let c_str = CStr::from_ptr(mmi.text_data);
            let _ = c_str.to_string_lossy();
        }

        // Validate file_path if non-null
        if !mmi.file_path.is_null() {
            let c_str = CStr::from_ptr(mmi.file_path);
            let _ = c_str.to_string_lossy();
        }

        // Validate metadata if non-null
        if !mmi.metadata.is_null() {
            let c_str = CStr::from_ptr(mmi.metadata);
            let s = c_str.to_string_lossy();
            // Try to parse as JSON (should not panic even on invalid JSON)
            let _: Result<serde_json::Value, _> = serde_json::from_str(&s);
        }

        // Validate binary_data bounds (only access if pointer is non-null and size > 0)
        if !mmi.binary_data.is_null() && mmi.binary_size > 0 {
            // We can only safely read if the actual backing buffer is large enough.
            // The generator sometimes lies about size, so we just verify the pointer exists.
            let _ = *mmi.binary_data; // Read first byte
        }
    }));

    match result {
        Ok(()) => FuzzResult::Ok,
        Err(e) => FuzzResult::Panic(format!("{:?}", e)),
    }
}

// ---------------------------------------------------------------------------
// Helper: load EchoPlugin if available (not available under Miri — no FFI)
// ---------------------------------------------------------------------------

#[cfg(not(miri))]
fn load_echo_plugin() -> Option<PluginRegistry> {
    let plugin_dir = PathUtils::plugin_dir();
    let plugin_dir_str = plugin_dir.to_str().unwrap_or("plugins");
    let reg = PluginRegistry::dynamic_registry(plugin_dir_str);
    if reg.get("EchoPlugin").is_some() {
        Some(reg)
    } else {
        println!("EchoPlugin not available, skipping plugin-dependent fuzz tests");
        None
    }
}

// ---------------------------------------------------------------------------
// Test functions
// ---------------------------------------------------------------------------

// Miri is ~100x slower; use fewer iterations. Even one iteration is valuable
// since Miri checks every memory operation for UB.
#[cfg(miri)]
const FUZZ_ITERATIONS: usize = 50;
#[cfg(not(miri))]
const FUZZ_ITERATIONS: usize = 10_000;

#[test]
fn fuzz_test_plugin_info_from_metadata() {
    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_plugin_info_from_metadata(&mut rng);
        stats.record(result);
    }

    stats.print_summary("PluginInfo::from_metadata");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in PluginInfo::from_metadata",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
fn fuzz_test_metadata_json_parsing() {
    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_metadata_json_parsing(&mut rng);
        stats.record(result);
    }

    stats.print_summary("Metadata JSON parsing");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in metadata JSON parsing",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
fn fuzz_test_multimodal_input_construction() {
    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_multimodal_input_construction(&mut rng);
        stats.record(result);
    }

    stats.print_summary("MultiModalInput construction");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in MultiModalInput construction",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
fn fuzz_test_synthetic_vtable_metadata() {
    println!("Testing synthetic vtables with adversarial get_metadata...");
    let results = fuzz_synthetic_vtable_metadata();

    let panics: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, FuzzResult::Panic(_)))
        .collect();

    assert!(
        panics.is_empty(),
        "Synthetic vtable tests had {} panics",
        panics.len()
    );
}

#[test]
#[cfg(not(miri))]
#[serial]
fn fuzz_test_echo_plugin_run() {
    let reg = match load_echo_plugin() {
        Some(r) => r,
        None => return,
    };
    let echo = reg.get("EchoPlugin").unwrap();
    let vtable = unsafe { &*echo.vtable };

    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_plugin_run(vtable, &mut rng);
        stats.record(result);
    }

    stats.print_summary("EchoPlugin::run");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in EchoPlugin::run",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
#[cfg(not(miri))]
#[serial]
fn fuzz_test_echo_plugin_validate_input() {
    let reg = match load_echo_plugin() {
        Some(r) => r,
        None => return,
    };
    let echo = reg.get("EchoPlugin").unwrap();
    let vtable = unsafe { &*echo.vtable };

    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_plugin_validate_input(vtable, &mut rng);
        stats.record(result);
    }

    stats.print_summary("EchoPlugin::validate_input");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in EchoPlugin::validate_input",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
#[cfg(not(miri))]
#[serial]
fn fuzz_test_echo_plugin_run_with_buffer() {
    let reg = match load_echo_plugin() {
        Some(r) => r,
        None => return,
    };
    let echo = reg.get("EchoPlugin").unwrap();
    let vtable = unsafe { &*echo.vtable };

    let seed = get_seed();
    println!(
        "Fuzz seed: {} (reproduce with LAO_FUZZ_SEED={})",
        seed, seed
    );
    let mut rng = FuzzRng::new(seed);
    let mut stats = FuzzStats::default();

    for _ in 0..FUZZ_ITERATIONS {
        let result = fuzz_plugin_run_with_buffer(vtable, &mut rng);
        stats.record(result);
    }

    stats.print_summary("EchoPlugin::run_with_buffer");
    assert!(
        !stats.has_failures(),
        "Fuzzer found {} panics and {} validation failures in EchoPlugin::run_with_buffer",
        stats.panics,
        stats.validation_failures
    );
}

#[test]
#[cfg(not(miri))]
#[serial]
fn fuzz_test_null_text_pointer_detection() {
    let reg = match load_echo_plugin() {
        Some(r) => r,
        None => return,
    };
    let echo = reg.get("EchoPlugin").unwrap();
    let vtable = unsafe { &*echo.vtable };

    let result = fuzz_null_text_detection(vtable);
    assert!(
        matches!(result, FuzzResult::Ok),
        "Null text detection test failed"
    );
}
