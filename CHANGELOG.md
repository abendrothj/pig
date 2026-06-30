# Changelog

All notable changes to LAO will be documented here.

This project follows semantic versioning for the CLI/core crates and treats `lao_plugin_api` as an external compatibility contract. Breaking ABI or workflow schema changes require a major version bump once the first stable release is cut.

## Unreleased

- Production-readiness hardening is in progress.
- Workflow schema now rejects unsupported control-flow and modality fields.
- CLI/core execution routes through the loop-capable workflow runner.
- Core plugin calls validate inputs and reclaim host-owned FFI input memory.
- Trust policy denies dangerous plugin capabilities by default.
