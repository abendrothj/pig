# Plugin API Compatibility

`lao_plugin_api` is the compatibility contract between the LAO host and plugins.

## ABI Version

The current supported ABI version is `LAO_PLUGIN_ABI_VERSION = 1`. The host rejects plugins whose vtable version does not match the supported version.

## Semver Policy

Once LAO cuts a stable release:

- Patch releases must not change the layout or meaning of stable ABI structs.
- Minor releases may add optional host-side helpers but must preserve ABI v1 layout.
- Major releases are required for changing stable struct layout, vtable order, memory ownership, or plugin result semantics.

## Memory Ownership

- Plugin inputs are owned by the host and valid only for the duration of a plugin call.
- Plugin outputs are owned by the plugin and must be released by the host through `free_output`.
- Plugins must return null-terminated strings for text output.

## Experimental Surface

`MultiModalInput` and `run_with_buffer` are ABI v1 experimental surface. Do not rely on them as stable external plugin contracts.

## ABI v2 (structured results)

ABI v2 is shipped (`LAO_PLUGIN_ABI_VERSION = 2`). It adds a structured result channel at
the FFI boundary so the host no longer parses an `error:` text prefix to detect failure:

```rust
// lao_plugin_api (Rust):
pub const LAO_STATUS_SUCCESS: u32 = 0;
pub const LAO_STATUS_VALIDATION_FAILED: u32 = 1;
pub const LAO_STATUS_RUNTIME_ERROR: u32 = 2;

#[repr(C)]
pub struct PluginResult {
    pub status: u32,
    pub text: *mut c_char, // output on success, error message otherwise
}
```

```c
// Equivalent C ABI:
typedef struct { uint32_t status; char* text; } LaoPluginResult;
```

The `run_structured` and `free_result` function pointers are **appended** to
`PluginVTable` after the stable v1 prefix. The host reads them only when the plugin
reports `version >= 2`, and accesses every vtable field through raw, version-gated
pointer reads so a shorter v1 vtable can never be over-read.

### Compatibility

- `LAO_PLUGIN_ABI_MIN_SUPPORTED = 1`: v1 plugins still load and run through the legacy
  `run`/`free_output` text channel (status is derived from the `error:`/empty convention).
- New plugins should target v2. The simplest migration keeps your existing `run` and adds
  the adapters via the macro:

```rust
lao_plugin_api::lao_structured_adapter!(run); // generates __lao_run_structured / __lao_free_result

pub static PLUGIN_VTABLE: PluginVTable = PluginVTable {
    version: 2,
    /* ...v1 fields... */
    run_structured: __lao_run_structured,
    free_result: __lao_free_result,
};
```

- Plugins that want true machine-readable status (e.g. distinguishing validation from
  runtime failure) implement `run_structured` natively and return explicit status codes,
  as `EchoPlugin` does.
- A future out-of-process plugin host (ADR 0005) reuses this status model over a JSON
  envelope. A semver-major bump for `lao_plugin_api` accompanies any breaking change to
  the v1 prefix.
