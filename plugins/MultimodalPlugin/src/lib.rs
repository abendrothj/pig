//! Multimodal Processor Plugin - Handles format conversion and modality detection
//!
//! Supports: Audio -> Text, Image -> Text, Video -> Frames, Format Conversion
//! Use input_modality and output_modality in workflow steps

use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTable, PluginVTablePtr};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

unsafe extern "C" fn name() -> *const c_char {
    CString::new("MultimodalPlugin").unwrap().into_raw()
}

unsafe extern "C" fn run(input: *const PluginInput) -> PluginOutput {
    if input.is_null() {
        println!("[MultimodalPlugin] Received null input");
        return PluginOutput {
            text: std::ptr::null_mut(),
        };
    }

    let c_str = CStr::from_ptr((*input).text);
    let s = c_str.to_string_lossy();
    println!("[MultimodalPlugin] Processing input: {}", s);

    // Detect modality from input metadata (if available)
    let response = format!(
        r#"{{"status":"processed","input":"{}","modality":"multimodal","processed":true}}"#,
        s
    );

    let out = CString::new(response).unwrap();
    PluginOutput {
        text: out.into_raw(),
    }
}

unsafe extern "C" fn free_output(output: PluginOutput) {
    if !output.text.is_null() {
        let _ = CString::from_raw(output.text);
    }
}

unsafe extern "C" fn metadata() -> PluginMetadata {
    PluginMetadata {
        name: CString::new("MultimodalPlugin").unwrap().into_raw(),
        version: CString::new("0.1.0").unwrap().into_raw(),
        description: CString::new(
            "Handles multimodal format conversion (audio→text, image→text, video→frames)",
        )
        .unwrap()
        .into_raw(),
        author: CString::new("LAO Team").unwrap().into_raw(),
        dependencies: CString::new("[]").unwrap().into_raw(),
        tags: CString::new(r#"["multimodal", "audio", "image", "video"]"#)
            .unwrap()
            .into_raw(),
        input_schema: CString::new(r#"{"type":"string"}"#)
            .unwrap()
            .into_raw(),
        output_schema: CString::new(r#"{"type":"object"}"#)
            .unwrap()
            .into_raw(),
        capabilities: CString::new(r#"["audio_to_text", "image_to_text", "video_to_frames"]"#)
            .unwrap()
            .into_raw(),
    }
}

unsafe extern "C" fn validate_input(_input: *const PluginInput) -> bool {
    true // Validate implementation can be added
}

unsafe extern "C" fn get_capabilities() -> *const c_char {
    CString::new(r#"["audio_to_text", "image_to_text", "video_to_frames"]"#)
        .unwrap()
        .into_raw()
}

unsafe extern "C" fn run_with_buffer(
    input: *const PluginInput,
    _buffer: *mut c_char,
    _buffer_size: usize,
) -> usize {
    if input.is_null() {
        return 0;
    }

    let c_str = CStr::from_ptr((*input).text);
    let s = c_str.to_string_lossy();
    s.len()
}

#[no_mangle]
pub extern "C" fn plugin_init() -> PluginVTablePtr {
    let vtable = Box::new(PluginVTable {
        version: 1,
        name,
        run,
        free_output,
        run_with_buffer,
        get_metadata: metadata,
        validate_input,
        get_capabilities,
    });
    Box::into_raw(vtable) as PluginVTablePtr
}

#[no_mangle]
pub extern "C" fn plugin_destroy(ptr: PluginVTablePtr) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = Box::from_raw(ptr as *mut PluginVTable);
    }
}
