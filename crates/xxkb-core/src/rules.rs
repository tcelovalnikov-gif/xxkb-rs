//! App-specific rules.
//!
//! The legacy xxkb supported `XXkb.app_list.<property>.<action>: <patterns>`
//! options — see [legacy man page](https://github.com/uliscat/xxkb/blob/main/xxkb.man).
//! We model the same concept but in a more uniform way: each rule is an
//! `AppRule` consisting of an `AppMatch` (pattern set) and a `RuleAction`.

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::{layout::Group, CoreError};

/// A snapshot of properties for a single window — what we match against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowProps {
    /// Class part of `WM_CLASS` (typically the application's "official" name,
    /// e.g. "Firefox" or "Gnome-terminal").
    pub wm_class_class: String,
    /// Instance part of `WM_CLASS` (e.g. "firefox", "gnome-terminal").
    pub wm_class_name: String,
    /// `WM_NAME` (the window title).
    pub wm_name: String,
}

/// What part of `WindowProps` a rule matches against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppMatch {
    /// Match against `WM_CLASS.class`.
    WmClassClass(String),
    /// Match against `WM_CLASS.name`.
    WmClassName(String),
    /// Match against `WM_NAME`.
    WmName(String),
}

impl AppMatch {
    fn pattern(&self) -> &str {
        match self {
            Self::WmClassClass(p) | Self::WmClassName(p) | Self::WmName(p) => p,
        }
    }
}

/// What we should do when a rule matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    /// Don't manage this window at all (no per-window indicator, no
    /// remembered layout). Mirrors legacy `app_list.*.ignore`.
    Ignore,
    /// Switch to the configured `alt_group` when this window first appears.
    /// Mirrors legacy `app_list.*.start_alt`.
    StartAlt,
    /// Use a specific group as alt for this window.
    /// Mirrors legacy `app_list.*.alt_groupN`.
    AltGroup(Group),
}

/// A single configured rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppRule {
    /// What to match against.
    pub match_: AppMatch,
    /// What to do on match.
    pub action: RuleAction,
}

/// Compiled set of rules. Patterns are compiled once at config load time;
/// matching is then a tight loop with no allocations.
pub struct AppRules {
    rules: Vec<CompiledRule>,
    /// If set, the meaning of [`RuleAction::Ignore`] is inverted: only
    /// matched windows are managed (mirrors legacy `XXkb.ignore.reverse`).
    pub ignore_reverse: bool,
}

struct CompiledRule {
    matcher: CompiledMatcher,
    action: RuleAction,
}

enum CompiledMatcher {
    WmClassClass(GlobMatcher),
    WmClassName(GlobMatcher),
    WmName(GlobMatcher),
}

impl CompiledMatcher {
    fn matches(&self, props: &WindowProps) -> bool {
        match self {
            Self::WmClassClass(g) => g.is_match(&props.wm_class_class),
            Self::WmClassName(g) => g.is_match(&props.wm_class_name),
            Self::WmName(g) => g.is_match(&props.wm_name),
        }
    }
}

/// Verdict for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Window is managed normally.
    Manage,
    /// Don't track or decorate this window.
    Ignore,
    /// Manage, but immediately switch to the alt group.
    StartAlt,
    /// Manage and use this specific group as the alt for this window.
    AltGroup(Group),
}

impl AppRules {
    /// Build from a slice of `AppRule`s. Returns an error if any glob
    /// pattern is invalid.
    pub fn build(rules: &[AppRule], ignore_reverse: bool) -> Result<Self, CoreError> {
        let mut compiled = Vec::with_capacity(rules.len());
        for r in rules {
            let glob = Glob::new(r.match_.pattern())
                .map_err(|e| CoreError::BadGlob {
                    pattern: r.match_.pattern().to_owned(),
                    source: e,
                })?
                .compile_matcher();
            let matcher = match &r.match_ {
                AppMatch::WmClassClass(_) => CompiledMatcher::WmClassClass(glob),
                AppMatch::WmClassName(_) => CompiledMatcher::WmClassName(glob),
                AppMatch::WmName(_) => CompiledMatcher::WmName(glob),
            };
            compiled.push(CompiledRule {
                matcher,
                action: r.action,
            });
        }
        Ok(Self {
            rules: compiled,
            ignore_reverse,
        })
    }

