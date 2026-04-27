//! Pure-Rust rendering: SVG via `resvg`/`tiny-skia`, raster via `image`.
//!
//! Output is always a square `PixelBuffer` of the requested size in
//! cairo's native BGRA-premultiplied byte order.

use std::path::Path;

use crate::{IconSource, IconsError, PixelBuffer, ResolvedIcon};

/// Render `icon` at `size_px × size_px`.
pub fn render(icon: &ResolvedIcon, size_px: u32) -> Result<PixelBuffer, IconsError> {
    if size_px == 0 {
        return Err(IconsError::InvalidSize(0));
    }
    let bytes = read_bytes(&icon.source)?;
    if icon.is_svg {
        render_svg(&bytes, size_px)
    } else {
        render_raster(&bytes, size_px)
    }
}

fn read_bytes(source: &IconSource) -> Result<Vec<u8>, IconsError> {
    match source {
        IconSource::File(p) => Ok(std::fs::read(p)?),
        IconSource::Embedded { bytes, .. } => Ok((*bytes).to_vec()),
    }
}

fn render_svg(svg_bytes: &[u8], size_px: u32) -> Result<PixelBuffer, IconsError> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opt)
        .map_err(|e| IconsError::Decode(format!("usvg parse: {e}")))?;
    let mut pixmap = tiny_skia::Pixmap::new(size_px, size_px)
        .ok_or_else(|| IconsError::Decode(format!("tiny-skia pixmap {size_px}^2")))?;

    // Scale the SVG's intrinsic viewport so it fills our square pixmap.
    let svg_size = tree.size();
    let sx = size_px as f32 / svg_size.width();
    let sy = size_px as f32 / svg_size.height();
    let transform = tiny_skia::Transform::from_scale(sx, sy);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(rgba_to_bgra_premul(pixmap.data(), size_px, size_px))
}

fn render_raster(bytes: &[u8], size_px: u32) -> Result<PixelBuffer, IconsError> {
    let img =
        image::load_from_memory(bytes).map_err(|e| IconsError::Decode(format!("image: {e}")))?;
    let resized = img.resize_exact(size_px, size_px, image::imageops::FilterType::Lanczos3);
    let rgba = resized.into_rgba8();
    Ok(rgba_to_bgra_premul(rgba.as_raw(), size_px, size_px))
}

/// Convert RGBA8 (already premultiplied for `tiny-skia` output, but
/// straight-alpha for `image::DynamicImage`) into cairo's native
/// BGRA-premultiplied layout.
pub(crate) fn rgba_to_bgra_premul(rgba: &[u8], width: u32, height: u32) -> PixelBuffer {
    let stride = (width as usize) * 4;
    let mut data = Vec::with_capacity(stride * height as usize);
    for chunk in rgba.chunks_exact(4) {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];
        // Idempotent on already-premultiplied input as long as the
        // src channels are <= alpha.
        let pre = |c: u8| -> u8 {
            if a == 255 {
                c
            } else {
                ((u32::from(c) * u32::from(a)) / 255) as u8
            }
        };
        data.extend_from_slice(&[pre(b), pre(g), pre(r), a]);
    }
    PixelBuffer {
        width,
        height,
        stride,
        data,
    }
}

/// Convenience: render directly from disk (used by the configurator and tests).
pub fn render_file(path: &Path, size_px: u32) -> Result<PixelBuffer, IconsError> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let icon = ResolvedIcon {
        source: IconSource::File(path.to_path_buf()),
        is_svg,
    };
    render(&icon, size_px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle;

    #[test]
    fn render_builtin_en_svg_at_48px() {
        let (id, bytes) = bundle::lookup("en").unwrap();
        let icon = ResolvedIcon {
            source: IconSource::Embedded { id, bytes },
            is_svg: true,
        };
        let buf = render(&icon, 48).unwrap();
        assert_eq!(buf.width, 48);
        assert_eq!(buf.height, 48);
        assert_eq!(buf.stride, 48 * 4);
        assert_eq!(buf.data.len(), 48 * 48 * 4);
        // The Union Jack–style flag uses a white background, so the
        // four corners must be (very close to) opaque.
        let alpha_at = |x: u32, y: u32| -> u8 {
            let i = (y as usize) * buf.stride + (x as usize) * 4 + 3;
            buf.data[i]
        };
        assert!(alpha_at(0, 0) > 200, "top-left should be opaque");
        assert!(alpha_at(47, 47) > 200, "bottom-right should be opaque");
    }

    #[test]
    fn render_zero_size_is_an_error() {
        let (id, bytes) = bundle::lookup("ru").unwrap();
        let icon = ResolvedIcon {
            source: IconSource::Embedded { id, bytes },
            is_svg: true,
        };
        assert!(matches!(render(&icon, 0), Err(IconsError::InvalidSize(0))));
    }

    #[test]
    fn ru_flag_has_three_horizontal_bands() {
        // Top band white, middle blue, bottom red — sample one pixel each.
        let (id, bytes) = bundle::lookup("ru").unwrap();
        let icon = ResolvedIcon {
            source: IconSource::Embedded { id, bytes },
            is_svg: true,
        };
        let buf = render(&icon, 60).unwrap();
        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let i = (y as usize) * buf.stride + (x as usize) * 4;
            [
                buf.data[i],
                buf.data[i + 1],
                buf.data[i + 2],
                buf.data[i + 3],
            ]
        };
        let [b1, g1, r1, _] = pixel(30, 8);
        let [b2, g2, r2, _] = pixel(30, 30);
        let [b3, g3, r3, _] = pixel(30, 52);
        // White ≈ (255,255,255), blue ≈ (0,0,~150–180), red ≈ (~200,0,0).
        assert!(
            r1 > 200 && g1 > 200 && b1 > 200,
            "top band should be ~white, got bgr=({b1},{g1},{r1})"
        );
        assert!(
            b2 > 80 && r2 < 80 && g2 < 100,
            "middle band should be ~blue, got bgr=({b2},{g2},{r2})"
        );
        assert!(
            r3 > 150 && g3 < 80 && b3 < 80,
            "bottom band should be ~red, got bgr=({b3},{g3},{r3})"
        );
    }
}
