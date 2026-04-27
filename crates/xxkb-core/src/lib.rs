//! Pure-logic core of xxkb-rs.
//!
//! This crate intentionally has **no I/O**:
//! no X11, no D-Bus, no filesystem, no sound. Everything here is data
//! transformations on plain Rust types. That makes the most behavior-critical
//! parts of the project trivially unit-testable, which is why we want the bulk
//! of our test coverage to live here.
//!
//! Top-level modules:
//! * [`layout`] — `LayoutState`: the current keyboard group + history, plus
//!   the rules around `two_state` mode.
//! * [`registry`] — `WindowRegistry`: per-window remembered layout, used to
//!   restore the layout on focus change.
//! * [`rules`] — `AppRules`: glob-based matching of `WM_CLASS` / `WM_NAME` to
//!   `ignore` / `start_alt` / `alt_group_N` actions.
//! * [`monitors`] — `MonitorLayout`: indicator coordinates bound to the RandR
//!   output **name** (so monitor reorder/replug doesn't move flags around).
//! * [`placement`] — `IndicatorPlacement`: where to draw a per-window
//!   indicator relative to a window's frame extents.

#![deny(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]
// X11 protocol property names (WM_CLASS, WM_NAME); keep the `Wm` prefix.
#![allow(clippy::enum_variant_names)]

pub mod layout;
pub mod monitors;
pub mod placement;
pub mod registry;
pub mod rules;

pub use layout::{Group, LayoutState, SwitchKind, TwoStateConfig};
pub use monitors::{MonitorLayout, Output, OutputName, Point, Rect};
pub use placement::{FrameExtents, IndicatorPlacement, Offset};
pub use registry::{RememberedLayout, WindowId, WindowRegistry};
pub use rules::{AppMatch, AppRule, AppRules, RuleAction, WindowProps};

/// Crate-level error type for the (rare) operations that can fail.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A group index outside `1..=max_groups` was supplied.
    #[error("group {given} out of range 1..={max}")]
    GroupOutOfRange {
        /// The offending value.
        given: u8,
        /// The maximum group configured for this state.
        max: u8,
    },

    /// A glob pattern in [`AppRules`] failed to compile.
    #[error("bad glob pattern '{pattern}': {source}")]
    BadGlob {
        /// The pattern as supplied by the user.
        pattern: String,
        #[source]
        /// Underlying error from `globset`.
        source: globset::Error,
    },
}
