//! Events the backend emits to the daemon's main loop.
//!
//! These are deliberately backend-agnostic — the daemon never sees raw
//! `xproto` events.

use xxkb_core::{
    layout::SwitchKind, monitors::Output, placement::FrameExtents, registry::WindowId,
    rules::WindowProps, Point,
};

/// Geometry of a managed (top-level client) window in **root coords**,
/// plus the WM-reported frame extents.
///
/// The per-window indicator placement uses these values together with
/// the configured offset and indicator size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeom {
    /// Top-left of the *client* area in root coordinates.
    pub origin: Point,
    /// Client width in pixels.
    pub width: u32,
    /// Client height in pixels.
    pub height: u32,
    /// `_NET_FRAME_EXTENTS` (left, right, top, bottom). Defaults to all
    /// zeros when the WM does not advertise frame extents.
    pub frame: FrameExtents,
}

/// Mouse button as reported by the backend on indicator clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// Primary button.
    Left,
    /// Middle button.
    Middle,
    /// Secondary button.
    Right,
}

/// Whether the indicator click was on the main (display) indicator
/// or on a per-window indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorTarget {
    /// One of the per-display indicators, identified by the RandR
    /// output name it lives on.
    Main(String),
    /// The per-window indicator owned by `window`.
    Window(WindowId),
}

/// Generic event coming up from the backend.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// XKB reported a state change.
    LayoutChanged {
        /// New 0-based group index.
        new_group: u8,
        /// How the change was triggered.
        kind: SwitchKind,
    },
    /// `_NET_ACTIVE_WINDOW` changed.
    ActiveWindowChanged {
        /// The new active window (None if there's no active window).
        wid: Option<WindowId>,
        /// Properties of the new active window (None if `wid` is None).
        props: Option<WindowProps>,
        /// Geometry + frame extents (None if `wid` is None or geometry
        /// could not be queried).
        geom: Option<WindowGeom>,
    },
    /// A tracked window was moved, resized, or its frame extents changed.
    WindowGeometryChanged {
        /// The window.
        wid: WindowId,
        /// New geometry.
        geom: WindowGeom,
    },
    /// A new window appeared that we should consider managing.
    WindowCreated {
        /// The window.
        wid: WindowId,
        /// Properties at creation time.
        props: WindowProps,
    },
    /// A managed window vanished.
    WindowDestroyed {
        /// The window.
        wid: WindowId,
    },
    /// RandR reported a change in the set or geometry of outputs.
    MonitorsChanged {
        /// The new output set.
        outputs: Vec<Output>,
    },
    /// User clicked one of our indicators.
    IndicatorClicked {
        /// Which indicator.
        target: IndicatorTarget,
        /// Mouse button.
        button: MouseButton,
        /// True if Ctrl was held (used for drag-to-move).
        ctrl: bool,
        /// True if Shift was held.
        shift: bool,
    },
    /// User finished dragging an indicator (released the mouse button).
    IndicatorDragged {
        /// Which indicator.
        target: IndicatorTarget,
        /// New top-left coordinate of the indicator (root coords).
        new_origin: xxkb_core::Point,
    },
}
