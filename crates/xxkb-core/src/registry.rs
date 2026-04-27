//! Per-window remembered layout.
//!
//! Mirrors the legacy xxkb behaviour:
//!
//! > xxkb remembers the current layout in each application and switches to
//! > it on the focus change.
//!
//! Implementation is just a hashmap keyed by `WindowId` (an opaque newtype
//! around the X11 window id). All eviction / forgetting is explicit.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::layout::Group;

/// Opaque X11 window id.
///
/// We don't depend on `x11rb` here so that `xxkb-core` stays I/O-free; the
/// daemon will convert from `x11rb::protocol::xproto::Window` when calling in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowId(pub u32);

impl From<u32> for WindowId {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

/// What we remember per window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RememberedLayout {
    /// The group to restore when this window receives focus again.
    pub group: Group,
    /// Whether the user has explicitly excluded this window from management
    /// (see legacy `XXkb.controls.button_delete_and_forget`).
    pub forgotten: bool,
}

/// The window registry.
#[derive(Debug, Default, Clone)]
pub struct WindowRegistry {
    inner: HashMap<WindowId, RememberedLayout>,
}

impl WindowRegistry {
    /// Build an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of tracked windows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if no windows are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Remember a layout for `wid`. If the window was previously forgotten,
    /// this resurrects it.
    pub fn remember(&mut self, wid: WindowId, group: Group) {
        self.inner.insert(
            wid,
            RememberedLayout {
                group,
                forgotten: false,
            },
        );
    }

    /// Look up the remembered layout (returns `None` if forgotten or unseen).
    #[must_use]
    pub fn get(&self, wid: WindowId) -> Option<Group> {
        self.inner
            .get(&wid)
            .filter(|r| !r.forgotten)
            .map(|r| r.group)
    }

    /// Stop tracking `wid` entirely.
    pub fn drop_window(&mut self, wid: WindowId) {
        self.inner.remove(&wid);
    }

    /// Remember the window but flag it as "do not restore layout for it".
    /// Equivalent to legacy `button_delete_and_forget`.
    pub fn forget(&mut self, wid: WindowId) {
        self.inner.entry(wid).and_modify(|r| r.forgotten = true);
    }

    /// Iterate over `(window, remembered)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (WindowId, RememberedLayout)> + '_ {
        self.inner.iter().map(|(&w, &r)| (w, r))
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
    fn remember_and_lookup() {
        let mut r = WindowRegistry::new();
        r.remember(WindowId(1), g(2));
        assert_eq!(r.get(WindowId(1)), Some(g(2)));
        assert_eq!(r.get(WindowId(2)), None);
    }

    #[test]
    fn forget_hides_window_but_keeps_entry() {
        let mut r = WindowRegistry::new();
        r.remember(WindowId(1), g(2));
        r.forget(WindowId(1));
        assert_eq!(r.get(WindowId(1)), None);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn re_remember_resurrects_forgotten() {
        let mut r = WindowRegistry::new();
        r.remember(WindowId(1), g(2));
        r.forget(WindowId(1));
        r.remember(WindowId(1), g(0));
        assert_eq!(r.get(WindowId(1)), Some(g(0)));
    }

    #[test]
    fn drop_window_removes_completely() {
        let mut r = WindowRegistry::new();
        r.remember(WindowId(1), g(2));
        r.drop_window(WindowId(1));
        assert_eq!(r.len(), 0);
    }
}
