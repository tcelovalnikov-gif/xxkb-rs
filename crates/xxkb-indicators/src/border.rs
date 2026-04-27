//! Border drawing on top of an already-rendered `PixelBuffer`.
//!
//! Cheap deterministic per-pixel routine — fast enough that we don't
//! bother with SIMD. The buffer is in cairo's native BGRA-premultiplied
//! layout, which matches our [`PixelBuffer`].

use crate::PixelBuffer;

/// Hex-coded RGB color (no alpha — alpha is handled separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse a `#RRGGBB` string. `#` is optional. Returns `None` for
    /// any malformed input.
    #[must_use]
    pub fn parse_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self(r, g, b))
    }
}

/// Border style. `width_px == 0` (or `enabled == false`) is a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BorderStyle {
    /// Whether to actually draw.
    pub enabled: bool,
    /// Stroke color (RGB). Always drawn fully opaque.
    pub color: Rgb,
    /// Border width in pixels.
    pub width_px: u32,
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Rgb(0, 0, 0),
            width_px: 0,
        }
    }
}

impl BorderStyle {
    /// Build from raw values; clamps width so it can't exceed half of
    /// the smaller image dimension.
    #[must_use]
    pub fn new(enabled: bool, color: Rgb, width_px: u32) -> Self {
        Self {
            enabled,
            color,
            width_px,
        }
    }
}

/// Paint `style` onto `buf` in place. No-op when disabled or zero-width.
pub fn apply_border(buf: &mut PixelBuffer, style: &BorderStyle) {
    if !style.enabled || style.width_px == 0 {
        return;
    }
    let w = buf.width as i64;
    let h = buf.height as i64;
    let bw = i64::from(style.width_px).min(w / 2).min(h / 2);
    if bw == 0 {
        return;
    }
    let Rgb(r, g, b) = style.color;
    // Premultiplied at full alpha == passthrough.
    let bgra = [b, g, r, 255_u8];

    let stride = buf.stride;
    for y in 0..h {
        for x in 0..w {
            let on_edge = x < bw || x >= w - bw || y < bw || y >= h - bw;
            if !on_edge {
                continue;
            }
            let idx = (y as usize) * stride + (x as usize) * 4;
            buf.data[idx..idx + 4].copy_from_slice(&bgra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(buf: &PixelBuffer, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize) * buf.stride + (x as usize) * 4;
        [
            buf.data[i],
            buf.data[i + 1],
            buf.data[i + 2],
            buf.data[i + 3],
        ]
    }

    #[test]
    fn rgb_parse_accepts_hash_and_bare() {
        assert_eq!(Rgb::parse_hex("#102030"), Some(Rgb(0x10, 0x20, 0x30)));
        assert_eq!(Rgb::parse_hex("AABBCC"), Some(Rgb(0xAA, 0xBB, 0xCC)));
        assert_eq!(Rgb::parse_hex("xyz"), None);
        assert_eq!(Rgb::parse_hex("#1234"), None);
    }

    #[test]
    fn disabled_border_is_a_noop() {
        let mut buf = PixelBuffer::solid(8, 8, [0, 0, 0, 0]);
        let before = buf.data.clone();
        apply_border(&mut buf, &BorderStyle::default());
        assert_eq!(buf.data, before);
    }

    #[test]
    fn one_pixel_border_paints_the_outline() {
        let mut buf = PixelBuffer::solid(6, 6, [0, 0, 0, 0]);
        apply_border(
            &mut buf,
            &BorderStyle {
                enabled: true,
                color: Rgb(255, 0, 0),
                width_px: 1,
            },
        );
        // Corner is red.
        assert_eq!(pixel(&buf, 0, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&buf, 5, 5), [0, 0, 255, 255]);
        // Interior untouched (transparent).
        assert_eq!(pixel(&buf, 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn two_pixel_border_paints_two_rings() {
        let mut buf = PixelBuffer::solid(8, 8, [0, 0, 0, 0]);
        apply_border(
            &mut buf,
            &BorderStyle {
                enabled: true,
                color: Rgb(0, 255, 0),
                width_px: 2,
            },
        );
        // Inner ring is also painted.
        assert_eq!(pixel(&buf, 1, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&buf, 6, 6), [0, 255, 0, 255]);
        // Center is still untouched.
        assert_eq!(pixel(&buf, 4, 4), [0, 0, 0, 0]);
    }

    #[test]
    fn border_wider_than_image_is_clamped() {
        let mut buf = PixelBuffer::solid(4, 4, [0, 0, 0, 0]);
        apply_border(
            &mut buf,
            &BorderStyle {
                enabled: true,
                color: Rgb(255, 255, 255),
                width_px: 999,
            },
        );
        // Clamped to half the dimension == 2 px ring on each side,
        // which fully covers the 4×4 image.
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(pixel(&buf, x, y), [255, 255, 255, 255]);
            }
        }
    }
}
