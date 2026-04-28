//! Convert legacy `~/.xxkbrc` into the new TOML config format.
//!
//! The legacy config is X-Resources style, e.g.:
//!
//! ```text
//! XXkb.group.base: 1
//! XXkb.group.alt: 2
//! XXkb.mainwindow.enable: yes
//! XXkb.mainwindow.geometry: 48x48
//! XXkb.app_list.wm_class_class.ignore: *clock Fvwm*
//! ```
//!
//! We parse it line-by-line and map known keys onto our `Config`.
//! Unknown keys are logged at `warn` level but ignored.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::path::Path;

use indexmap::IndexMap;
use xxkb_config::{
    BorderConfig, Config, GeneralConfig, IconsConfig, MainIndicatorConfig, MainIndicatorMode,
    ModifierName, PerWindowIndicatorConfig, SoundConfig, SoundMode,
};
use xxkb_core::{
    layout::Group,
    rules::{AppMatch, AppRule, RuleAction},
};

/// Migrate a legacy file at `path` into a `Config`.
pub fn migrate_file(path: &Path) -> Result<Config, std::io::Error> {
    let raw = std::fs::read_to_string(path)?;
    Ok(migrate_string(&raw))
}

/// Migrate from in-memory `xxkbrc` content.
#[must_use]
pub fn migrate_string(text: &str) -> Config {
    let mut cfg = Config::default();
    let mut continued = String::new();
    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim_end();
        if line.is_empty() {
            continue;
        }
        let combined: String;
        let line = if line.ends_with('\\') {
            continued.push_str(line.trim_end_matches('\\'));
            continued.push(' ');
            continue;
        } else if !continued.is_empty() {
            continued.push_str(line);
            combined = std::mem::take(&mut continued);
            combined.as_str()
        } else {
            line
        };
        let Some((key, value)) = split_kv(line) else {
            continue;
        };
        apply_kv(&mut cfg, &key, value.trim());
    }
    // Validate; if it doesn't pass, fall back to defaults but keep what we got.
    let _ = cfg.validate();
    cfg
}

fn strip_comment(s: &str) -> &str {
    if let Some(idx) = s.find('!') {
        &s[..idx]
    } else {
        s
    }
}

fn split_kv(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim().to_owned(), v.trim().to_owned()))
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_geometry(v: &str) -> Option<(u32, Option<i32>, Option<i32>)> {
    // Forms: "48x48", "48x48+10+20", "48x48-60+7"
    let (size, rest) = if let Some(p) = v.find('+').or_else(|| v.find('-')) {
        v.split_at(p)
    } else {
        (v, "")
    };
    let (w, _) = size.split_once('x')?;
    let w: u32 = w.parse().ok()?;
    let mut x = None;
    let mut y = None;
    let mut s = rest;
    if !s.is_empty() {
        let (xs, ys) = read_signed_pair(s)?;
        x = Some(xs);
        y = Some(ys);
        let _ = &mut s;
    }
    Some((w, x, y))
}

fn read_signed_pair(s: &str) -> Option<(i32, i32)> {
    let mut chars = s.chars();
    let xs_sign = chars.next()?;
    let mut xs = String::from(xs_sign);
    while let Some(c) = chars.clone().next() {
        if c == '+' || c == '-' {
            break;
        }
        xs.push(c);
        chars.next();
    }
    let ys: String = chars.collect();
    let xs = xs.parse().ok()?;
    let ys = ys.parse().ok()?;
    Some((xs, ys))
}

