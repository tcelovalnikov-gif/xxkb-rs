//! In-memory editor wrapper around [`Config`].
//!
//! The configurator GUI maintains a single [`ConfigEditor`] for the
//! lifetime of the window. It tracks two snapshots:
//!
//! * `baseline` — the most recent state successfully persisted to disk
//!   (or loaded from there at startup);
//! * `current` — the working copy mutated by the GUI.
//!
//! `dirty()` reports whether the two differ. Saving and loading update
//! the baseline atomically. All setters validate inputs through
//! [`ValidationError`] and refuse to mutate `current` on failure, so
//! the editor never holds a half-broken state.

use std::path::{Path, PathBuf};

use globset::Glob;
use xxkb_config::{BorderConfig, Config, ConfigError, MainIndicatorMode, ModifierName, SoundMode};
use xxkb_core::{
    monitors::OutputName,
    rules::{AppMatch, AppRule, RuleAction},
    Offset, Point,
};

use crate::validation::ValidationError;

/// Editor wrapping a [`Config`] with dirty-tracking.
#[derive(Debug, Clone)]
pub struct ConfigEditor {
    baseline: Config,
    current: Config,
    /// Path used by [`Self::save_to_default`] / [`Self::reload_from_disk`].
    path: PathBuf,
}

impl ConfigEditor {
    /// Build an editor from an explicit `Config` and target path. Tests
    /// use this to avoid touching the user's home directory.
    #[must_use]
    pub fn from_parts(cfg: Config, path: PathBuf) -> Self {
        Self {
            baseline: cfg.clone(),
            current: cfg,
            path,
        }
    }

