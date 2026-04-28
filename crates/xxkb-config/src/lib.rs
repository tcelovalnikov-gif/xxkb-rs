//! Configuration schema for xxkb-rs.
//!
//! The on-disk format is **TOML**, located at `$XDG_CONFIG_HOME/xxkb/config.toml`
//! (defaulting to `~/.config/xxkb/config.toml`).
//!
//! Loading flow:
//!
//! ```text
//!   defaults  +  file  +  env (XXKB_*)
//!         (figment merge, last wins)
//!                     |
//!                     v
//!               Config (this struct)
//!                     |
//!         validate / canonicalise paths
//!                     v
//!                Config (validated)
//! ```
//!
//! Saving is atomic (`tempfile` + `persist`) so a crash mid-write doesn't
//! leave a half-baked config on disk.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::{
    io,
    path::{Path, PathBuf},
};

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, trace};
use xxkb_core::{monitors::OutputName, AppRule, Point};

mod paths;

pub use paths::{config_dir, config_path, data_dir, expand_path, sound_dir, user_icons_dir};

/// Errors loading or saving config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Underlying I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// `figment` (TOML or Env) error during load. Boxed because
    /// `figment::Error` is large (>200 bytes) and would bloat all
    /// `Result<_, ConfigError>` returns.
    #[error("config load error: {0}")]
    Load(Box<figment::Error>),

    /// TOML serialization error during save.
    #[error("config serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// Validation error after parsing.
    #[error("invalid config: {0}")]
    Validation(String),
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Global, mode-affecting flags.
    pub general: GeneralConfig,
    /// The "main" indicator that lives on each display.
    pub main_indicator: MainIndicatorConfig,
    /// The per-window indicator drawn over title bars.
    pub per_window_indicator: PerWindowIndicatorConfig,
    /// Icon configuration (paths, mappings, etc).
    pub icons: IconsConfig,
    /// Sound configuration.
    pub sound: SoundConfig,
    /// Per-application rules.
    pub app_rules: Vec<AppRule>,
}

/// Global flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralConfig {
    /// Toggle two-state cycle (between [`base_group`] and [`alt_group`]).
    pub two_state: bool,
    /// Primary group (1-based).
    pub base_group: u8,
    /// Alternative group (1-based).
    pub alt_group: u8,
    /// Modifier required to cycle layouts via the legacy hotkey
    /// (`none`, `shift`, `lock`, `ctrl`, `alt`, `mod1`..`mod5`).
    pub cycle_modifier: ModifierName,
    /// If true, the meaning of `app_rules` `Ignore` is inverted.
    pub ignore_reverse: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            two_state: false,
            base_group: 1,
            alt_group: 2,
            cycle_modifier: ModifierName::None,
            ignore_reverse: false,
        }
    }
}

/// Display mode for the main indicator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MainIndicatorMode {
    /// Show the indicator only on the RandR primary output.
    PrimaryOnly,
    /// Show on every active output.
    #[default]
    AllDisplays,
}

/// Border around an indicator window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BorderConfig {
    /// Master switch.
    pub enabled: bool,
    /// Hex color string `#RRGGBB` or `#RRGGBBAA`.
    pub color: String,
    /// Border width in pixels.
    pub width: u32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            color: "#000000".into(),
            width: 1,
        }
    }
}

/// Main (per-display) indicator settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MainIndicatorConfig {
    /// Master switch.
    pub enable: bool,
    /// Display mode: primary-only vs all displays.
    pub mode: MainIndicatorMode,
    /// Side length in pixels (square).
    pub size_px: u32,
    /// Optional border.
    pub border: BorderConfig,
    /// If true, after Ctrl-dragging the main indicator the daemon opens a
    /// short confirmation dialog (`zenity` or `kdialog`) before writing
    /// `config.toml`. If the user declines, the indicator snaps back to its
    /// previous position.
    #[serde(default)]
    pub confirm_drag_save: bool,
    /// Saved positions, keyed by RandR output name.
    pub positions: IndexMap<OutputName, Point>,
}

impl Default for MainIndicatorConfig {
    fn default() -> Self {
        Self {
            enable: true,
            mode: MainIndicatorMode::AllDisplays,
            size_px: 48,
            border: BorderConfig::default(),
            positions: IndexMap::new(),
            confirm_drag_save: false,
        }
    }
}

