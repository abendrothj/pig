# Security Policy

pig runs on local hardware and routes inference traffic across private LAN workers.

## Trust Model

- The coordinator and worker processes run with the privileges of the launching user.
- Worker HTTP APIs should not be exposed to untrusted networks — bind to LAN addresses only.
- Auth tokens (if configured) are passed as bearer tokens; keep `pig.toml` readable only by the owning user.

## Reporting Vulnerabilities

Please report suspected vulnerabilities privately to the maintainer listed in `Cargo.toml`.
Include reproduction steps, affected component, and platform.
