//! Glue between `xxkb-config` and `xxkb-indicators`.
//!
//! Resolves the active layout's icon name from config, renders it via
//! the shared [`IconCache`], and exposes the resulting `Arc<PixelBuffer>`
//! to the X11 paint path. Cache hits are the common case — a typical
//! session re-paints the same `(name, size, border)` triple thousands
//! of times and we want the rasterizer to run only once per triple.

use std::sync::Arc;

use xxkb_config::{BorderConfig, Config};
use xxkb_core::layout::Group;
use xxkb_indicators::{
    border::{BorderStyle, Rgb},
    resolve_icon, IconCache, IconsError, PixelBuffer, ResolvedIcon,
};

/// Render the icon for `group` at the given main-indicator settings.
///
/// Falls back to the builtin SVG for that name if the user's
/// `search_paths` don't contain it.
pub fn render_main(
    cache: &IconCache,
    cfg: &Config,
    group: Group,
) -> Result<Arc<PixelBuffer>, IconsError> {
    render_for(
        cache,
        cfg,
        group,
        cfg.main_indicator.size_px,
        &cfg.main_indicator.border,
    )
}

/// Render the icon for `group` at the per-window indicator's size /
/// border settings.
pub fn render_per_window(
    cache: &IconCache,
    cfg: &Config,
    group: Group,
) -> Result<Arc<PixelBuffer>, IconsError> {
    render_for(
        cache,
        cfg,
        group,
        cfg.per_window_indicator.size_px,
        &cfg.per_window_indicator.border,
    )
}

fn render_for(
    cache: &IconCache,
    cfg: &Config,
    group: Group,
    size_px: u32,
    border_cfg: &BorderConfig,
) -> Result<Arc<PixelBuffer>, IconsError> {
    let icon = resolve_for_group(cfg, group)?;
    let border = to_border_style(border_cfg);
    cache.get_or_render(&icon, size_px, border)
}

/// Look up the icon name for `group` in `[icons]` and resolve it
/// against the configured search paths, with `"builtin"` always tried
/// last.
fn resolve_for_group(cfg: &Config, group: Group) -> Result<ResolvedIcon, IconsError> {
    let one_based = group.as_one_based();
    let name = cfg
        .icons
        .icon_for(one_based)
        .ok_or_else(|| IconsError::NotFound {
            name: format!("group:{one_based}"),
        })?;
    let mut paths = cfg.icons.search_paths.clone();
    if !paths.iter().any(|p| p == "builtin") {
        paths.push("builtin".into());
    }
    resolve_icon(name, &paths, cfg.icons.prefer_svg)
}

fn to_border_style(cfg: &BorderConfig) -> BorderStyle {
    let color = Rgb::parse_hex(&cfg.color).unwrap_or(Rgb(0, 0, 0));
    BorderStyle::new(cfg.enabled, color, cfg.width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_mapping_and_builtin_paths() -> Config {
        // Defaults already map 1->en, 2->ru, 3->ua, 4->by; add "builtin"
        // to the search paths so resolution succeeds in CI.
        let mut cfg = Config::default();
        cfg.icons.search_paths = vec!["builtin".into()];
        cfg
    }

    #[test]
    fn renders_default_main_indicator_for_group_one() {
        let cfg = cfg_with_mapping_and_builtin_paths();
        let cache = IconCache::new();
        let group = Group::new(0, 4).unwrap(); // 0-based == 1-based 1
        let buf = render_main(&cache, &cfg, group).unwrap();
        assert_eq!(buf.width, cfg.main_indicator.size_px);
        assert_eq!(buf.height, cfg.main_indicator.size_px);
    }

    #[test]
    fn cache_is_reused_across_calls() {
        let cfg = cfg_with_mapping_and_builtin_paths();
        let cache = IconCache::new();
        let group = Group::new(1, 4).unwrap();
        let a = render_main(&cache, &cfg, group).unwrap();
        let b = render_main(&cache, &cfg, group).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn different_borders_make_different_cache_entries() {
        let mut cfg = cfg_with_mapping_and_builtin_paths();
        let cache = IconCache::new();
        let group = Group::new(0, 4).unwrap();
        let _a = render_main(&cache, &cfg, group).unwrap();

        cfg.main_indicator.border = BorderConfig {
            enabled: true,
            color: "#FF0000".into(),
            width: 2,
        };
        let _b = render_main(&cache, &cfg, group).unwrap();
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn unknown_group_returns_not_found() {
        let cfg = cfg_with_mapping_and_builtin_paths();
        let cache = IconCache::new();
        // group 7 (1-based 8) isn't mapped in defaults.
        let group = Group::new(7, 8).unwrap();
        let err = render_main(&cache, &cfg, group).err().unwrap();
        assert!(matches!(err, IconsError::NotFound { .. }));
    }
}
