//! Daemon main loop.
//!
//! Wires together:
//! * `xxkb-config` — read config + watch for changes
//! * `xxkb-x11` — talk to the X server
//! * `xxkb-core` — pure logic (registry, rules, layout)
//! * `xxkb-sound` — play click on switch
//! * `xxkb-dbus` — expose `org.xxkb.Daemon1` for the configurator

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use xxkb_config::{Config, MainIndicatorMode};
use xxkb_core::{
    layout::{Group, LayoutState, SwitchKind, TwoStateConfig},
    monitors::{MonitorLayout, OutputName, Point},
    registry::{WindowId, WindowRegistry},
    rules::{AppRules, Verdict},
    IndicatorPlacement,
};
use xxkb_dbus::{DaemonInterface, DbusError, Emitter, WireOutput, WireWindow};
use xxkb_indicators::IconCache;
use xxkb_sound::{MockPlayer, SoundPlayer, Trigger};
use xxkb_x11::{Backend, BackendEvent, IndicatorTarget, MouseButton, WindowGeom, X11Backend};

/// Per-window geometry cache keyed by tracked client window.
///
/// We update entries on `ActiveWindowChanged`/`WindowGeometryChanged`
/// and consult them when (re)placing the per-window indicator.
type GeomCache = Arc<Mutex<HashMap<WindowId, WindowGeom>>>;

/// Per-window properties cache (`WM_CLASS`, `WM_NAME`) keyed by
/// tracked client window.
///
/// Used by the D-Bus `GetActiveWindows` reply so the configurator's
/// rules editor can populate "Capture from active window" entries
/// with real strings.
type PropsCache = Arc<Mutex<HashMap<WindowId, xxkb_core::rules::WindowProps>>>;

use crate::{flag, hot_reload};

