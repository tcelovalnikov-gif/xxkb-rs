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
}
