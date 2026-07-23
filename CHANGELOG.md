# Changelog

All notable changes to pig will be documented here.

This project follows semantic versioning.

## Unreleased

## 0.1.0

Initial release as pig (Private Inference Gateway).

- OpenAI-compatible coordinator gateway (`pig coordinator serve`)
- Worker management: serve, install/uninstall as systemd service, start/stop/status/logs
- Model registry: list, inspect, load/unload, generate, benchmark
- Constraint-based routing across llama.cpp workers
- Worker telemetry and metrics API
- Remote coordinator profiles via `pig.toml`
