//! Path resolution for config / icons / sounds.

use std::path::PathBuf;

use crate::ConfigError;

/// Canonical config directory: `$XDG_CONFIG_HOME/xxkb`, fallback `~/.config/xxkb`.
pub fn config_dir() -> Result<PathBuf, ConfigError> {
    Ok(dirs::config_dir()
        .ok_or_else(|| ConfigError::Validation("XDG_CONFIG_HOME unavailable".into()))?
        .join("xxkb"))
}

/// Canonical config file: `<config_dir()>/config.toml`.
pub fn config_path() -> Result<PathBuf, ConfigError> {
    Ok(config_dir()?.join("config.toml"))
}

/// User-private data directory: `~/.local/share/xxkb`.
pub fn data_dir() -> Result<PathBuf, ConfigError> {
    Ok(dirs::data_dir()
        .ok_or_else(|| ConfigError::Validation("XDG_DATA_HOME unavailable".into()))?
        .join("xxkb"))
}

/// User-private icons directory: `~/.local/share/icons/xxkb`.
pub fn user_icons_dir() -> Result<PathBuf, ConfigError> {
    Ok(dirs::data_dir()
        .ok_or_else(|| ConfigError::Validation("XDG_DATA_HOME unavailable".into()))?
        .join("icons")
        .join("xxkb"))
}

/// User-private sounds directory: `~/.local/share/sounds/xxkb`.
pub fn sound_dir() -> Result<PathBuf, ConfigError> {
    Ok(dirs::data_dir()
        .ok_or_else(|| ConfigError::Validation("XDG_DATA_HOME unavailable".into()))?
        .join("sounds")
        .join("xxkb"))
}

/// Expand `~` and environment variables in a path-string.
pub fn expand_path(s: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(s).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_path_handles_tilde() {
        let home = std::env::var("HOME").ok();
        let p = expand_path("~/foo");
        if let Some(h) = home {
            assert_eq!(p, PathBuf::from(format!("{h}/foo")));
        }
    }

    #[test]
    fn expand_path_passthrough() {
        assert_eq!(expand_path("/tmp/foo"), PathBuf::from("/tmp/foo"));
    }
}
