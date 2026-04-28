//! Real on-disk-style `~/.xxkbrc` fixtures (`tests/fixtures/*.xxkbrc`).
//! These complement the synthetic strings embedded in `lib.rs` unit tests.

use xxkb_config::{ModifierName, SoundMode};
use xxkb_core::rules::RuleAction;

#[test]
fn fixture_minimal_validates() {
    let raw = include_str!("fixtures/minimal.xxkbrc");
    let cfg = xxkb_migrate::migrate_string(raw);
    assert_eq!(cfg.general.base_group, 1);
    assert_eq!(cfg.general.alt_group, 3);
    cfg.validate()
        .expect("minimal fixture should produce a valid Config");
}

#[test]
fn fixture_debian_hints_round_trips_toml() {
    let raw = include_str!("fixtures/debian_hints.xxkbrc");
    let cfg = xxkb_migrate::migrate_string(raw);
    assert_eq!(cfg.general.base_group, 1);
    assert_eq!(cfg.general.alt_group, 2);
    assert!(cfg.general.two_state);
    assert_eq!(cfg.general.cycle_modifier, ModifierName::Alt);
    assert!(cfg.general.ignore_reverse);
    assert_eq!(cfg.main_indicator.size_px, 32);
    assert!(cfg.main_indicator.border.enabled);
    assert_eq!(cfg.sound.mode, SoundMode::Off);
    assert_eq!(cfg.per_window_indicator.size_px, 14);
    assert_eq!(cfg.per_window_indicator.offset.x, -55);
    assert_eq!(cfg.per_window_indicator.offset.y, -5);
    assert_eq!(cfg.app_rules.len(), 3);

    cfg.validate().expect("fixture should validate");

    let tmpdir = tempfile::tempdir().unwrap();
    let path = tmpdir.path().join("out.toml");
    cfg.save_to(&path).unwrap();
    let reloaded = xxkb_config::Config::load_from(&path).unwrap();
    assert_eq!(cfg, reloaded);
}

#[test]
fn fixture_debian_hints_rule_actions() {
    let raw = include_str!("fixtures/debian_hints.xxkbrc");
    let cfg = xxkb_migrate::migrate_string(raw);
    assert!(matches!(cfg.app_rules[0].action, RuleAction::Ignore));
    assert!(matches!(cfg.app_rules[1].action, RuleAction::Ignore));
    assert!(matches!(cfg.app_rules[2].action, RuleAction::StartAlt));
}