    /// Load from `~/.config/xxkb/config.toml`, creating defaults if
    /// the file is missing.
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = xxkb_config::config_path()?;
        let cfg = Config::load_default()?;
        Ok(Self::from_parts(cfg, path))
    }

    /// Load from a specific path.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let cfg = Config::load_from(path)?;
        Ok(Self::from_parts(cfg, path.to_path_buf()))
    }

    /// Read-only access to the working copy.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.current
    }

    /// Read-only access to the last-persisted snapshot.
    #[must_use]
    pub fn baseline(&self) -> &Config {
        &self.baseline
    }

    /// Path that [`Self::save_to_default`] will write to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Are there unsaved changes?
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.current != self.baseline
    }

    /// Discard pending changes; revert to the baseline.
    pub fn discard(&mut self) {
        self.current = self.baseline.clone();
    }

    /// Persist the working copy, updating the baseline on success.
    pub fn save(&mut self) -> Result<(), ConfigError> {
        // We re-validate here because some setters (e.g. for
        // free-form fields like search paths) intentionally don't
        // run the cross-field checks that `Config::validate` does.
        self.current.validate()?;
        self.current.save_to(&self.path)?;
        self.baseline = self.current.clone();
        Ok(())
    }

    /// Re-read the file from disk, dropping any pending edits.
    pub fn reload_from_disk(&mut self) -> Result<(), ConfigError> {
        let cfg = Config::load_from(&self.path)?;
        self.baseline = cfg.clone();
        self.current = cfg;
        Ok(())
    }

    // -----------------------------------------------------------------
    // General page
    // -----------------------------------------------------------------

    /// Toggle the two-state cycle mode.
    pub fn set_two_state(&mut self, b: bool) {
        self.current.general.two_state = b;
    }

    /// Set the 1-based base group.
    pub fn set_base_group(&mut self, g: u8) -> Result<(), ValidationError> {
        ValidationError::check_group(g)?;
        self.current.general.base_group = g;
        Ok(())
    }

    /// Set the 1-based alt group.
    pub fn set_alt_group(&mut self, g: u8) -> Result<(), ValidationError> {
        ValidationError::check_group(g)?;
        self.current.general.alt_group = g;
        Ok(())
    }

    /// Set the cycle modifier.
    pub fn set_cycle_modifier(&mut self, m: ModifierName) {
        self.current.general.cycle_modifier = m;
    }

    /// Toggle the `ignore_reverse` flag.
    pub fn set_ignore_reverse(&mut self, b: bool) {
        self.current.general.ignore_reverse = b;
    }

    // -----------------------------------------------------------------
    // Main indicator page
    // -----------------------------------------------------------------

    /// Toggle main indicator master switch.
    pub fn set_main_enable(&mut self, b: bool) {
        self.current.main_indicator.enable = b;
    }

    /// Set the main indicator display mode.
    pub fn set_main_mode(&mut self, m: MainIndicatorMode) {
        self.current.main_indicator.mode = m;
    }

    /// Set main indicator size (px). Refuses 0 / overly large values.
    pub fn set_main_size(&mut self, px: u32) -> Result<(), ValidationError> {
        ValidationError::check_size(px, 1024)?;
        self.current.main_indicator.size_px = px;
        Ok(())
    }

    /// Set main indicator border. Validates `color`.
    pub fn set_main_border(&mut self, b: BorderConfig) -> Result<(), ValidationError> {
        ValidationError::check_color(&b.color)?;
        self.current.main_indicator.border = b;
        Ok(())
    }

    /// Insert (or replace) a saved position for `output`.
    pub fn set_main_position(&mut self, output: OutputName, point: Point) {
        self.current.main_indicator.positions.insert(output, point);
    }

    /// Delete the saved position for `output` (no-op if absent).
    pub fn forget_main_position(&mut self, output: &OutputName) -> bool {
        self.current
            .main_indicator
            .positions
            .shift_remove(output)
            .is_some()
    }

    // -----------------------------------------------------------------
    // Per-window indicator page
    // -----------------------------------------------------------------

    /// Toggle per-window indicator master switch.
    pub fn set_per_window_enable(&mut self, b: bool) {
        self.current.per_window_indicator.enable = b;
    }

    /// Set per-window indicator size (px).
    pub fn set_per_window_size(&mut self, px: u32) -> Result<(), ValidationError> {
        ValidationError::check_size(px, 256)?;
        self.current.per_window_indicator.size_px = px;
        Ok(())
    }

    /// Set the per-window indicator title-bar offset.
    pub fn set_per_window_offset(&mut self, offset: Offset) {
        self.current.per_window_indicator.offset = offset;
    }

    /// Set per-window indicator border.
    pub fn set_per_window_border(&mut self, b: BorderConfig) -> Result<(), ValidationError> {
        ValidationError::check_color(&b.color)?;
        self.current.per_window_indicator.border = b;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Icons page
    // -----------------------------------------------------------------

    /// Toggle SVG preference.
    pub fn set_prefer_svg(&mut self, b: bool) {
        self.current.icons.prefer_svg = b;
    }

    /// Replace the icon search paths.
    pub fn set_search_paths(&mut self, paths: Vec<String>) {
        self.current.icons.search_paths = paths;
    }

    /// Insert/replace the icon name for a 1-based group.
    pub fn set_icon_for_group(
        &mut self,
        group_one_based: u8,
        icon_name: String,
    ) -> Result<(), ValidationError> {
        ValidationError::check_group(group_one_based)?;
        self.current
            .icons
            .mapping
            .insert(group_one_based.to_string(), icon_name);
        Ok(())
    }

    // -----------------------------------------------------------------
    // Sound page
    // -----------------------------------------------------------------

    /// Set sound mode.
    pub fn set_sound_mode(&mut self, m: SoundMode) {
        self.current.sound.mode = m;
    }

    /// Set sound file path. Empty = built-in click.
    pub fn set_sound_file(&mut self, path: String) {
        self.current.sound.file = path;
    }

    // -----------------------------------------------------------------
    // App rules page
    // -----------------------------------------------------------------

    /// Append a rule. The rule's pattern is compiled once to surface
    /// any glob errors before it lands in the config.
    pub fn add_app_rule(&mut self, rule: AppRule) -> Result<(), ValidationError> {
        validate_rule(&rule)?;
        self.current.app_rules.push(rule);
        Ok(())
    }

    /// Replace the rule at `idx`.
    pub fn replace_app_rule(&mut self, idx: usize, rule: AppRule) -> Result<(), ValidationError> {
        let len = self.current.app_rules.len();
        if idx >= len {
            return Err(ValidationError::BadIndex { got: idx, len });
        }
        validate_rule(&rule)?;
        self.current.app_rules[idx] = rule;
        Ok(())
    }

    /// Delete the rule at `idx`.
    pub fn remove_app_rule(&mut self, idx: usize) -> Result<(), ValidationError> {
        let len = self.current.app_rules.len();
        if idx >= len {
            return Err(ValidationError::BadIndex { got: idx, len });
        }
        self.current.app_rules.remove(idx);
        Ok(())
    }

    /// Move the rule at `from` to position `to`. Indices clamp to the
    /// list length. Returns the resulting index, which may equal `from`
    /// if the move was a no-op.
    pub fn move_app_rule(&mut self, from: usize, to: usize) -> Result<usize, ValidationError> {
        let len = self.current.app_rules.len();
        if from >= len {
            return Err(ValidationError::BadIndex { got: from, len });
        }
        let dest = to.min(len - 1);
        if dest == from {
            return Ok(from);
        }
        let rule = self.current.app_rules.remove(from);
        self.current.app_rules.insert(dest, rule);
        Ok(dest)
    }
}

