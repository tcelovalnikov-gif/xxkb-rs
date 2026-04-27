//! Keyboard layout (XKB group) state machine.
//!
//! The X server keeps an *effective group* between 0 and 3 inclusive
//! (the XKB extension supports up to 4 groups). xxkb-rs exposes this
//! through [`Group`], which is internally `0..=3` but presented to users
//! 1-based (matching the legacy `XXkb.group.base` / `XXkb.group.alt`
//! configuration keys).

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// A 0-based XKB group in `0..max_groups`.
///
/// Use [`Group::from_one_based`] when reading configuration so the
/// 1-based representation that ships in TOML/legacy `xxkbrc` files
/// is normalized at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Group(u8);

impl Group {
    /// Build from a raw 0-based index, asserting it fits in `0..max`.
    pub fn new(idx: u8, max_groups: u8) -> Result<Self, CoreError> {
        if idx >= max_groups {
            return Err(CoreError::GroupOutOfRange {
                given: idx + 1,
                max: max_groups,
            });
        }
        Ok(Self(idx))
    }

    /// Build from a 1-based index (matching legacy config files).
    pub fn from_one_based(one_based: u8, max_groups: u8) -> Result<Self, CoreError> {
        if one_based == 0 || one_based > max_groups {
            return Err(CoreError::GroupOutOfRange {
                given: one_based,
                max: max_groups,
            });
        }
        Ok(Self(one_based - 1))
    }

    /// 0-based index, suitable for the X server.
    #[must_use]
    pub const fn as_index(self) -> u8 {
        self.0
    }

    /// 1-based index, suitable for display / config files.
    #[must_use]
    pub const fn as_one_based(self) -> u8 {
        self.0 + 1
    }
}

impl std::fmt::Display for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_one_based())
    }
}

/// Two-state mode configuration.
///
/// When `enabled`, only `base` and `alt` are cycled through; the X server
/// may still know about more groups, but xxkb-rs will skip them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoStateConfig {
    /// Master switch.
    pub enabled: bool,
    /// Primary group (typically the Latin/ASCII layout).
    pub base: Group,
    /// Alternative group.
    pub alt: Group,
}

impl Default for TwoStateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base: Group(0),
            alt: Group(1),
        }
    }
}

/// How a layout switch was initiated.
///
/// This drives the `sound.mode` selector: e.g. with mode = `manual_only`
/// we play sound on [`SwitchKind::Keyboard`] but ignore [`SwitchKind::Auto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchKind {
    /// User pressed the layout switch hotkey.
    Keyboard,
    /// User clicked one of our indicators.
    Click,
    /// Programmatic switch on focus change (we restored a remembered layout).
    Auto,
    /// Initial state at startup.
    Initial,
}

/// Mutable state of the keyboard.
///
/// This is intentionally just a struct; the daemon will own one of these
/// and feed it events from `xxkb-x11`.
#[derive(Debug, Clone)]
pub struct LayoutState {
    max_groups: u8,
    current: Group,
    previous: Group,
    two_state: TwoStateConfig,
}

impl LayoutState {
    /// Build a fresh state.
    pub fn new(max_groups: u8, initial: Group, two_state: TwoStateConfig) -> Self {
        Self {
            max_groups: max_groups.max(1),
            current: initial,
            previous: initial,
            two_state,
        }
    }

    /// How many groups the X server reported.
    #[must_use]
    pub const fn max_groups(&self) -> u8 {
        self.max_groups
    }

    /// Current group.
    #[must_use]
    pub const fn current(&self) -> Group {
        self.current
    }

    /// Previously active group (for "toggle to previous").
    #[must_use]
    pub const fn previous(&self) -> Group {
        self.previous
    }

    /// Get the two-state configuration.
    #[must_use]
    pub const fn two_state(&self) -> TwoStateConfig {
        self.two_state
    }

    /// Update the two-state configuration in place.
    pub fn set_two_state(&mut self, ts: TwoStateConfig) {
        self.two_state = ts;
    }

    /// Apply an external state change (one we observed from the X server).
    pub fn observe(&mut self, group: Group) {
        if group != self.current {
            self.previous = self.current;
            self.current = group;
        }
    }

    /// Compute the next group when the user issues a "cycle" command.
    ///
    /// Honours [`TwoStateConfig`]: if `two_state.enabled`, this just toggles
    /// `base` and `alt`. Otherwise we cycle through every group `0..max`.
    #[must_use]
    pub fn next_cycle(&self) -> Group {
        if self.two_state.enabled {
            if self.current == self.two_state.base {
                self.two_state.alt
            } else {
                self.two_state.base
            }
        } else {
            let next_idx = (self.current.as_index() + 1) % self.max_groups;
            Group(next_idx)
        }
    }

    /// Reverse cycle (useful if `XXkb.mousebutton.1.reverse` was set).
    #[must_use]
    pub fn prev_cycle(&self) -> Group {
        if self.two_state.enabled {
            self.next_cycle() // only two states, direction is irrelevant
        } else {
            let max = self.max_groups;
            let cur = self.current.as_index();
            let next_idx = if cur == 0 { max - 1 } else { cur - 1 };
            Group(next_idx)
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn g(i: u8) -> Group {
        Group::new(i, 4).unwrap()
    }

    #[test]
    fn group_one_based_roundtrip() {
        assert_eq!(Group::from_one_based(1, 4).unwrap(), g(0));
        assert_eq!(Group::from_one_based(4, 4).unwrap(), g(3));
        assert!(Group::from_one_based(0, 4).is_err());
        assert!(Group::from_one_based(5, 4).is_err());
    }

    #[test]
    fn cycle_without_two_state() {
        let mut s = LayoutState::new(3, g(0), TwoStateConfig::default());
        assert_eq!(s.next_cycle(), g(1));
        s.observe(g(1));
        assert_eq!(s.next_cycle(), g(2));
        s.observe(g(2));
        assert_eq!(s.next_cycle(), g(0));
    }

    #[test]
    fn cycle_in_two_state_skips_third_group() {
        let ts = TwoStateConfig {
            enabled: true,
            base: g(0),
            alt: g(2),
        };
        let mut s = LayoutState::new(4, g(0), ts);
        assert_eq!(s.next_cycle(), g(2));
        s.observe(g(2));
        assert_eq!(s.next_cycle(), g(0));
        // Even if the X server reports group 1 (e.g. xkbsymbols configured 4 layouts
        // but the user is on group 1 momentarily), our next_cycle returns base.
        s.observe(g(1));
        assert_eq!(s.next_cycle(), g(0));
    }

    #[test]
    fn observe_updates_previous_only_on_change() {
        let mut s = LayoutState::new(3, g(0), TwoStateConfig::default());
        s.observe(g(0));
        assert_eq!(s.previous(), g(0));
        s.observe(g(2));
        assert_eq!(s.current(), g(2));
        assert_eq!(s.previous(), g(0));
        s.observe(g(2));
        assert_eq!(s.previous(), g(0));
    }

    #[test]
    fn prev_cycle_wraps_around() {
        let mut s = LayoutState::new(3, g(0), TwoStateConfig::default());
        assert_eq!(s.prev_cycle(), g(2));
        s.observe(g(0));
        assert_eq!(s.prev_cycle(), g(2));
        s.observe(g(1));
        assert_eq!(s.prev_cycle(), g(0));
    }
}
