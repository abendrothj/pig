//! On-disk cache for code-graph query results.
//!
//! Cache keys include provider identity, provider version, repository path, git
//! revision, dirty state, operation, and normalized arguments. Caching is disabled
//! outright for a dirty worktree: a dirty query is always served fresh and never
//! written to the cache, since a dirty working tree means the provider's index may not
//! reflect uncommitted changes at all.

use crate::code_intelligence::codebase_memory_cli::{git_is_dirty, git_revision};
use crate::code_intelligence::error::ProviderError;
use crate::code_intelligence::operations::GraphOperation;
use crate::code_intelligence::provider::{
    CodeIntelligenceProvider, ProviderHealth, ProviderMetadata,
};
use crate::execution::CodeGraphArtifact;
use std::fs;
use std::path::PathBuf;

pub struct CachingProvider<P: CodeIntelligenceProvider> {
    inner: P,
    repo_root: PathBuf,
    cache_dir: PathBuf,
}

impl<P: CodeIntelligenceProvider> CachingProvider<P> {
    pub fn new(inner: P, repo_root: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            inner,
            repo_root,
            cache_dir,
        }
    }
}

impl<P: CodeIntelligenceProvider> CodeIntelligenceProvider for CachingProvider<P> {
    fn health(&self) -> Result<ProviderHealth, ProviderError> {
        self.inner.health()
    }

    fn query(
        &self,
        operation: GraphOperation,
        args: serde_json::Value,
    ) -> Result<CodeGraphArtifact, ProviderError> {
        let dirty = git_is_dirty(&self.repo_root);
        if dirty {
            return self.inner.query(operation, args);
        }

        let revision = git_revision(&self.repo_root);
        let meta = self.inner.metadata();
        let key = cache_key(
            &meta,
            &self.repo_root,
            revision.as_deref(),
            operation,
            &args,
        );
        let cache_path = self.cache_dir.join(format!("{}.json", key));

        if let Ok(cached) = fs::read_to_string(&cache_path) {
            if let Ok(artifact) = serde_json::from_str::<CodeGraphArtifact>(&cached) {
                return Ok(artifact);
            }
        }

        let artifact = self.inner.query(operation, args)?;
        if let Ok(serialized) = serde_json::to_string(&artifact) {
            if fs::create_dir_all(&self.cache_dir).is_ok() {
                // The cache directory may live inside the repo it's caching queries
                // for; without this, writing a cache file would itself dirty the
                // worktree and permanently defeat the cache on the very next query.
                ensure_gitignored(&self.cache_dir);
                let _ = fs::write(&cache_path, serialized);
            }
        }
        Ok(artifact)
    }

    fn metadata(&self) -> ProviderMetadata {
        self.inner.metadata()
    }
}

/// Drop a `.gitignore` (`*`) into the cache directory so its contents never show up as
/// untracked changes in `git status`, regardless of where the cache directory sits
/// relative to the repository it caches queries for.
fn ensure_gitignored(cache_dir: &std::path::Path) {
    let gitignore = cache_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = fs::write(&gitignore, "*\n");
    }
}

