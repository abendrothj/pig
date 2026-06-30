use std::path::{Component, Path, PathBuf};

/// Canonicalize a path for trust checks without requiring the target to exist.
pub fn canonicalize_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty() {
        return Err("empty path".to_string());
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => normalized.push(p.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("path escapes above root via '..'".to_string());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("path resolved to empty".to_string());
    }

    Ok(normalized)
}

pub fn path_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    if roots.is_empty() {
        return false;
    }
    roots.iter().any(|root| path.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        assert!(canonicalize_path("../etc/passwd").is_err());
    }

    #[test]
    fn normalizes_relative_path() {
        let p = canonicalize_path("./foo/bar").unwrap();
        assert_eq!(p, PathBuf::from("foo/bar"));
    }
}
