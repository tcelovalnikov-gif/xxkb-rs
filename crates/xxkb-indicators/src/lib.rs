//! Flag rendering and icon resolution for xxkb-rs.
//!
//! Three responsibilities:
//!
//! 1. **Resolution** — given an icon `name` (e.g. `"en"`, `"ru"`) and a
//!    list of search paths, find the best icon file (preferring SVG when
//!    configured).
//! 2. **Rendering** — turn an SVG/raster file into a square `PixelBuffer`
//!    in cairo's native BGRA-premultiplied format (suitable for upload
//!    to an X11 pixmap or use as a GTK4 `Texture`). The default path is
//!    pure-Rust (`resvg` + `tiny-skia` for SVG, `image` for raster).
//! 3. **Border drawing & caching** — paint a configurable border around
//!    the icon and memoize the result by `(name, size, border)` so we
//!    don't re-render on every layout switch.
//!
//! The cairo + librsvg variant is opt-in behind the `gtk-render` feature
//! and is intended for the configurator GUI, which already pulls cairo.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::path::PathBuf;

use thiserror::Error;

pub mod border;
pub mod bundle;
pub mod cache;
pub mod render;

pub use border::{apply_border, BorderStyle, Rgb};
pub use cache::{CacheKey, IconCache};
pub use render::render;

/// Errors from icon resolution and rendering.
#[derive(Debug, Error)]
pub enum IconsError {
    /// No icon for the given name was found in any of the search paths.
    #[error("no icon found for '{name}'")]
    NotFound {
        /// Logical icon name (e.g. `"en"`, `"ru"`).
        name: String,
    },

    /// I/O error reading an icon file.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Failed to decode an icon file (corrupt SVG, unsupported raster, ...).
    #[error("decode error: {0}")]
    Decode(String),

    /// The requested size (px) is invalid.
    #[error("invalid size: {0}")]
    InvalidSize(u32),
}

/// Where the icon's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    /// File on disk.
    File(PathBuf),
    /// Bytes baked into the binary at compile time.
    Embedded {
        /// Stable identifier used as a cache key (e.g. `"builtin:en.svg"`).
        id: &'static str,
        /// Raw file bytes (SVG text or raster).
        bytes: &'static [u8],
    },
}

impl IconSource {
    /// Stable string identifying this source. Used as the cache key.
    #[must_use]
    pub fn cache_id(&self) -> String {
        match self {
            Self::File(p) => format!("file:{}", p.display()),
            Self::Embedded { id, .. } => format!("builtin:{id}"),
        }
    }
}

/// Lookup result for an icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcon {
    /// Where to read the bytes from.
    pub source: IconSource,
    /// Whether it's an SVG (vector). Determines which renderer is used.
    pub is_svg: bool,
}

/// Resolve `name` (e.g. `"en"`, `"ru"`) to an icon file or builtin.
///
/// `search_paths` is searched in order. Each entry is one of:
/// * a real filesystem path (`~` is expanded);
/// * the literal token `"system"` → `/usr/share/xxkb/icons`;
/// * the literal token `"builtin"` → falls back to the compiled-in
///   icons in [`bundle`].
pub fn resolve_icon(
    name: &str,
    search_paths: &[String],
    prefer_svg: bool,
) -> Result<ResolvedIcon, IconsError> {
    let extensions: &[&str] = if prefer_svg {
        &["svg", "png", "jpg", "jpeg", "bmp"]
    } else {
        &["png", "jpg", "jpeg", "bmp", "svg"]
    };

    for spec in search_paths {
        match spec.as_str() {
            "builtin" => {
                if let Some((id, bytes)) = bundle::lookup(name) {
                    return Ok(ResolvedIcon {
                        source: IconSource::Embedded { id, bytes },
                        is_svg: id.ends_with(".svg"),
                    });
                }
            }
            "system" => {
                if let Some(found) =
                    try_dir(&PathBuf::from("/usr/share/xxkb/icons"), name, extensions)
                {
                    return Ok(found);
                }
            }
            other => {
                let dir = PathBuf::from(shellexpand_tilde(other));
                if let Some(found) = try_dir(&dir, name, extensions) {
                    return Ok(found);
                }
            }
        }
    }
    Err(IconsError::NotFound {
        name: name.to_owned(),
    })
}

