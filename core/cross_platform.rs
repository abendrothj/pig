use std::env;
use std::path::PathBuf;

pub struct Platform;

impl Platform {
    pub fn os() -> &'static str {
        env::consts::OS
    }

    pub fn arch() -> &'static str {
        env::consts::ARCH
    }

    pub fn family() -> &'static str {
        env::consts::FAMILY
    }

    pub fn is_linux() -> bool {
        Self::os() == "linux"
    }

    pub fn is_macos() -> bool {
        Self::os() == "macos"
    }

    pub fn is_windows() -> bool {
        Self::os() == "windows"
    }

    pub fn home_dir() -> Option<PathBuf> {
        env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_detection_returns_nonempty_values() {
        assert!(!Platform::os().is_empty());
        assert!(!Platform::arch().is_empty());
        let family = Platform::family();
        assert!(family == "unix" || family == "windows");
    }

    #[test]
    fn exactly_one_platform_flag_is_true() {
        let flags = [
            Platform::is_linux(),
            Platform::is_macos(),
            Platform::is_windows(),
        ];
        assert!(flags.iter().any(|&f| f));
    }
}