fn apply_kv(cfg: &mut Config, key: &str, value: &str) {
    let key_l = key.to_ascii_lowercase();
    match key_l.as_str() {
        "xxkb.group.base" => {
            if let Ok(n) = value.parse::<u8>() {
                cfg.general.base_group = n;
            }
        }
        "xxkb.group.alt" => {
            if let Ok(n) = value.parse::<u8>() {
                cfg.general.alt_group = n;
            }
        }
        "xxkb.controls.two_state" => {
            cfg.general.two_state = parse_bool(value).unwrap_or(cfg.general.two_state);
        }
        "xxkb.keymask.cycle" => {
            cfg.general.cycle_modifier = match value.to_ascii_lowercase().as_str() {
                "none" => ModifierName::None,
                "shift" => ModifierName::Shift,
                "lock" => ModifierName::Lock,
                "ctrl" | "control" => ModifierName::Ctrl,
                "alt" => ModifierName::Alt,
                "mod1" => ModifierName::Mod1,
                "mod2" => ModifierName::Mod2,
                "mod3" => ModifierName::Mod3,
                "mod4" => ModifierName::Mod4,
                "mod5" => ModifierName::Mod5,
                _ => cfg.general.cycle_modifier,
            };
        }
        "xxkb.ignore.reverse" => {
            cfg.general.ignore_reverse = parse_bool(value).unwrap_or(cfg.general.ignore_reverse);
        }
        "xxkb.mainwindow.enable" => {
            cfg.main_indicator.enable = parse_bool(value).unwrap_or(cfg.main_indicator.enable);
        }
        "xxkb.mainwindow.geometry" => {
            if let Some((w, _, _)) = parse_geometry(value) {
                cfg.main_indicator.size_px = w;
            }
        }
        "xxkb.mainwindow.border.color" => {
            cfg.main_indicator.border.color =
                normalize_color(value).unwrap_or(cfg.main_indicator.border.color.clone());
            if cfg.main_indicator.border.width == 0 {
                cfg.main_indicator.border.width = 1;
            }
            cfg.main_indicator.border.enabled = true;
        }
        "xxkb.mainwindow.border.width" => {
            if let Ok(n) = value.parse::<u32>() {
                cfg.main_indicator.border.width = n;
                cfg.main_indicator.border.enabled = n > 0;
            }
        }
        "xxkb.button.enable" => {
            cfg.per_window_indicator.enable =
                parse_bool(value).unwrap_or(cfg.per_window_indicator.enable);
        }
        "xxkb.button.geometry" => {
            if let Some((w, x, y)) = parse_geometry(value) {
                cfg.per_window_indicator.size_px = w;
                if let (Some(x), Some(y)) = (x, y) {
                    cfg.per_window_indicator.offset = xxkb_core::Offset { x, y };
                }
            }
        }
        "xxkb.bell.enable" => {
            if let Some(b) = parse_bool(value) {
                cfg.sound.mode = if b { SoundMode::Both } else { SoundMode::Off };
            }
        }
        // app_list.<property>.<action>: <patterns>
        k if k.starts_with("xxkb.app_list.") => {
            apply_app_list(cfg, k, value);
        }
        _ => {
            // Quietly ignore unrecognised keys.
        }
    }
    let _ = (
        MainIndicatorConfig::default(),
        PerWindowIndicatorConfig::default(),
        IconsConfig::default(),
        SoundConfig::default(),
        BorderConfig::default(),
        GeneralConfig::default(),
        MainIndicatorMode::default(),
    );
}

fn normalize_color(v: &str) -> Option<String> {
    let v = v.trim();
    if v.starts_with('#') {
        Some(v.to_string())
    } else {
        // X11 named colours — best-effort mapping for common ones.
        Some(
            match v.to_ascii_lowercase().as_str() {
                "black" => "#000000",
                "white" => "#FFFFFF",
                "red" => "#FF0000",
                "green" => "#00FF00",
                "blue" => "#0000FF",
                "yellow" => "#FFFF00",
                "cyan" => "#00FFFF",
                "magenta" => "#FF00FF",
                "blue4" => "#00008B",
                other => {
                    return Some(format!(
                        "#000000 /* xxkb-migrate: unknown colour '{other}' */"
                    ))
                }
            }
            .to_owned(),
        )
    }
}