fn try_dir(dir: &std::path::Path, name: &str, extensions: &[&str]) -> Option<ResolvedIcon> {
    for ext in extensions {
        let candidate = dir.join(format!("{name}.{ext}"));
        if candidate.is_file() {
            return Some(ResolvedIcon {
                is_svg: *ext == "svg",
                source: IconSource::File(candidate),
            });
        }
    }
    None
}

fn shellexpand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut p = PathBuf::from(home);
            p.push(rest);
            return p.to_string_lossy().into_owned();
        }
    }
    s.to_owned()
}

/// A square pixel buffer in cairo's native BGRA-premultiplied layout.
///
/// `data.len() == stride * height`. Width and stride may differ when
/// the renderer pads rows to a 4-byte boundary; for square icons at
/// power-of-two sizes they coincide (`stride == width * 4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelBuffer {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub stride: usize,
    /// BGRA premultiplied bytes.
    pub data: Vec<u8>,
}

impl PixelBuffer {
    /// New solid-color buffer (BGRA premultiplied).
    #[must_use]
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let [r, g, b, a] = rgba;
        let pre_r = ((u32::from(r) * u32::from(a)) / 255) as u8;
        let pre_g = ((u32::from(g) * u32::from(a)) / 255) as u8;
        let pre_b = ((u32::from(b) * u32::from(a)) / 255) as u8;
        let stride = (width as usize) * 4;
        let mut data = Vec::with_capacity(stride * height as usize);
        for _ in 0..(width * height) {
            data.extend_from_slice(&[pre_b, pre_g, pre_r, a]);
        }
        Self {
            width,
            height,
            stride,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn resolve_prefers_svg_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("en.svg");
        let png = dir.path().join("en.png");
        std::fs::File::create(&svg)
            .unwrap()
            .write_all(b"<svg/>")
            .unwrap();
        std::fs::File::create(&png)
            .unwrap()
            .write_all(b"PNG")
            .unwrap();
        let r = resolve_icon("en", &[dir.path().to_string_lossy().to_string()], true).unwrap();
        assert_eq!(r.source, IconSource::File(svg));
        assert!(r.is_svg);
    }

    #[test]
    fn resolve_prefers_png_when_svg_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("en.svg");
        let png = dir.path().join("en.png");
        std::fs::File::create(&svg)
            .unwrap()
            .write_all(b"<svg/>")
            .unwrap();
        std::fs::File::create(&png)
            .unwrap()
            .write_all(b"PNG")
            .unwrap();
        let r = resolve_icon("en", &[dir.path().to_string_lossy().to_string()], false).unwrap();
        assert_eq!(r.source, IconSource::File(png));
        assert!(!r.is_svg);
    }

    #[test]
    fn resolve_falls_back_to_builtin() {
        // Empty user dir with builtin token should pick up the bundled SVG.
        let dir = tempfile::tempdir().unwrap();
        let r = resolve_icon(
            "en",
            &[dir.path().to_string_lossy().to_string(), "builtin".into()],
            true,
        )
        .unwrap();
        assert!(matches!(r.source, IconSource::Embedded { .. }));
        assert!(r.is_svg);
    }

    #[test]
    fn resolve_returns_not_found_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_icon(
            "zz_unknown",
            &[dir.path().to_string_lossy().to_string()],
            true,
        )
        .err()
        .unwrap();
        assert!(matches!(err, IconsError::NotFound { name } if name == "zz_unknown"));
    }

    #[test]
    fn solid_buffer_has_expected_dimensions() {
        let buf = PixelBuffer::solid(16, 16, [255, 0, 0, 255]);
        assert_eq!(buf.width, 16);
        assert_eq!(buf.height, 16);
        assert_eq!(buf.stride, 64);
        assert_eq!(buf.data.len(), 64 * 16);
        // BGRA: B=0, G=0, R=255, A=255.
        assert_eq!(&buf.data[0..4], &[0, 0, 255, 255]);
    }
}
