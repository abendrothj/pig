# ADR 0003: Plugin API Compatibility

## Status

Accepted

## Decision

`lao_plugin_api` is an external compatibility contract. The host enforces `LAO_PLUGIN_ABI_VERSION`, and stable ABI layout or ownership changes require documented semver impact.

## Consequences

- External plugin authors have a clear compatibility target.
- ABI layout tests protect the stable vtable shape.
- Experimental ABI v1 surface such as `MultiModalInput` and `run_with_buffer` is not treated as a stable production contract until formalized.
