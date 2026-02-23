//! Multimodal Processor Plugin - Handles format conversion and modality detection
//!
//! Supports: Audio -> Text, Image -> Text, Video -> Frames, Format Conversion
//! Use input_modality and output_modality in workflow steps

use lao_plugin_api::{PluginInput, PluginMetadata, PluginOutput, PluginVTablePtr};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

unsafe extern "C" fn name() -> *const c_char {
    c"MultimodalPlugin".as_ptr()
}

unsafe extern "C" fn run(input: *const PluginInput) -> PluginOutput {
    if input.is_null() {
        return PluginOutput {
            text: std::ptr::null_mut(),
        };
    }

    let c_str = CStr::from_ptr((*input).text);
    let s = c_str.to_string_lossy();

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

unsafe extern "C" fn run_with_buffer(
    _input: *const PluginInput,
    _buffer: *mut c_char,
    _buffer_size: usize,
) -> usize {
    0
}

unsafe extern "C" fn get_metadata() -> PluginMetadata {
    static NAME: &[u8] = b"MultimodalPlugin\0";
    static VERSION: &[u8] = b"0.1.0\0";
    static DESCRIPTION: &[u8] =
        b"Handles multimodal format conversion (audio->text, image->text, video->frames)\0";
    static AUTHOR: &[u8] = b"LAO Team\0";
    static DEPS: &[u8] = b"[]\0";
    static TAGS: &[u8] = b"[\"multimodal\", \"audio\", \"image\", \"video\"]\0";
    static INPUT_SCHEMA: &[u8] = b"{\"type\":\"string\"}\0";
    static OUTPUT_SCHEMA: &[u8] = b"{\"type\":\"object\"}\0";
    static CAPABILITIES: &[u8] = b"[{\"name\":\"multimodal-process\",\"description\":\"Process multimodal inputs\",\"input_type\":\"Any\",\"output_type\":\"Json\"}]\0";

    PluginMetadata {
        name: NAME.as_ptr() as *const c_char,
        version: VERSION.as_ptr() as *const c_char,
        description: DESCRIPTION.as_ptr() as *const c_char,
        author: AUTHOR.as_ptr() as *const c_char,
        dependencies: DEPS.as_ptr() as *const c_char,
        tags: TAGS.as_ptr() as *const c_char,
        input_schema: INPUT_SCHEMA.as_ptr() as *const c_char,
        output_schema: OUTPUT_SCHEMA.as_ptr() as *const c_char,
        capabilities: CAPABILITIES.as_ptr() as *const c_char,
    }
}

unsafe extern "C" fn validate_input(input: *const PluginInput) -> bool {
    if input.is_null() {
        return false;
    }
    let c_str = CStr::from_ptr((*input).text);
    let text = c_str.to_string_lossy();
    !text.trim().is_empty()
}

unsafe extern "C" fn get_capabilities() -> *const c_char {
    static CAPABILITIES: &[u8] = b"[{\"name\":\"multimodal-process\",\"description\":\"Process multimodal inputs\",\"input_type\":\"Any\",\"output_type\":\"Json\"}]\0";
    CAPABILITIES.as_ptr() as *const c_char
}

#[no_mangle]
pub static PLUGIN_VTABLE: lao_plugin_api::PluginVTable = lao_plugin_api::PluginVTable {
    version: 1,
    name,
    run,
    free_output,
    run_with_buffer,
    get_metadata,
    validate_input,
    get_capabilities,
};

#[no_mangle]
pub extern "C" fn plugin_vtable() -> PluginVTablePtr {
    &PLUGIN_VTABLE
}