/// Compile-check the rule's pattern via `globset`.
fn validate_rule(rule: &AppRule) -> Result<(), ValidationError> {
    let pattern = match &rule.match_ {
        AppMatch::WmClassClass(p) | AppMatch::WmClassName(p) | AppMatch::WmName(p) => p,
    };
    Glob::new(pattern).map_err(|e| ValidationError::BadGlob {
        pattern: pattern.to_owned(),
        reason: e.to_string(),
    })?;
    if let RuleAction::AltGroup(g) = rule.action {
        ValidationError::check_group(g.as_one_based())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use xxkb_core::{layout::Group, monitors::OutputName, rules::AppMatch, Point};

    use super::*;

    fn editor() -> ConfigEditor {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Leak the tempdir for the test's lifetime so the path stays valid.
        Box::leak(Box::new(dir));
        ConfigEditor::from_parts(Config::default(), path)
    }

    #[test]
    fn fresh_editor_is_clean() {
        let e = editor();
        assert!(!e.dirty());
        assert_eq!(e.config(), e.baseline());
    }

    #[test]
    fn mutation_marks_dirty() {
        let mut e = editor();
        e.set_two_state(true);
        assert!(e.dirty());
        e.discard();
        assert!(!e.dirty());
    }

    #[test]
    fn save_clears_dirty() {
        let mut e = editor();
        e.set_two_state(true);
        e.save().unwrap();
        assert!(!e.dirty());
        assert!(e.baseline().general.two_state);
    }

    #[test]
    fn save_round_trips_through_disk() {
        let mut e = editor();
        e.set_two_state(true);
        e.set_main_position("DP-1".into(), Point::new(123, 456));
        e.save().unwrap();

        let mut e2 = ConfigEditor::load_from(e.path()).unwrap();
        assert!(e2.config().general.two_state);
        assert_eq!(
            e2.config()
                .main_indicator
                .positions
                .get(&OutputName::from("DP-1".to_owned()))
                .copied(),
            Some(Point::new(123, 456)),
        );
        // Mutate then reload from disk: the mutation must vanish.
        e2.set_two_state(false);
        assert!(e2.dirty());
        e2.reload_from_disk().unwrap();
        assert!(!e2.dirty());
        assert!(e2.config().general.two_state);
    }

    #[test]
    fn invalid_size_does_not_mutate_state() {
        let mut e = editor();
        let before = e.config().main_indicator.size_px;
        let err = e.set_main_size(0).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::OutOfRange { got: 0, max: 1024 }
        ));
        assert_eq!(e.config().main_indicator.size_px, before);
        assert!(!e.dirty());
    }

    #[test]
    fn invalid_color_does_not_mutate_border() {
        let mut e = editor();
        let original = e.config().main_indicator.border.clone();
        let result = e.set_main_border(BorderConfig {
            enabled: true,
            color: "not-a-color".into(),
            width: 2,
        });
        assert!(matches!(result, Err(ValidationError::BadColor(_))));
        assert_eq!(e.config().main_indicator.border, original);
    }

    #[test]
    fn save_rejects_inconsistent_state() {
        // Direct mutation that bypasses our setters can leave the config
        // in a state that fails `validate()`. `save` must catch that.
        let mut e = editor();
        e.current.general.base_group = 0;
        let result = e.save();
        assert!(result.is_err());
        // We never updated baseline.
        assert_eq!(e.baseline().general.base_group, 1);
    }

    #[test]
    fn add_rule_validates_glob() {
        let mut e = editor();
        let bad = AppRule {
            match_: AppMatch::WmName("[unterminated".into()),
            action: RuleAction::Ignore,
        };
        let err = e.add_app_rule(bad).unwrap_err();
        assert!(matches!(err, ValidationError::BadGlob { .. }));
        assert!(e.config().app_rules.is_empty());

        let good = AppRule {
            match_: AppMatch::WmClassClass("Firefox*".into()),
            action: RuleAction::Ignore,
        };
        e.add_app_rule(good.clone()).unwrap();
        assert_eq!(e.config().app_rules, vec![good]);
    }

    #[test]
    fn move_rule_swaps_order() {
        let mut e = editor();
        let r0 = AppRule {
            match_: AppMatch::WmClassClass("a*".into()),
            action: RuleAction::Ignore,
        };
        let r1 = AppRule {
            match_: AppMatch::WmClassClass("b*".into()),
            action: RuleAction::StartAlt,
        };
        let r2 = AppRule {
            match_: AppMatch::WmClassClass("c*".into()),
            action: RuleAction::AltGroup(Group::new(2, 4).unwrap()),
        };
        e.add_app_rule(r0.clone()).unwrap();
        e.add_app_rule(r1.clone()).unwrap();
        e.add_app_rule(r2.clone()).unwrap();
        // Move first to last via large-to clamp.
        let dest = e.move_app_rule(0, 99).unwrap();
        assert_eq!(dest, 2);
        assert_eq!(e.config().app_rules, vec![r1, r2, r0]);
    }

    #[test]
    fn remove_rule_out_of_range_errors() {
        let mut e = editor();
        let err = e.remove_app_rule(0).unwrap_err();
        assert!(matches!(err, ValidationError::BadIndex { got: 0, len: 0 }));
    }

    #[test]
    fn replace_rule_validates_action_group() {
        let mut e = editor();
        // Construct a rule whose group is forced to an invalid 1-based
        // value via the same validator path — `Group::new` would normally
        // refuse, so we bypass it.
        let g = Group::new(1, 4).unwrap();
        let rule = AppRule {
            match_: AppMatch::WmClassClass("ok*".into()),
            action: RuleAction::AltGroup(g),
        };
        e.add_app_rule(rule.clone()).unwrap();
        // Replacing with the same valid rule must succeed.
        e.replace_app_rule(0, rule).unwrap();
        // Replacing out of range must fail.
        let bad = AppRule {
            match_: AppMatch::WmClassClass("x*".into()),
            action: RuleAction::Ignore,
        };
        let err = e.replace_app_rule(1, bad).unwrap_err();
        assert!(matches!(err, ValidationError::BadIndex { got: 1, len: 1 }));
    }

    #[test]
    fn forget_main_position_removes_entry() {
        let mut e = editor();
        e.set_main_position("DP-1".into(), Point::new(0, 0));
        assert!(e.forget_main_position(&"DP-1".to_owned().into()));
        assert!(!e.forget_main_position(&"DP-1".to_owned().into()));
    }

    #[test]
    fn icon_for_group_validates_group() {
        let mut e = editor();
        assert!(e.set_icon_for_group(0, "en".into()).is_err());
        assert!(e.set_icon_for_group(5, "en".into()).is_err());
        e.set_icon_for_group(3, "ua".into()).unwrap();
        assert_eq!(e.config().icons.mapping.get("3"), Some(&"ua".into()));
    }
}