/// Run forever (Ctrl+C to exit).
pub async fn run() -> Result<()> {
    let cfg = Config::load_default().context("loading config")?;
    let cfg = Arc::new(Mutex::new(cfg));

    let mut backend = X11Backend::new();
    backend.connect().await.context("connect to X server")?;

    let initial_group = backend
        .current_group()
        .await
        .unwrap_or_else(|_| Group::new(0, 4).expect("0 is always a valid 0-based group"));
    let outputs = backend.outputs().await.unwrap_or_default();

    let two_state = build_two_state(&cfg.lock());
    let layout = Arc::new(Mutex::new(LayoutState::new(4, initial_group, two_state)));
    let registry = Arc::new(Mutex::new(WindowRegistry::new()));
    let monitor_layout = Arc::new(Mutex::new({
        let positions = cfg.lock().main_indicator.positions.clone();
        let mut ml = MonitorLayout::new(positions);
        ml.update_outputs(outputs.clone());
        ml
    }));
    let rules = Arc::new(Mutex::new(build_rules(&cfg.lock())));
    let player: Arc<dyn SoundPlayer> = Arc::new(MockPlayer::new()); // swapped out at runtime
    let icons = Arc::new(IconCache::new());
    let geom_cache: GeomCache = Arc::new(Mutex::new(HashMap::new()));
    let props_cache: PropsCache = Arc::new(Mutex::new(HashMap::new()));

    let std_rx = backend
        .take_event_rx()
        .ok_or_else(|| anyhow::anyhow!("event_rx already taken"))?;
    let (async_tx, async_rx) = tokio::sync::mpsc::unbounded_channel::<BackendEvent>();
    tokio::task::spawn_blocking(move || {
        while let Ok(ev) = std_rx.recv() {
            if async_tx.send(ev).is_err() {
                break;
            }
        }
    });
    let backend = Arc::new(tokio::sync::Mutex::new(backend));

    place_initial_main_indicators(&backend, &monitor_layout, &cfg, &layout, &icons).await?;

    let dbus_iface = Arc::new(DaemonHandle {
        cfg: cfg.clone(),
        backend: backend.clone(),
        layout: layout.clone(),
        monitor_layout: monitor_layout.clone(),
        rules: rules.clone(),
        icons: icons.clone(),
        registry: registry.clone(),
        props_cache: props_cache.clone(),
    });
    // `_dbus_conn` is held to keep the bus connection alive for the
    // duration of `event_loop` — dropping it would unregister the
    // interface and revoke the well-known name.
    let (_dbus_conn, emitter) = match xxkb_dbus::serve(dbus_iface).await {
        Ok((c, em)) => (Some(c), Some(em)),
        Err(DbusError::NameTaken(e)) => {
            tracing::warn!(error = %e, "another xxkbd is already running on D-Bus; continuing without bus");
            (None, None)
        }
        Err(e) => return Err(anyhow::anyhow!("d-bus: {e}")),
    };

    // The `notify` watcher runs its callback on its own worker thread,
    // which is *not* a tokio runtime thread. Capture a runtime handle
    // so we can correctly spawn the async `reload` from there.
    let rt_handle = tokio::runtime::Handle::current();
    let cfg_path = xxkb_config::config_path()?;
    let _watcher_guard = hot_reload::start_watch(cfg_path, {
        let cfg = cfg.clone();
        let backend = backend.clone();
        let layout = layout.clone();
        let monitor_layout = monitor_layout.clone();
        let rules = rules.clone();
        let icons = icons.clone();
        move || {
            let cfg = cfg.clone();
            let backend = backend.clone();
            let layout = layout.clone();
            let monitor_layout = monitor_layout.clone();
            let rules = rules.clone();
            let icons = icons.clone();
            rt_handle.spawn(async move {
                if let Err(e) =
                    reload(&cfg, &backend, &layout, &monitor_layout, &rules, &icons).await
                {
                    tracing::error!(error = %e, "reload failed");
                }
            });
        }
    });

    let _player = player;

    event_loop(
        async_rx,
        &backend,
        &cfg,
        &layout,
        &registry,
        &monitor_layout,
        &rules,
        &icons,
        &geom_cache,
        &props_cache,
        emitter.as_ref(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn event_loop(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<BackendEvent>,
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    cfg: &Arc<Mutex<Config>>,
    layout: &Arc<Mutex<LayoutState>>,
    registry: &Arc<Mutex<WindowRegistry>>,
    monitor_layout: &Arc<Mutex<MonitorLayout>>,
    rules: &Arc<Mutex<AppRules>>,
    icons: &Arc<IconCache>,
    geom_cache: &GeomCache,
    props_cache: &PropsCache,
    emitter: Option<&Emitter>,
) -> Result<()> {
    // Track the window id we last decorated so signal subscribers
    // can correlate `LayoutChanged` with a window. `0` means "no
    // active window known yet"; this is what the original xxkb
    // daemon also does on the wire.
    let mut last_active_wid: u32 = 0;

    while let Some(event) = rx.recv().await {
        match event {
            BackendEvent::LayoutChanged { new_group, kind } => {
                if let Ok(g) = Group::new(new_group, 4) {
                    layout.lock().observe(g);
                    let trigger = match kind {
                        SwitchKind::Auto => Trigger::Auto,
                        _ => Trigger::Manual,
                    };
                    let _ = trigger;
                    repaint_main_indicators(backend, monitor_layout, cfg, icons, g).await;
                    repaint_all_per_window_indicators(backend, registry, cfg, icons, g).await;
                    if let Some(em) = emitter {
                        if let Err(e) = em.layout_changed(g.as_one_based(), last_active_wid).await {
                            tracing::debug!(error = %e, "LayoutChanged signal emit failed");
                        }
                    }
                }
            }
            BackendEvent::ActiveWindowChanged { wid, props, geom } => {
                if let Some(w) = wid {
                    last_active_wid = w.0;
                }
                if let (Some(wid), Some(props)) = (wid, props) {
                    let verdict = rules.lock().verdict(&props);
                    if matches!(verdict, Verdict::Ignore) {
                        registry.lock().forget(wid);
                    } else {
                        let remembered = registry.lock().get(wid);
                        if let Some(g) = remembered {
                            let _ = backend.lock().await.set_group(g).await;
                        }
                    }
                    props_cache.lock().insert(wid, props.clone());
                    if let Some(g) = geom {
                        geom_cache.lock().insert(wid, g);
                    }
                    let current_group = layout.lock().current();
                    apply_per_window_indicator(backend, cfg, icons, wid, geom, current_group)
                        .await?;
                }
            }
            BackendEvent::WindowGeometryChanged { wid, geom } => {
                geom_cache.lock().insert(wid, geom);
                let current_group = layout.lock().current();
                apply_per_window_indicator(backend, cfg, icons, wid, Some(geom), current_group)
                    .await?;
            }
            BackendEvent::WindowDestroyed { wid } => {
                registry.lock().drop_window(wid);
                geom_cache.lock().remove(&wid);
                props_cache.lock().remove(&wid);
                let _ = backend.lock().await.remove_window_indicator(wid).await;
            }
            BackendEvent::MonitorsChanged { outputs } => {
                monitor_layout.lock().update_outputs(outputs);
                place_initial_main_indicators(backend, monitor_layout, cfg, layout, icons).await?;
            }
            BackendEvent::IndicatorClicked {
                target,
                button,
                ctrl,
                shift: _,
            } => {
                if !ctrl && matches!(button, MouseButton::Left) {
                    cycle_layout(backend, layout).await;
                }
                let _ = target;
            }
            BackendEvent::IndicatorDragged { target, new_origin } => {
                if let IndicatorTarget::Main(output_name) = target {
                    if let Err(e) =
                        save_main_position(cfg, monitor_layout, &output_name, new_origin)
                    {
                        tracing::warn!(error = %e, "failed to persist dragged position");
                    }
                }
                // Per-window drag is intentionally a no-op: the offset is
                // configured globally and applies relative to each window's
                // title bar, so per-instance drag wouldn't have a stable
                // reference frame anyway.
            }
            BackendEvent::WindowCreated { .. } => {
                // Wired up via _NET_ACTIVE_WINDOW transitions in
                // `ActiveWindowChanged`. A separate WindowCreated path
                // would let us decorate inactive windows too — that's a
                // follow-up.
            }
        }
    }
    Ok(())
}

async fn cycle_layout(
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    layout: &Arc<Mutex<LayoutState>>,
) {
    let next = layout.lock().next_cycle();
    if let Err(e) = backend.lock().await.set_group(next).await {
        tracing::warn!(error = %e, "set_group failed on cycle");
    }
}

fn save_main_position(
    cfg: &Arc<Mutex<Config>>,
    monitor_layout: &Arc<Mutex<MonitorLayout>>,
    output_name: &str,
    new_origin: Point,
) -> Result<()> {
    let name = OutputName::from(output_name.to_owned());
    monitor_layout
        .lock()
        .save_position(name.clone(), new_origin);
    {
        let mut cfg_guard = cfg.lock();
        cfg_guard.main_indicator.positions.insert(name, new_origin);
    }
    let path = xxkb_config::config_path()?;
    let snapshot = cfg.lock().clone();
    snapshot.save_to(&path)?;
    Ok(())
}

async fn apply_per_window_indicator(
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    cfg: &Arc<Mutex<Config>>,
    icons: &Arc<IconCache>,
    wid: WindowId,
    geom: Option<WindowGeom>,
    group: Group,
) -> Result<()> {
    let pw = cfg.lock().per_window_indicator.clone();
    if !pw.enable {
        return Ok(());
    }
    let Some(geom) = geom else {
        // We have no idea where the window is (yet); skip placement.
        // A subsequent WindowGeometryChanged event will retry.
        return Ok(());
    };
    let placement =
        IndicatorPlacement::compute(geom.origin, geom.width, geom.frame, pw.offset, pw.size_px);
    backend
        .lock()
        .await
        .place_window_indicator(wid, placement, pw.size_px)
        .await
        .ok();
    let buf = match flag::render_per_window(icons, &cfg.lock(), group) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, group = group.as_one_based(), "no per-window flag for group");
            return Ok(());
        }
    };
    let _ = backend.lock().await.paint_window_indicator(wid, buf).await;
    Ok(())
}

