//! Code intelligence provider abstraction: read-only structural code queries backed by
//! an external tool, reached by spawning its CLI directly (never an MCP client, never a
//! shell). See `provider` for the trait contract and `operations` for the allowlist.

pub mod cache;
pub mod codebase_memory_cli;
pub mod error;
pub mod operations;
pub mod provider;

pub use cache::CachingProvider;
pub use codebase_memory_cli::CodebaseMemoryCliProvider;
pub use error::ProviderError;
pub use operations::GraphOperation;
pub use provider::{CodeIntelligenceProvider, ProviderHealth, ProviderMetadata};