    /// True if there are no rules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Decide what to do with `props`.
    ///
    /// First match wins. If `ignore_reverse` is on, windows that match no
    /// rule are themselves ignored (so `Ignore` rules become `Manage`).
    #[must_use]
    pub fn verdict(&self, props: &WindowProps) -> Verdict {
        for r in &self.rules {
            if r.matcher.matches(props) {
                return match r.action {
                    RuleAction::Ignore => {
                        if self.ignore_reverse {
                            Verdict::Manage
                        } else {
                            Verdict::Ignore
                        }
                    }
                    RuleAction::StartAlt => Verdict::StartAlt,
                    RuleAction::AltGroup(g) => Verdict::AltGroup(g),
                };
            }
        }
        if self.ignore_reverse && !self.rules.is_empty() {
            // Reverse mode + no match means we ignore.
            Verdict::Ignore
        } else {
            Verdict::Manage
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn props(class: &str, instance: &str, name: &str) -> WindowProps {
        WindowProps {
            wm_class_class: class.into(),
            wm_class_name: instance.into(),
            wm_name: name.into(),
        }
    }

    fn r_ignore_class(pattern: &str) -> AppRule {
        AppRule {
            match_: AppMatch::WmClassClass(pattern.into()),
            action: RuleAction::Ignore,
        }
    }

    #[test]
    fn glob_matches_class() {
        let rs = AppRules::build(&[r_ignore_class("Firefox*")], false).unwrap();
        assert_eq!(
            rs.verdict(&props("Firefox", "Navigator", "")),
            Verdict::Ignore
        );
        assert_eq!(rs.verdict(&props("Firefox-esr", "x", "")), Verdict::Ignore);
        assert_eq!(rs.verdict(&props("Chromium", "x", "")), Verdict::Manage);
    }

    #[test]
    fn first_match_wins() {
        let rules = vec![
            AppRule {
                match_: AppMatch::WmName("*clock*".into()),
                action: RuleAction::Ignore,
            },
            AppRule {
                match_: AppMatch::WmClassClass("Firefox".into()),
                action: RuleAction::StartAlt,
            },
        ];
        let rs = AppRules::build(&rules, false).unwrap();
        // matches first rule (clock in window name)
        assert_eq!(
            rs.verdict(&props("Firefox", "", "clockwise")),
            Verdict::Ignore
        );
        // matches second rule
        assert_eq!(
            rs.verdict(&props("Firefox", "", "Reddit")),
            Verdict::StartAlt
        );
    }

    #[test]
    fn ignore_reverse_inverts_meaning() {
        let rs = AppRules::build(&[r_ignore_class("term*")], true).unwrap();
        // Match — would be ignore, but with reverse it's manage.
        assert_eq!(rs.verdict(&props("terminator", "", "")), Verdict::Manage);
        // No match — would be manage, but with reverse it's ignore.
        assert_eq!(rs.verdict(&props("Firefox", "", "")), Verdict::Ignore);
    }

    #[test]
    fn alt_group_action_carries_group() {
        let rs = AppRules::build(
            &[AppRule {
                match_: AppMatch::WmClassClass("Firefox".into()),
                action: RuleAction::AltGroup(Group::new(2, 4).unwrap()),
            }],
            false,
        )
        .unwrap();
        assert_eq!(
            rs.verdict(&props("Firefox", "", "")),
            Verdict::AltGroup(Group::new(2, 4).unwrap())
        );
    }

    #[test]
    fn bad_glob_returns_error() {
        let rules = vec![r_ignore_class("[unbalanced")];
        let err = AppRules::build(&rules, false).err().unwrap();
        assert!(matches!(err, CoreError::BadGlob { .. }));
    }
}
