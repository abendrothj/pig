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

`MultiModalInput` and `run_with_buffer` are ABI v1 experimental surface. Do not rely on them as stable external plugin contracts until ABI v2 formalizes structured inputs/results.

## ABI v2 (Planned)

ABI v2 will add structured plugin results at the FFI boundary:

```rust
// Host-side today (Rust):
pub struct PluginRunResult {
    pub status: PluginRunStatus, // success | validation_failed | runtime_error | ...
    pub output: Option<String>,
    pub error: Option<String>,
}

// Planned C ABI (v2):
typedef struct {
    uint32_t status;
    char* output;
    char* error;
} LaoPluginResult;
```

Goals for v2:

- Eliminate the `error:` text-prefix convention for machine-readable failures.
- Optional out-of-process plugin host with JSON envelopes (see ADR 0005).
- Semver-major bump for `lao_plugin_api` when v2 ships.

ABI v1 plugins remain supported until a documented migration window closes.
