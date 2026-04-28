//! Per-window indicator placement.
//!
//! Given a window's frame extents (decorations) and a configured offset,
//! compute where to place the per-window indicator. The convention
//! mirrors the legacy xxkb behaviour:
//!
//! * positive `offset.x` means "from the **left** edge of the title bar";
//! * negative `offset.x` means "from the **right** edge";
//! * `offset.y` is always relative to the **top** of the title bar.

use serde::{Deserialize, Serialize};

use crate::monitors::Point;

/// Frame extents reported by the window manager
/// (`_NET_FRAME_EXTENTS = left, right, top, bottom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameExtents {
    /// Left border width.
    pub left: u32,
    /// Right border width.
    pub right: u32,
    /// Top decoration (typically the title bar) height.
    pub top: u32,
    /// Bottom border width.
    pub bottom: u32,
}

impl FrameExtents {
    /// Synthesise frame extents from the geometry of the WM-supplied
    /// *frame* window and the *client* window's geometry in root
    /// coordinates.
    ///
    /// This is the EWMH-fallback path: some WMs (older Mutter, some
    /// xmonad / dwm builds, IceWM in nodecor mode) do not advertise
    /// `_NET_FRAME_EXTENTS`. The daemon walks the parent chain of
    /// the client window with `QueryTree`, finds the immediate child
    /// of the root (the WM's reparenting container), reads its
    /// geometry, and feeds both rectangles here. The resulting
    /// extents are good enough to keep the per-window indicator
    /// inside the title bar on every WM that reparents at all.
    ///
    /// Inputs are signed `i32` (root coords / sizes can be cast
    /// back). Negative deltas — which would only happen under buggy
    /// WMs — are clamped to zero so we never fail open.
    #[must_use]
    pub fn from_frame_and_client(
        frame_origin: Point,
        frame_width: u32,
        frame_height: u32,
        client_origin: Point,
        client_width: u32,
        client_height: u32,
    ) -> Self {
        let fw = i32::try_from(frame_width).unwrap_or(0);
        let fh = i32::try_from(frame_height).unwrap_or(0);
        let cw = i32::try_from(client_width).unwrap_or(0);
        let ch = i32::try_from(client_height).unwrap_or(0);

        let left = (client_origin.x - frame_origin.x).max(0) as u32;
        let top = (client_origin.y - frame_origin.y).max(0) as u32;
        let right = ((frame_origin.x + fw) - (client_origin.x + cw)).max(0) as u32;
        let bottom = ((frame_origin.y + fh) - (client_origin.y + ch)).max(0) as u32;

        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    /// True if all four borders are zero. Useful for "is the WM
    /// undecorated for this window?" decisions in the daemon.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.left == 0 && self.right == 0 && self.top == 0 && self.bottom == 0
    }
}

/// Configured offset for the per-window indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offset {
    /// X offset. Positive = from left edge, negative = from right edge.
    pub x: i32,
    /// Y offset, from the top.
    pub y: i32,
}

impl Default for Offset {
    fn default() -> Self {
        // Mirrors XXkb.button.geometry: 15x15-60+7 (60 from right, 7 from top).
        Self { x: -60, y: 7 }
    }
}

/// Per-window indicator placement helper.
pub struct IndicatorPlacement;

