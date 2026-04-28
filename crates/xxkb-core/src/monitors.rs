//! Multi-monitor coordinate book-keeping.
//!
//! Indicator positions are stored keyed by RandR **output name** (e.g.
//! `DP-1`, `HDMI-1`, `eDP-1`), *not* by index. This means:
//! * unplugging a monitor and replugging it later restores the previous
//!   indicator position;
//! * reordering monitors in a multi-head setup doesn't move flags;
//! * a brand-new output gets a default position — by convention,
//!   bottom-right of that output's geometry, with a small inset.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Stable name of a RandR output.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutputName(pub String);

impl<S: Into<String>> From<S> for OutputName {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

impl std::ops::Deref for OutputName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

/// 2D point in *root window* coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    /// X coordinate.
    pub x: i32,
    /// Y coordinate.
    pub y: i32,
}

impl Point {
    /// Construct.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Rectangle (geometry of a single output), in root coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Top-left corner.
    pub origin: Point,
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
}

impl Rect {
    /// Construct.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            origin: Point::new(x, y),
            width,
            height,
        }
    }

    /// Right edge (exclusive).
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.origin.x + self.width as i32
    }

    /// Bottom edge (exclusive).
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.origin.y + self.height as i32
    }

    /// True if `p` is contained in this rectangle.
    #[must_use]
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x && p.x < self.right() && p.y >= self.origin.y && p.y < self.bottom()
    }
}

/// Snapshot of a single monitor as reported by RandR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Stable name (used as the persistence key).
    pub name: OutputName,
    /// Geometry in root coordinates.
    pub geometry: Rect,
    /// True for the RandR primary output.
    pub is_primary: bool,
    /// True if this output is currently connected and active.
    pub is_active: bool,
}

/// Persisted positions plus runtime view of the current set of outputs.
///
/// The struct supports two operations the daemon needs:
/// * lookup the indicator point for an output (with default fallback
///   if the output is new),
/// * record a new indicator position from a drag gesture.
#[derive(Debug, Clone, Default)]
pub struct MonitorLayout {
    /// Persisted indicator positions, keyed by output name.
    saved_positions: IndexMap<OutputName, Point>,
    /// Live snapshot of outputs reported by RandR.
    current: Vec<Output>,
}

impl MonitorLayout {
    /// Build with no outputs known yet.
    #[must_use]
    pub fn new(saved_positions: IndexMap<OutputName, Point>) -> Self {
        Self {
            saved_positions,
            current: Vec::new(),
        }
    }

    /// Replace the live view with what RandR reports now.
    pub fn update_outputs(&mut self, outputs: Vec<Output>) {
        self.current = outputs;
    }

    /// Iterate active outputs.
    pub fn active(&self) -> impl Iterator<Item = &Output> {
        self.current.iter().filter(|o| o.is_active)
    }

    /// Find the primary output, if any.
    #[must_use]
    pub fn primary(&self) -> Option<&Output> {
        self.current.iter().find(|o| o.is_primary && o.is_active)
    }

    /// Where should we draw the indicator on `output`?
    ///
    /// Order:
    /// 1. saved position from `saved_positions`,
    /// 2. otherwise, default = `bottom-right - inset`.
    #[must_use]
    pub fn position_for(&self, output: &Output, indicator_size: u32) -> Point {
        if let Some(p) = self.saved_positions.get(&output.name) {
            return *p;
        }
        Self::default_position(output, indicator_size)
    }

    /// Default placement (bottom-right with a 16-pixel inset).
    #[must_use]
    pub fn default_position(output: &Output, indicator_size: u32) -> Point {
        let inset = 16i32;
        let s = i32::try_from(indicator_size).unwrap_or(48);
        Point::new(
            output.geometry.right() - s - inset,
            output.geometry.bottom() - s - inset,
        )
    }

    /// Persist a new indicator position (e.g. after a drag-and-save).
    pub fn save_position(&mut self, name: OutputName, p: Point) {
        self.saved_positions.insert(name, p);
    }

    /// Read-only access to all saved positions.
    #[must_use]
    pub fn saved(&self) -> &IndexMap<OutputName, Point> {
        &self.saved_positions
    }

    /// Determine which output a given root-coordinate point belongs to.
    #[must_use]
    pub fn output_at(&self, p: Point) -> Option<&Output> {
        self.current
            .iter()
            .filter(|o| o.is_active)
            .find(|o| o.geometry.contains(p))
    }
}