async fn place_initial_main_indicators(
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    monitor_layout: &Arc<Mutex<MonitorLayout>>,
    cfg: &Arc<Mutex<Config>>,
    layout: &Arc<Mutex<LayoutState>>,
    icons: &Arc<IconCache>,
) -> Result<()> {
    let main = cfg.lock().main_indicator.clone();
    if !main.enable {
        return Ok(());
    }
    let outs: Vec<_> = monitor_layout.lock().active().cloned().collect();
    let primary_only = matches!(main.mode, MainIndicatorMode::PrimaryOnly);
    let primary_name = monitor_layout.lock().primary().map(|p| p.name.0.clone());
    let group = layout.lock().current();
    let buf = match flag::render_main(icons, &cfg.lock(), group) {
        Ok(b) => Some(b),
        Err(e) => {
            tracing::warn!(error = %e, group = group.as_one_based(), "no main flag for group");
            None
        }
    };
    for o in outs {
        if primary_only {
            if let Some(name) = &primary_name {
                if &o.name.0 != name {
                    continue;
                }
            }
        }
        let p = monitor_layout.lock().position_for(&o, main.size_px);
        let _ = backend
            .lock()
            .await
            .place_main_indicator(&o.name.0, p, main.size_px)
            .await;
        if let Some(buf) = buf.clone() {
            let _ = backend
                .lock()
                .await
                .paint_main_indicator(&o.name.0, buf)
                .await;
        }
    }
    Ok(())
}