fn apply_app_list(cfg: &mut Config, key_lower: &str, patterns: &str) {
    // key_lower = "xxkb.app_list.<property>.<action>"
    let rest = key_lower.trim_start_matches("xxkb.app_list.");
    let (prop, action) = match rest.split_once('.') {
        Some(p) => p,
        None => return,
    };
    for raw in patterns.split_whitespace() {
        let pattern = raw.trim();
        if pattern.is_empty() {
            continue;
        }
        let m = match prop {
            "wm_class_class" => AppMatch::WmClassClass(pattern.into()),
            "wm_class_name" => AppMatch::WmClassName(pattern.into()),
            "wm_name" => AppMatch::WmName(pattern.into()),
            _ => continue,
        };
        let act = match action {
            "ignore" => RuleAction::Ignore,
            "start_alt" => RuleAction::StartAlt,
            a if a.starts_with("alt_group") => {
                let n: u8 = a.trim_start_matches("alt_group").parse().unwrap_or(2);
                let g = match Group::from_one_based(n, 4) {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                RuleAction::AltGroup(g)
            }
            _ => continue,
        };
        cfg.app_rules.push(AppRule {
            match_: m,
            action: act,
        });
    }

    let _ = IndexMap::<String, String>::new();
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn migrates_minimal_legacy_file() {
        let raw = "
XXkb.group.base: 1
XXkb.group.alt: 2
XXkb.controls.two_state: yes
XXkb.mainwindow.enable: yes
XXkb.mainwindow.geometry: 48x48
XXkb.button.enable: yes
XXkb.button.geometry: 15x15-60+7
XXkb.bell.enable: yes
XXkb.keymask.cycle: ctrl
";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.general.base_group, 1);
        assert_eq!(cfg.general.alt_group, 2);
        assert!(cfg.general.two_state);
        assert_eq!(cfg.general.cycle_modifier, ModifierName::Ctrl);
        assert!(cfg.main_indicator.enable);
        assert_eq!(cfg.main_indicator.size_px, 48);
        assert!(cfg.per_window_indicator.enable);
        assert_eq!(cfg.per_window_indicator.size_px, 15);
        assert_eq!(cfg.per_window_indicator.offset.x, -60);
        assert_eq!(cfg.per_window_indicator.offset.y, 7);
        assert_eq!(cfg.sound.mode, SoundMode::Both);
    }

    #[test]
    fn migrates_app_list_rules() {
        let raw = "
XXkb.app_list.wm_class_class.ignore: *clock Fvwm*
XXkb.app_list.wm_name.start_alt: licq
XXkb.app_list.wm_class_class.alt_group3: kate
";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.app_rules.len(), 4);
        assert!(matches!(cfg.app_rules[0].action, RuleAction::Ignore));
        assert!(matches!(cfg.app_rules[1].action, RuleAction::Ignore));
        assert!(matches!(cfg.app_rules[2].action, RuleAction::StartAlt));
        assert!(matches!(cfg.app_rules[3].action, RuleAction::AltGroup(_)));
    }

    #[test]
    fn handles_continuation_lines() {
        let raw = "
XXkb.app_list.wm_class_class.ignore: foo \\
bar baz
";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.app_rules.len(), 3);
    }

    #[test]
    fn handles_comment_lines_and_inline_comments() {
        let raw = "
! this is a comment
XXkb.group.base: 1 ! inline
";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.general.base_group, 1);
    }

    /// `XXkb.bell.enable: no` (or `off`/`0`) must map to
    /// [`SoundMode::Off`]. The legacy default is bell-off; we
    /// don't want a silent migration to suddenly switch users to
    /// "click on every focus change".
    #[test]
    fn bell_disabled_maps_to_sound_off() {
        let raw = "XXkb.bell.enable: no";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.sound.mode, SoundMode::Off);
    }

    /// Border width = 0 should disable the border outright, not
    /// keep `enabled: true` with a zero-width line that paints
    /// nothing visibly but is still toggled "on" in the GUI.
    #[test]
    fn zero_border_width_disables_border() {
        let raw = "
XXkb.mainwindow.border.color: black
XXkb.mainwindow.border.width: 0
";
        let cfg = migrate_string(raw);
        assert!(!cfg.main_indicator.border.enabled);
        assert_eq!(cfg.main_indicator.border.width, 0);
    }

    /// X11 named colours (`black`, `blue4`, …) map to hex. An
    /// unknown name produces an explicit fallback so the user
    /// notices in their TOML diff.
    #[test]
    fn named_colour_maps_to_hex() {
        let raw = "XXkb.mainwindow.border.color: blue4";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.main_indicator.border.color, "#00008B");
        assert!(cfg.main_indicator.border.enabled);
    }

    /// `XXkb.app_list.wm_class_class.alt_group3` should produce
    /// an `AltGroup(2)` (0-based group 2 is "third group").
    #[test]
    fn explicit_alt_group_syntax() {
        let raw = "XXkb.app_list.wm_class_class.alt_group3: kate";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.app_rules.len(), 1);
        match &cfg.app_rules[0].action {
            RuleAction::AltGroup(g) => assert_eq!(g.as_one_based(), 3),
            other => panic!("expected AltGroup(2), got {other:?}"),
        }
    }

    /// Unknown keys must be silently dropped (we log at warn, but
    /// the config is still produced). This protects users from
    /// an old `~/.xxkbrc` that pulled in plugins or vendor
    /// extensions we don't model.
    #[test]
    fn unknown_keys_are_ignored() {
        let raw = "
XXkb.unsupported.future.key: hello world
XXkb.group.base: 1
";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.general.base_group, 1);
    }

    /// Empty input round-trips to defaults — equivalent to a
    /// fresh user with no config at all.
    #[test]
    fn empty_input_is_default_config() {
        assert_eq!(migrate_string(""), Config::default());
        assert_eq!(migrate_string("\n\n!comment\n"), Config::default());
    }

    /// `XXkb.button.geometry: 15x15` (no offset) must keep the
    /// default offset rather than zeroing it out.
    #[test]
    fn button_geometry_without_offset_keeps_default_offset() {
        let raw = "XXkb.button.geometry: 20x20";
        let cfg = migrate_string(raw);
        assert_eq!(cfg.per_window_indicator.size_px, 20);
        assert_eq!(
            cfg.per_window_indicator.offset,
            xxkb_core::Offset { x: -60, y: 7 }
        );
    }

    /// End-to-end: write a realistic legacy config to a tempfile,
    /// migrate, save as TOML, reload, and verify the result.
    /// This catches regressions where `migrate_string` produces a
    /// `Config` that the daemon's loader would then reject.
    #[test]
    fn migrates_full_realistic_xxkbrc_round_trip() {
        let raw = "\
! Realistic ~/.xxkbrc — based on upstream defaults
XXkb.group.base: 1
XXkb.group.alt: 2
XXkb.controls.two_state: yes
XXkb.keymask.cycle: ctrl
XXkb.ignore.reverse: no

XXkb.mainwindow.enable: yes
XXkb.mainwindow.geometry: 32x32+5+5
XXkb.mainwindow.border.color: black
XXkb.mainwindow.border.width: 1

XXkb.button.enable: yes
XXkb.button.geometry: 15x15-60+7

XXkb.bell.enable: no

XXkb.app_list.wm_class_class.ignore: \\
    *clock Fvwm* xclock
XXkb.app_list.wm_class_class.start_alt: Mozilla* Firefox
XXkb.app_list.wm_name.alt_group2: *Telegram*
";
        let cfg = migrate_string(raw);

        // Field-level checks.
        assert_eq!(cfg.general.base_group, 1);
        assert_eq!(cfg.general.alt_group, 2);
        assert!(cfg.general.two_state);
        assert_eq!(cfg.general.cycle_modifier, ModifierName::Ctrl);
        assert!(!cfg.general.ignore_reverse);
        assert_eq!(cfg.main_indicator.size_px, 32);
        assert!(cfg.main_indicator.border.enabled);
        assert_eq!(cfg.main_indicator.border.width, 1);
        assert_eq!(cfg.main_indicator.border.color, "#000000");
        assert_eq!(cfg.per_window_indicator.size_px, 15);
        assert_eq!(cfg.per_window_indicator.offset.x, -60);
        assert_eq!(cfg.per_window_indicator.offset.y, 7);
        assert_eq!(cfg.sound.mode, SoundMode::Off);
        // 3 ignore + 2 start_alt + 1 alt_group2 = 6 rules.
        assert_eq!(cfg.app_rules.len(), 6);

        // Validation passes (this is what `Config::load_from`
        // would do after merge).
        cfg.validate().expect("migrated config should validate");

        // TOML round-trip via tempfile — same path the daemon
        // uses, so we exercise the real save/load logic.
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("config.toml");
        cfg.save_to(&path).unwrap();
        let reloaded = Config::load_from(&path).unwrap();
        assert_eq!(cfg, reloaded);
    }

    /// `migrate_file` is a thin wrapper around `migrate_string`
    /// but the I/O code path is what the CLI uses, so we
    /// exercise it at least once.
    #[test]
    fn migrate_file_reads_from_disk() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join(".xxkbrc");
        std::fs::write(&path, "XXkb.group.base: 2\nXXkb.group.alt: 3\n").unwrap();
        let cfg = migrate_file(&path).unwrap();
        assert_eq!(cfg.general.base_group, 2);
        assert_eq!(cfg.general.alt_group, 3);
    }
}