/// What [`reconcile_main_indicators`] tells the daemon to do with the
/// X server in response to a config change or RandR notification.
///
/// Intentionally a plain struct of `Vec<String>`s so we can unit-test
/// the planner independently of the (async, mutex-heavy) driver code
/// in the daemon. The driver simply walks `to_remove` then `to_place`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MainIndicatorPlan {
    /// Output names whose existing main indicator must be destroyed.
    /// Includes:
    ///
    /// * outputs that disappeared from RandR (hot-unplug);
    /// * outputs that no longer match the configured `mode` (flip
    ///   `all_displays` → `primary_only`, primary changed, master
    ///   toggle disabled);
    /// * outputs we still know about but the user just unticked
    ///   `main_indicator.enable`.
    pub to_remove: Vec<String>,
    /// Output names that should host a main indicator and currently
    /// don't. The driver creates them with `place_main_indicator`.
    /// Already-existing indicators are *not* listed here — repaint
    /// of those is handled by the regular `LayoutChanged` path, so
    /// reconciliation is flicker-free in the steady state.
    pub to_place: Vec<String>,
}

/// Pure reconciliation step. Given:
///
/// * `existing`: output names that currently have a main indicator
///   window mapped on the X server (queried via the backend's
///   `main_indicator_outputs`),
/// * `want`: output names that *should* host one given the latest
///   config + RandR snapshot.
///
/// returns the diff. Order is deterministic: removals follow the
/// `existing` order; placements follow the `want` order — which
/// matches how we sort outputs in [`MonitorLayout`] (insertion
/// order from RandR, primary first by convention).
///
/// This is the load-bearing piece of multi-monitor hot-plug
/// behaviour: the daemon calls it on every `MonitorsChanged` and
/// every config reload, and it is what makes a primary-flip /
/// hot-unplug not leave zombie indicator windows on disconnected
/// outputs.
#[must_use]
pub fn reconcile_main_indicators(existing: &[String], want: &[String]) -> MainIndicatorPlan {
    let want_set: std::collections::HashSet<&str> = want.iter().map(String::as_str).collect();
    let existing_set: std::collections::HashSet<&str> =
        existing.iter().map(String::as_str).collect();

    let to_remove: Vec<String> = existing
        .iter()
        .filter(|n| !want_set.contains(n.as_str()))
        .cloned()
        .collect();
    let to_place: Vec<String> = want
        .iter()
        .filter(|n| !existing_set.contains(n.as_str()))
        .cloned()
        .collect();
    MainIndicatorPlan {
        to_remove,
        to_place,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn out(name: &str, x: i32, y: i32, w: u32, h: u32, primary: bool) -> Output {
        Output {
            name: name.into(),
            geometry: Rect::new(x, y, w, h),
            is_primary: primary,
            is_active: true,
        }
    }

    #[test]
    fn default_position_is_bottom_right_with_inset() {
        let o = out("DP-1", 0, 0, 1920, 1080, true);
        let p = MonitorLayout::default_position(&o, 48);
        assert_eq!(p, Point::new(1920 - 48 - 16, 1080 - 48 - 16));
    }

    #[test]
    fn saved_position_takes_precedence() {
        let mut l = MonitorLayout::default();
        l.save_position("DP-1".into(), Point::new(100, 100));
        let o = out("DP-1", 0, 0, 1920, 1080, true);
        assert_eq!(l.position_for(&o, 48), Point::new(100, 100));
    }

    #[test]
    fn unplug_replug_keeps_position() {
        let mut l = MonitorLayout::default();
        l.save_position("HDMI-1".into(), Point::new(50, 50));
        // Initially HDMI-1 is unplugged.
        l.update_outputs(vec![out("DP-1", 0, 0, 1920, 1080, true)]);
        assert!(l.active().all(|o| o.name.0 != "HDMI-1"));
        // Now HDMI-1 reappears.
        l.update_outputs(vec![
            out("DP-1", 0, 0, 1920, 1080, true),
            out("HDMI-1", 1920, 0, 1280, 1024, false),
        ]);
        let hdmi = l.active().find(|o| o.name.0 == "HDMI-1").unwrap();
        assert_eq!(l.position_for(hdmi, 48), Point::new(50, 50));
    }

    #[test]
    fn output_at_picks_correct_monitor() {
        let mut l = MonitorLayout::default();
        l.update_outputs(vec![
            out("DP-1", 0, 0, 1920, 1080, true),
            out("HDMI-1", 1920, 0, 1280, 1024, false),
        ]);
        assert_eq!(l.output_at(Point::new(100, 100)).unwrap().name.0, "DP-1");
        assert_eq!(l.output_at(Point::new(2000, 50)).unwrap().name.0, "HDMI-1");
        assert!(l.output_at(Point::new(5000, 5000)).is_none());
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Initial daemon start: nothing on the server yet, RandR
    /// reports two outputs, mode = all_displays — both should be
    /// placed, none removed.
    #[test]
    fn reconcile_initial_placement_creates_all_targets() {
        let plan = reconcile_main_indicators(&[], &s(&["DP-1", "HDMI-1"]));
        assert_eq!(
            plan,
            MainIndicatorPlan {
                to_remove: vec![],
                to_place: s(&["DP-1", "HDMI-1"]),
            }
        );
    }

    /// Hot-unplug: HDMI-1 disappears from RandR. Its existing
    /// indicator must be torn down; DP-1 stays untouched and is
    /// *not* re-placed (no flicker).
    #[test]
    fn reconcile_hot_unplug_removes_disappeared_output_only() {
        let existing = s(&["DP-1", "HDMI-1"]);
        let want = s(&["DP-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert_eq!(
            plan,
            MainIndicatorPlan {
                to_remove: s(&["HDMI-1"]),
                to_place: vec![],
            }
        );
    }

    /// Hot-plug: a new monitor appears. Existing indicators are
    /// preserved; the new output gets a fresh indicator.
    #[test]
    fn reconcile_hot_plug_places_new_output_only() {
        let existing = s(&["DP-1"]);
        let want = s(&["DP-1", "HDMI-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert_eq!(
            plan,
            MainIndicatorPlan {
                to_remove: vec![],
                to_place: s(&["HDMI-1"]),
            }
        );
    }

    /// User flips `mode = all_displays` -> `primary_only`. The
    /// non-primary indicator(s) get destroyed; the primary stays.
    #[test]
    fn reconcile_mode_flip_to_primary_only_destroys_secondaries() {
        let existing = s(&["DP-1", "HDMI-1"]);
        let want = s(&["DP-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert_eq!(plan.to_remove, s(&["HDMI-1"]));
        assert!(plan.to_place.is_empty());
    }

    /// Mirror of the above: the user flips back to `all_displays`.
    /// Existing primary indicator stays; the secondary is created.
    #[test]
    fn reconcile_mode_flip_to_all_displays_creates_secondaries() {
        let existing = s(&["DP-1"]);
        let want = s(&["DP-1", "HDMI-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert!(plan.to_remove.is_empty());
        assert_eq!(plan.to_place, s(&["HDMI-1"]));
    }

    /// User changes which output is primary while in `primary_only`
    /// mode: old primary's indicator must be destroyed, new
    /// primary's indicator created. Both happen in one tick.
    #[test]
    fn reconcile_primary_changed_in_primary_only_mode() {
        let existing = s(&["DP-1"]);
        let want = s(&["HDMI-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert_eq!(plan.to_remove, s(&["DP-1"]));
        assert_eq!(plan.to_place, s(&["HDMI-1"]));
    }

    /// User unticks `main_indicator.enable`. `target_main_outputs`
    /// returns []; reconciler must tear down everything we have on
    /// the wire.
    #[test]
    fn reconcile_master_disable_destroys_all_existing() {
        let existing = s(&["DP-1", "HDMI-1", "VGA-1"]);
        let plan = reconcile_main_indicators(&existing, &[]);
        assert_eq!(plan.to_remove, s(&["DP-1", "HDMI-1", "VGA-1"]));
        assert!(plan.to_place.is_empty());
    }

    /// Steady state: nothing changed, reconciler must be a complete
    /// no-op. This is what we hit on every spurious RandR notify
    /// (and there are many — KWin flips primary on each animation
    /// tick on some setups).
    #[test]
    fn reconcile_steady_state_is_a_noop() {
        let existing = s(&["DP-1", "HDMI-1"]);
        let want = s(&["DP-1", "HDMI-1"]);
        let plan = reconcile_main_indicators(&existing, &want);
        assert_eq!(plan, MainIndicatorPlan::default());
    }

    /// Order of `existing` must not affect the diff. RandR's
    /// `GetScreenResources` order is implementation-defined and
    /// can flip across reboots / hot-plugs.
    #[test]
    fn reconcile_is_set_based_not_order_based() {
        let plan_a = reconcile_main_indicators(&s(&["DP-1", "HDMI-1"]), &s(&["DP-1"]));
        let plan_b = reconcile_main_indicators(&s(&["HDMI-1", "DP-1"]), &s(&["DP-1"]));
        assert_eq!(plan_a, plan_b);
    }

    /// Orphan keys in `saved_positions` (e.g. an unplugged output) must not
    /// affect placement on a *different* active output.
    #[test]
    fn orphan_saved_keys_do_not_affect_active_outputs() {
        let mut saved = IndexMap::new();
        saved.insert(OutputName::from("HDMI-1"), Point::new(999, 999));
        let mut ml = MonitorLayout::new(saved);
        ml.update_outputs(vec![out("DP-1", 0, 0, 1920, 1080, true)]);
        let dp = ml.active().next().expect("DP-1 active");
        let p = ml.position_for(dp, 48);
        assert_ne!(p, Point::new(999, 999));
        assert_eq!(
            p,
            MonitorLayout::default_position(dp, 48),
            "DP-1 should get bottom-right default, not HDMI-1's stale coords"
        );
    }
}
