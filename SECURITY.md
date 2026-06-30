# Security Policy

LAO runs local workflows and dynamically loads plugins. Treat workflows and plugins as code.

## Trust Model

- Bundled plugins run in-process through a C ABI and have the privileges of the `lao-cli` process.
- Dangerous capabilities are denied by default by the core `TrustPolicy`.
- Use `lao.toml` to explicitly allow filesystem, shell, network, subprocess, or specific plugin capabilities for trusted workflows.
- `LAO_ALLOW_SHELL=1` remains a compatibility escape hatch for local trusted use only.

## Generated Workflows

`lao prompt` writes generated workflow YAML for review. Generated workflows should be validated and reviewed before execution, especially if they reference filesystem, shell, network, or subprocess plugins.

## Reporting Vulnerabilities

Please report suspected vulnerabilities privately to the maintainer listed in `Cargo.toml`. Include reproduction steps, affected plugin/workflow, platform, and whether a generated workflow was involved.

## Known Boundaries

The current production path is a hardened trusted in-process plugin host. Out-of-process plugin isolation is planned as a future compatibility milestone.