/// Per-window indicator settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PerWindowIndicatorConfig {
    /// Master switch.
    pub enable: bool,
    /// Side length in pixels.
    pub size_px: u32,
    /// Offset relative to the title bar.
    pub offset: xxkb_core::Offset,
    /// Optional border.
    pub border: BorderConfig,
}

impl Default for PerWindowIndicatorConfig {
    fn default() -> Self {
        Self {
            enable: true,
            size_px: 15,
            offset: xxkb_core::Offset { x: -60, y: 7 },
            border: BorderConfig::default(),
        }
    }
}

/// Icon (flag) configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct IconsConfig {
    /// Prefer SVG over raster when both are available.
    pub prefer_svg: bool,
    /// Search paths for icons. The literal `"system"` and `"builtin"` are
    /// recognised special values and replaced with the system-wide path
    /// and bundled fallbacks respectively.
    pub search_paths: Vec<String>,
    /// Mapping `group_one_based -> icon_name`. Stored with stringified
    /// keys because TOML tables require string keys.
    pub mapping: IndexMap<String, String>,
}

impl Default for IconsConfig {
    fn default() -> Self {
        Self {
            prefer_svg: true,
            search_paths: vec![
                "~/.local/share/icons/xxkb".into(),
                "system".into(),
                "builtin".into(),
            ],
            mapping: IndexMap::from([
                ("1".into(), "en".into()),
                ("2".into(), "ru".into()),
                ("3".into(), "ua".into()),
                ("4".into(), "by".into()),
            ]),
        }
    }
}

impl IconsConfig {
    /// Look up the icon name for a 1-based group.
    #[must_use]
    pub fn icon_for(&self, group_one_based: u8) -> Option<&str> {
        self.mapping
            .get(&group_one_based.to_string())
            .map(String::as_str)
    }
}

/// Sound configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SoundConfig {
    /// Playback mode.
    pub mode: SoundMode,
    /// Optional file path. If empty, a built-in click is used.
    pub file: String,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            mode: SoundMode::Off,
            file: String::new(),
        }
    }
}

/// When to play the sound.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoundMode {
    /// Never.
    Off,
    /// Only when the user pressed the hotkey.
    ManualOnly,
    /// Only on programmatic switch (focus change).
    AutoOnly,
    /// Always.
    Both,
}

/// Modifier key name (mirrors legacy `XXkb.keymask.cycle`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModifierName {
    /// No modifier required.
    None,
    /// Shift.
    Shift,
    /// CapsLock (legacy `lock`).
    Lock,
    /// Control.
    #[serde(alias = "control")]
    Ctrl,
    /// Alt.
    Alt,
    /// Mod1.
    Mod1,
    /// Mod2.
    Mod2,
    /// Mod3.
    Mod3,
    /// Mod4.
    Mod4,
    /// Mod5.
    Mod5,
}