async fn repaint_main_indicators(
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    monitor_layout: &Arc<Mutex<MonitorLayout>>,
    cfg: &Arc<Mutex<Config>>,
    icons: &Arc<IconCache>,
    group: Group,
) {
    let buf = match flag::render_main(icons, &cfg.lock(), group) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, group = group.as_one_based(), "skip main repaint");
            return;
        }
    };
    let names: Vec<_> = monitor_layout
        .lock()
        .active()
        .map(|o| o.name.0.clone())
        .collect();
    for name in names {
        let _ = backend
            .lock()
            .await
            .paint_main_indicator(&name, buf.clone())
            .await;
    }
}

async fn repaint_all_per_window_indicators(
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    registry: &Arc<Mutex<WindowRegistry>>,
    cfg: &Arc<Mutex<Config>>,
    icons: &Arc<IconCache>,
    group: Group,
) {
    let buf = match flag::render_per_window(icons, &cfg.lock(), group) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, group = group.as_one_based(), "skip per-window repaint");
            return;
        }
    };
    let wids: Vec<_> = registry.lock().iter().map(|(w, _)| w).collect();
    for wid in wids {
        let _ = backend
            .lock()
            .await
            .paint_window_indicator(wid, buf.clone())
            .await;
    }
}

async fn reload(
    cfg: &Arc<Mutex<Config>>,
    backend: &Arc<tokio::sync::Mutex<X11Backend>>,
    layout: &Arc<Mutex<LayoutState>>,
    monitor_layout: &Arc<Mutex<MonitorLayout>>,
    rules: &Arc<Mutex<AppRules>>,
    icons: &Arc<IconCache>,
) -> Result<()> {
    tracing::info!("reloading config");
    let new_cfg = Config::load_default()?;
    *layout.lock() = {
        let cur = layout.lock().current();
        let max = layout.lock().max_groups();
        let mut s = LayoutState::new(max, cur, build_two_state(&new_cfg));
        s.observe(cur);
        s
    };
    *rules.lock() = build_rules(&new_cfg);
    {
        let mut ml = monitor_layout.lock();
        for (k, v) in &new_cfg.main_indicator.positions {
            ml.save_position(k.clone(), *v);
        }
    }
    *cfg.lock() = new_cfg;
    // Sizes / borders / mappings may all have changed.
    icons.clear();
    place_initial_main_indicators(backend, monitor_layout, cfg, layout, icons).await?;
    Ok(())
}

