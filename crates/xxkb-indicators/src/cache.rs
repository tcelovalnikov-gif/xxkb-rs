//! Memoization layer over `render` + `apply_border`.
//!
//! Layout switches happen often (every keystroke that moves the
//! modifier latches), so we never want to re-rasterize an SVG for the
//! same `(name, size, border)` triple. The cache stores
//! `Arc<PixelBuffer>` so consumers can hand the buffer to other threads
//! (e.g. the X11 worker thread) without copying.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::{
    border::{apply_border, BorderStyle},
    render::render,
    IconsError, PixelBuffer, ResolvedIcon,
};

/// Hashable key uniquely identifying a rendered+bordered icon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Stable id of the source (file path or `"builtin:..."`).
    pub source_id: String,
    /// Whether the source is SVG (so RGBA path differs).
    pub is_svg: bool,
    /// Render side length in pixels.
    pub size_px: u32,
    /// Border parameters baked into the cache key.
    pub border: BorderStyle,
}

/// Thread-safe LRU-ish cache keyed on `(source, size, border)`.
///
/// We don't currently evict entries: the working set is tiny (≤ a few
/// dozen flags × a couple of sizes per session), and entries are
/// `Arc<PixelBuffer>` so the memory overhead is bounded.
#[derive(Debug, Default)]
pub struct IconCache {
    inner: Mutex<indexmap::IndexMap<CacheKey, Arc<PixelBuffer>>>,
}

impl IconCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Render `icon` at `size_px` with `border` applied, returning the
    /// cached buffer.
    pub fn get_or_render(
        &self,
        icon: &ResolvedIcon,
        size_px: u32,
        border: BorderStyle,
    ) -> Result<Arc<PixelBuffer>, IconsError> {
        let key = CacheKey {
            source_id: icon.source.cache_id(),
            is_svg: icon.is_svg,
            size_px,
            border,
        };
        if let Some(hit) = self.inner.lock().get(&key).cloned() {
            return Ok(hit);
        }
        // Render outside the lock — another thread might race; the
        // `IndexMap::entry` below resolves the race deterministically
        // (last writer wins, but both copies are byte-identical).
        let mut buf = render(icon, size_px)?;
        apply_border(&mut buf, &key.border);
        let arc = Arc::new(buf);
        self.inner.lock().insert(key, Arc::clone(&arc));
        Ok(arc)
    }

    /// Forget every cached buffer. Called on hot-reload when the
    /// border/size config changes.
    pub fn clear(&self) {
        self.inner.lock().clear();
    }

    /// Number of cached entries (for tests/metrics).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// True iff the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bundle, IconSource};

    fn fake_en() -> ResolvedIcon {
        let (id, bytes) = bundle::lookup("en").unwrap();
        ResolvedIcon {
            source: IconSource::Embedded { id, bytes },
            is_svg: true,
        }
    }

    #[test]
    fn second_lookup_returns_same_arc() {
        let cache = IconCache::new();
        let a = cache
            .get_or_render(&fake_en(), 32, BorderStyle::default())
            .unwrap();
        let b = cache
            .get_or_render(&fake_en(), 32, BorderStyle::default())
            .unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn different_sizes_are_separate_entries() {
        let cache = IconCache::new();
        let _ = cache
            .get_or_render(&fake_en(), 16, BorderStyle::default())
            .unwrap();
        let _ = cache
            .get_or_render(&fake_en(), 32, BorderStyle::default())
            .unwrap();
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_drops_entries() {
        let cache = IconCache::new();
        let _ = cache
            .get_or_render(&fake_en(), 24, BorderStyle::default())
            .unwrap();
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }
}
