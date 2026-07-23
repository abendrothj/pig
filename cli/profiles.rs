//! Explicit coordinator-selection profiles.  A remote profile can never silently
//! become an embedded coordinator: production intent must fail closed.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Profile {
    Embedded,
    Remote {
        coordinator_url: String,
        token_env: Option<String>,
    },
}

#[derive(Debug, Deserialize, Default)]
struct ProfileRoot {
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
}

pub fn selected(name: Option<&str>, config_text: &str) -> Result<Profile, String> {
    let Some(name) = name else {
        return Ok(Profile::Embedded);
    };
    let root: ProfileRoot =
        toml::from_str(config_text).map_err(|e| format!("invalid profile configuration: {e}"))?;
    root.profiles
        .get(name)
        .cloned()
        .ok_or_else(|| format!("unknown profile '{name}'"))
}

impl Profile {
    pub fn remote(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Remote {
                coordinator_url,
                token_env,
            } => Some((coordinator_url, token_env.as_deref())),
            Self::Embedded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_requires_a_url_at_parse_time() {
        assert!(selected(Some("bad"), "[profiles.bad]\nmode = 'remote'\n").is_err());
    }

    #[test]
    fn remote_profile_rejects_unknown_fields() {
        let config = "[profiles.prod]\nmode = 'remote'\ncoordinator_url = 'http://spectre:3001'\nunknown_field = 'x'\n";
        assert!(selected(Some("prod"), config).is_err());
    }

    #[test]
    fn omitted_profile_preserves_embedded_compatibility() {
        assert_eq!(selected(None, "").unwrap(), Profile::Embedded);
    }

    #[test]
    fn remote_profile_parses_with_url_and_token() {
        let config = "[profiles.prod]\nmode = 'remote'\ncoordinator_url = 'http://spectre:3001'\ntoken_env = 'PIG_TOKEN'\n";
        let profile = selected(Some("prod"), config).unwrap();
        assert_eq!(
            profile.remote(),
            Some(("http://spectre:3001", Some("PIG_TOKEN")))
        );
    }
}