fn build_two_state(cfg: &Config) -> TwoStateConfig {
    TwoStateConfig {
        enabled: cfg.general.two_state,
        base: Group::from_one_based(cfg.general.base_group, 4)
            .unwrap_or_else(|_| Group::new(0, 4).unwrap()),
        alt: Group::from_one_based(cfg.general.alt_group, 4)
            .unwrap_or_else(|_| Group::new(1, 4).unwrap()),
    }
}

fn build_rules(cfg: &Config) -> AppRules {
    AppRules::build(&cfg.app_rules, cfg.general.ignore_reverse).unwrap_or_else(|e| {
        tracing::error!(error = %e, "rules disabled due to bad config");
        AppRules::build(&[], false).unwrap()
    })
}

/// Live state shared between the X event loop, the inotify watcher,
/// and the D-Bus interface. Holding `Arc<Mutex<...>>` of all the
/// subsystems lets the bus's `Reload` method execute the *same* full
/// reload path as the file watcher — i.e. it rebuilds rules, clears
/// the icon cache, repaints indicators, and so on, instead of just
/// swapping `cfg` in memory.
struct DaemonHandle {
    cfg: Arc<Mutex<Config>>,
    backend: Arc<tokio::sync::Mutex<X11Backend>>,
    layout: Arc<Mutex<LayoutState>>,
    /// Held to keep the registry alive for parity with other state and
    /// to allow future D-Bus methods (e.g. `RememberLayout`) to mutate
    /// it without further plumbing.
    #[allow(dead_code)]
    registry: Arc<Mutex<WindowRegistry>>,
    monitor_layout: Arc<Mutex<MonitorLayout>>,
    rules: Arc<Mutex<AppRules>>,
    icons: Arc<IconCache>,
    props_cache: PropsCache,
}

#[async_trait]
impl DaemonInterface for DaemonHandle {
    async fn reload(&self) -> Result<(), String> {
        reload(
            &self.cfg,
            &self.backend,
            &self.layout,
            &self.monitor_layout,
            &self.rules,
            &self.icons,
        )
        .await
        .map_err(|e| e.to_string())
    }

    async fn outputs(&self) -> Result<Vec<WireOutput>, String> {
        Ok(self
            .monitor_layout
            .lock()
            .active()
            .map(|o| WireOutput {
                name: o.name.0.clone(),
                x: o.geometry.origin.x,
                y: o.geometry.origin.y,
                width: o.geometry.width,
                height: o.geometry.height,
                is_primary: o.is_primary,
                is_active: o.is_active,
            })
            .collect())
    }

    async fn active_windows(&self) -> Result<Vec<WireWindow>, String> {
        let cache = self.props_cache.lock();
        Ok(cache
            .iter()
            .map(|(wid, props)| WireWindow {
                wid: wid.0,
                wm_class_class: props.wm_class_class.clone(),
                wm_class_name: props.wm_class_name.clone(),
                wm_name: props.wm_name.clone(),
            })
            .collect())
    }

    async fn save_positions(&self, positions: HashMap<String, (i32, i32)>) -> Result<(), String> {
        let mut cfg = self.cfg.lock();
        for (k, (x, y)) in positions {
            // Persist into the live monitor_layout too so the next
            // `place_main_indicator` consults up-to-date overrides
            // without waiting for a Reload roundtrip.
            self.monitor_layout
                .lock()
                .save_position(OutputName::from(k.clone()), Point::new(x, y));
            cfg.main_indicator
                .positions
                .insert(OutputName::from(k), Point::new(x, y));
        }
        let path = xxkb_config::config_path().map_err(|e| e.to_string())?;
        cfg.save_to(&path).map_err(|e| e.to_string())?;
        Ok(())
    }
}
