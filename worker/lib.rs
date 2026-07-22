//! LAO worker: a long-running process that supervises a model backend, exposes it
//! over HTTP, and (via `coordinator`) lets the CLI/workflow engine talk to any
//! configured worker without knowing backend details.

pub mod backend;
pub mod config;
pub mod hardware;
pub mod job;
pub mod server;
pub mod state;