fn cache_key(
    meta: &ProviderMetadata,
    repo_root: &std::path::Path,
    git_revision: Option<&str>,
    operation: GraphOperation,
    args: &serde_json::Value,
) -> String {
    let normalized_args = serde_json::to_string(args).unwrap_or_default();
    let raw = format!(
        "{}|{}|{}|{}|{}|{}",
        meta.name,
        meta.version.as_deref().unwrap_or(""),
        repo_root.display(),
        git_revision.unwrap_or(""),
        operation.tool_name(),
        normalized_args,
    );
    let mut hash: u64 = 1469598103934665603; // FNV-1a 64-bit offset basis
    for b in raw.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{:x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_intelligence::provider::CodeIntelligenceProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    impl CodeIntelligenceProvider for CountingProvider {
        fn health(&self) -> Result<ProviderHealth, ProviderError> {
            Ok(ProviderHealth {
                available: true,
                detail: "ok".to_string(),
            })
        }

        fn query(
            &self,
            operation: GraphOperation,
            _args: serde_json::Value,
        ) -> Result<CodeGraphArtifact, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CodeGraphArtifact {
                provider: "counting".to_string(),
                provider_version: Some("1.0.0".to_string()),
                repo_root: PathBuf::from("/repo"),
                git_revision: Some("deadbeef".to_string()),
                dirty: false,
                indexed_at: None,
                operation: operation.tool_name().to_string(),
                payload: serde_json::json!({"n": self.calls.load(Ordering::SeqCst)}),
            })
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "counting".to_string(),
                version: Some("1.0.0".to_string()),
            }
        }
    }

    // Cache-key construction is exercised directly (no git dependency) rather than
    // through a real repo, since these tests must be hermetic and fast.
    #[test]
    fn cache_key_is_stable_for_identical_inputs() {
        let meta = ProviderMetadata {
            name: "p".to_string(),
            version: Some("1".to_string()),
        };
        let args = serde_json::json!({"a": 1});
        let k1 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::SearchGraph,
            &args,
        );
        let k2 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::SearchGraph,
            &args,
        );
        assert_eq!(k1, k2);
    }

    #[test]
    fn cache_key_differs_when_revision_differs() {
        let meta = ProviderMetadata {
            name: "p".to_string(),
            version: Some("1".to_string()),
        };
        let args = serde_json::json!({"a": 1});
        let k1 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev1"),
            GraphOperation::SearchGraph,
            &args,
        );
        let k2 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev2"),
            GraphOperation::SearchGraph,
            &args,
        );
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_differs_when_operation_differs() {
        let meta = ProviderMetadata {
            name: "p".to_string(),
            version: Some("1".to_string()),
        };
        let args = serde_json::json!({"a": 1});
        let k1 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::SearchGraph,
            &args,
        );
        let k2 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::QueryGraph,
            &args,
        );
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_differs_when_args_differ() {
        let meta = ProviderMetadata {
            name: "p".to_string(),
            version: Some("1".to_string()),
        };
        let k1 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::SearchGraph,
            &serde_json::json!({"a": 1}),
        );
        let k2 = cache_key(
            &meta,
            std::path::Path::new("/repo"),
            Some("rev"),
            GraphOperation::SearchGraph,
            &serde_json::json!({"a": 2}),
        );
        assert_ne!(k1, k2);
    }

    /// A hermetic, freshly-committed temp git repo — independent of the ambient
    /// checkout's dirty state (which is unpredictable, e.g. mid-development).
    fn init_clean_temp_repo(name: &str) -> PathBuf {
        let repo = std::env::temp_dir().join(format!("{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&repo).expect("create temp repo dir");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .expect("git is available")
        };
        git(&["init", "-q"]);
        fs::write(repo.join("f.txt"), "hello").expect("write fixture file");
        git(&["add", "."]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "commit",
            "-q",
            "-m",
            "init",
        ]);
        repo
    }

    #[test]
    fn caching_provider_serves_second_call_from_cache_in_a_clean_repo() {
        let repo_root = init_clean_temp_repo("lao_cache_test_clean_repo");
        assert!(
            !git_is_dirty(&repo_root),
            "freshly committed repo should be clean"
        );

        let cache_dir = repo_root.join(".lao_cache");
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CachingProvider::new(
            CountingProvider {
                calls: calls.clone(),
            },
            repo_root.clone(),
            cache_dir,
        );

        let first = provider
            .query(GraphOperation::SearchGraph, serde_json::json!({"q": "x"}))
            .unwrap();
        let second = provider
            .query(GraphOperation::SearchGraph, serde_json::json!({"q": "x"}))
            .unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second query should be served from cache"
        );
        assert_eq!(first.payload, second.payload);

        let _ = fs::remove_dir_all(&repo_root);
    }

    #[test]
    fn caching_provider_bypasses_cache_in_a_dirty_repo() {
        let repo_root = init_clean_temp_repo("lao_cache_test_dirty_repo");
        // Introduce an uncommitted change so the worktree is dirty.
        fs::write(repo_root.join("f.txt"), "modified").expect("dirty the worktree");
        assert!(git_is_dirty(&repo_root), "modified repo should be dirty");

        let cache_dir = repo_root.join(".lao_cache");
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CachingProvider::new(
            CountingProvider {
                calls: calls.clone(),
            },
            repo_root.clone(),
            cache_dir,
        );

        provider
            .query(GraphOperation::SearchGraph, serde_json::json!({"q": "x"}))
            .unwrap();
        provider
            .query(GraphOperation::SearchGraph, serde_json::json!({"q": "x"}))
            .unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "dirty worktree must never be served from cache"
        );

        let _ = fs::remove_dir_all(&repo_root);
    }
}