impl IndicatorPlacement {
    /// Compute the absolute root-coordinate point at which to place the
    /// indicator for a window with top-left at `window_origin`, size
    /// `(window_width, _)`, frame extents `extents`, and indicator size
    /// `indicator_size`.
    #[must_use]
    pub fn compute(
        window_origin: Point,
        window_width: u32,
        extents: FrameExtents,
        offset: Offset,
        indicator_size: u32,
    ) -> Point {
        let title_y = window_origin.y - i32::try_from(extents.top).unwrap_or(0);
        let title_left = window_origin.x - i32::try_from(extents.left).unwrap_or(0);
        let title_right = window_origin.x
            + i32::try_from(window_width).unwrap_or(0)
            + i32::try_from(extents.right).unwrap_or(0);

        let x = if offset.x >= 0 {
            title_left + offset.x
        } else {
            // Negative offset: x measured from right edge inward.
            // We also subtract indicator_size so the indicator fits inside
            // the title bar.
            title_right + offset.x - i32::try_from(indicator_size).unwrap_or(0)
        };
        let y = title_y + offset.y;

        Point::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn negative_offset_anchors_to_right_edge() {
        // Window at (100, 200), 800 wide, 30px title bar. Offset -60+7, 15px indicator.
        let p = IndicatorPlacement::compute(
            Point::new(100, 200),
            800,
            FrameExtents {
                left: 1,
                right: 1,
                top: 30,
                bottom: 1,
            },
            Offset { x: -60, y: 7 },
            15,
        );
        // Right edge = 100 + 800 + 1 = 901
        // x = 901 + (-60) - 15 = 826
        // title_y = 200 - 30 = 170; y = 170 + 7 = 177
        assert_eq!(p, Point::new(826, 177));
    }

    #[test]
    fn positive_offset_anchors_to_left_edge() {
        let p = IndicatorPlacement::compute(
            Point::new(100, 200),
            800,
            FrameExtents {
                left: 5,
                right: 5,
                top: 30,
                bottom: 5,
            },
            Offset { x: 10, y: 5 },
            15,
        );
        // title_left = 100 - 5 = 95; x = 95 + 10 = 105
        // title_y = 200 - 30 = 170; y = 170 + 5 = 175
        assert_eq!(p, Point::new(105, 175));
    }

    #[test]
    fn zero_extents_does_not_panic() {
        let p = IndicatorPlacement::compute(
            Point::new(0, 0),
            500,
            FrameExtents::default(),
            Offset { x: -20, y: 0 },
            10,
        );
        assert_eq!(p, Point::new(500 - 20 - 10, 0));
    }

    /// Typical Mutter-style decoration: title bar 28 px, side
    /// borders 1 px, bottom border 1 px. Frame is at (100, 200);
    /// client lives offset by (1, 28) inside the frame.
    #[test]
    fn synth_extents_recovers_typical_mutter_decoration() {
        let extents = FrameExtents::from_frame_and_client(
            Point::new(100, 200),
            802,
            629,
            Point::new(101, 228),
            800,
            600,
        );
        assert_eq!(
            extents,
            FrameExtents {
                left: 1,
                right: 1,
                top: 28,
                bottom: 1,
            }
        );
    }

    /// Tiling WM that doesn't reparent (xmonad in noBorders mode):
    /// frame == client, so all extents are zero. Must not panic
    /// and must not produce phantom extents.
    #[test]
    fn synth_extents_for_undecorated_window_is_zero() {
        let extents = FrameExtents::from_frame_and_client(
            Point::new(0, 0),
            1920,
            1080,
            Point::new(0, 0),
            1920,
            1080,
        );
        assert_eq!(extents, FrameExtents::default());
        assert!(extents.is_zero());
    }

    /// Buggy WM that reports the frame window *smaller* than the
    /// client (shouldn't happen, but seen in the wild on broken
    /// XWayland surfaces). We clamp to zero — never produce
    /// negative numbers in `u32`.
    #[test]
    fn synth_extents_clamps_negative_deltas_to_zero() {
        let extents = FrameExtents::from_frame_and_client(
            Point::new(50, 50),
            100,
            100,
            Point::new(40, 40),
            120,
            120,
        );
        // Frame is (50,50)..(150,150); client is (40,40)..(160,160)
        // — client overhangs frame on every side. All extents
        // clamp to zero.
        assert_eq!(extents, FrameExtents::default());
    }

    /// Asymmetric decoration: thick left border (e.g. tiled vertical
    /// title bar like in i3 nodecor with a bar). Verify each side
    /// is computed independently.
    #[test]
    fn synth_extents_handles_asymmetric_decoration() {
        let extents = FrameExtents::from_frame_and_client(
            Point::new(0, 0),
            100,
            100,
            Point::new(20, 5),
            70,
            85,
        );
        // left = 20-0 = 20
        // top = 5-0 = 5
        // right = (0+100) - (20+70) = 10
        // bottom = (0+100) - (5+85) = 10
        assert_eq!(
            extents,
            FrameExtents {
                left: 20,
                right: 10,
                top: 5,
                bottom: 10,
            }
        );
    }
}