impl Config {
    /// Load from default location (creating a default config if absent).
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = config_path()?;
        Self::load_from(&path)
    }

    /// Load from a specific path, merging with defaults and `XXKB_*` env vars.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        debug!(?path, "loading config");
        let mut figment = Figment::new().merge(Serialized::defaults(Self::default()));
        if path.exists() {
            figment = figment.merge(Toml::file(path));
        } else {
            trace!(?path, "config file does not exist, using defaults");
        }
        // Only accept env-var overrides that are *clearly* meant as
        // structured config (`XXKB_<SECTION>__<FIELD>=...`). The
        // `__` is our nesting marker; anything without it is treated
        // as application data we don't own — e.g. `XXKB_TEST_XVFB=1`
        // used by integration tests, or random `XXKB_DEBUG=1` flags
        // a user might already export. Without this filter, figment
        // (combined with `deny_unknown_fields`) would error out and
        // the daemon would refuse to start.
        // NOTE: filter MUST run before split — `Env::split` is
        // implemented as a key-rewriting map that replaces `__` with
        // `.`, so by the time the filter would see the key, the `__`
        // marker would already be gone.
        figment = figment.merge(
            Env::prefixed("XXKB_")
                .filter(|k| k.as_str().contains("__"))
                .split("__"),
        );
        let cfg: Self = figment
            .extract()
            .map_err(|e| ConfigError::Load(Box::new(e)))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Persist config atomically to `path`.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        use std::io::Write;
        tmp.write_all(toml_str.as_bytes())?;
        tmp.flush()?;
        tmp.persist(path).map_err(|e| ConfigError::Io(e.error))?;
        Ok(())
    }

    /// Validate cross-field constraints. Called from [`Self::load_from`].
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.general.base_group == 0 || self.general.base_group > 4 {
            return Err(ConfigError::Validation(format!(
                "general.base_group must be in 1..=4, got {}",
                self.general.base_group
            )));
        }
        if self.general.alt_group == 0 || self.general.alt_group > 4 {
            return Err(ConfigError::Validation(format!(
                "general.alt_group must be in 1..=4, got {}",
                self.general.alt_group
            )));
        }
        if self.main_indicator.size_px == 0 {
            return Err(ConfigError::Validation(
                "main_indicator.size_px must be > 0".into(),
            ));
        }
        if self.per_window_indicator.size_px == 0 {
            return Err(ConfigError::Validation(
                "per_window_indicator.size_px must be > 0".into(),
            ));
        }
        if !self.main_indicator.border.color.starts_with('#') {
            return Err(ConfigError::Validation(format!(
                "main_indicator.border.color must start with '#', got {}",
                self.main_indicator.border.color
            )));
        }
        Ok(())
    }

    /// Path to the canonical config file (creates parent dirs if needed,
    /// but does not create the file itself).
    pub fn ensure_config_dir() -> Result<PathBuf, ConfigError> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn defaults_round_trip() {
        let original = Config::default();
        let s = toml::to_string_pretty(&original).unwrap();
        let parsed: Config = toml::from_str(&s).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn load_from_minimal_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[general]
two_state = true
base_group = 1
alt_group = 2
cycle_modifier = "ctrl"

[main_indicator]
mode = "primary_only"
size_px = 32
"#
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.general.two_state);
        assert_eq!(cfg.general.cycle_modifier, ModifierName::Ctrl);
        assert_eq!(cfg.main_indicator.mode, MainIndicatorMode::PrimaryOnly);
        assert_eq!(cfg.main_indicator.size_px, 32);
    }

    #[test]
    fn save_and_reload_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.general.two_state = true;
        cfg.main_indicator
            .positions
            .insert("DP-1".into(), Point::new(100, 200));
        cfg.save_to(&path).unwrap();
        let reloaded = Config::load_from(&path).unwrap();
        assert_eq!(cfg, reloaded);
    }

    #[test]
    fn invalid_base_group_rejected() {
        let mut cfg = Config::default();
        cfg.general.base_group = 0;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
        cfg.general.base_group = 7;
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn invalid_color_rejected() {
        let mut cfg = Config::default();
        cfg.main_indicator.border.color = "red".into();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn unknown_field_in_config_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[general]\ntwo_state = true\nbogus = 42\n").unwrap();
        let err = Config::load_from(&path).err().unwrap();
        assert!(matches!(err, ConfigError::Load(_)));
    }

    /// Stray `XXKB_*` env vars without the structured `__` separator
    /// (e.g. `XXKB_TEST_XVFB=1` set by the integration-test runner,
    /// or `XXKB_DEBUG=1` exported by a user) must NOT prevent the
    /// daemon from starting. Regression test for the CI Xvfb job.
    #[test]
    #[allow(unsafe_code)] // env::set_var is unsafe on edition 2024 rustc
    fn stray_env_var_without_double_underscore_is_ignored() {
        // SAFETY: we only mutate one well-known variable and unset it
        // before the test returns. Tests in the same crate run in the
        // same process; nothing else here reads `XXKB_TEST_XVFB`.
        unsafe {
            std::env::set_var("XXKB_TEST_XVFB", "1");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let result = Config::load_from(&path);
        unsafe {
            std::env::remove_var("XXKB_TEST_XVFB");
        }
        assert!(
            result.is_ok(),
            "load should ignore stray XXKB_TEST_XVFB, got: {result:?}"
        );
    }

    /// Conversely, a properly structured override like
    /// `XXKB_GENERAL__TWO_STATE=true` MUST be honoured — that's the
    /// raison d'être of the env layer.
    #[test]
    #[allow(unsafe_code)]
    fn structured_double_underscore_env_override_is_applied() {
        // SAFETY: same single-var mutation as above.
        unsafe {
            std::env::set_var("XXKB_GENERAL__TWO_STATE", "true");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let result = Config::load_from(&path);
        unsafe {
            std::env::remove_var("XXKB_GENERAL__TWO_STATE");
        }
        let cfg = result.expect("structured env override should load");
        assert!(cfg.general.two_state);
    }
}
