//! D-Bus service `org.xxkb.Daemon1` on the session bus.
//!
//! This crate is the canonical source of truth for the IPC contract
//! between the running `xxkbd` daemon and any external client (the
//! configurator GUI, status-bar widgets, ad-hoc scripts, future
//! systemd-managed tooling, …).
//!
//! ## Surface
//!
//! ### Methods
//!
//! | Member                    | Args                                          | Returns        |
//! |---------------------------|-----------------------------------------------|----------------|
//! | `Reload`                  | —                                             | —              |
//! | `GetMonitors`             | —                                             | `a(siiqqbb)`   |
//! | `GetActiveWindows`        | —                                             | `a(uss s)`     |
//! | `SaveCurrentPositions`    | `a{s(ii)}`                                    | —              |
//! | `Version`                 | —                                             | `s`            |
//! | `Ping`                    | —                                             | `s` (`"pong"`) |
//!
//! ### Signals
//!
//! | Member            | Args                              | Meaning                                |
//! |-------------------|-----------------------------------|----------------------------------------|
//! | `LayoutChanged`   | `(group_one_based: u8, wid: u32)` | active group switched                  |
//! | `Reloaded`        | `(ok: bool)`                      | `Reload` finished (success/failure)    |
//! | `PositionsSaved`  | `(count: u32)`                    | drag positions persisted to disk       |
//!
//! ## Why non-generic exporter
//!
//! The daemon emits signals from places that are *not* inside a method
//! handler (e.g. when the X server announces a layout switch). The
//! cleanest way to reach the live signal context is via
//! `Connection::object_server().interface::<_, DaemonService>(path)`,
//! which requires the interface type to be statically nameable. A
//! generic `DaemonExporter<T>` would force every emission site to know
//! `T`. The sealed `DaemonService` (which itself holds a
//! `Arc<dyn DaemonInterface>`) avoids that.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zbus::object_server::{InterfaceRef, SignalContext};
use zbus::{interface, zvariant::Type};

/// Canonical session-bus well-known name.
pub const DAEMON_BUS: &str = "org.xxkb.Daemon1";

/// Canonical object path the service is registered at.
pub const DAEMON_PATH: &str = "/org/xxkb/Daemon1";

/// Canonical interface name (matches [`DAEMON_BUS`]).
pub const DAEMON_IFACE: &str = "org.xxkb.Daemon1";

/// Errors from the D-Bus subsystem.
#[derive(Debug, Error)]
pub enum DbusError {
    /// Underlying zbus error.
    #[error(transparent)]
    Zbus(#[from] zbus::Error),
    /// We could not acquire the well-known name on the bus.
    ///
    /// Typically means another `xxkbd` instance is already running.
    #[error("could not acquire {DAEMON_BUS}: {0}")]
    NameTaken(String),
}

/// Wire-format of a RandR output, sent over the bus.
///
/// We deliberately use plain primitives (no internal newtypes) so the
/// D-Bus signature is stable and easy for non-Rust clients (e.g. a
/// `gjs` shell extension or `dbus-send` from the command line) to
/// consume.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct WireOutput {
    /// Stable RandR output name (e.g. `DP-1`, `eDP-1`, `HDMI-A-2`).
    pub name: String,
    /// X coordinate of the top-left corner, in screen pixels.
    pub x: i32,
    /// Y coordinate of the top-left corner, in screen pixels.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Whether RandR marks this output as the primary one.
    pub is_primary: bool,
    /// Whether this output currently has an attached CRTC.
    pub is_active: bool,
}

/// Wire-format of a tracked window, sent over the bus.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct WireWindow {
    /// X11 window id.
    pub wid: u32,
    /// `WM_CLASS` class part (e.g. `Firefox`).
    pub wm_class_class: String,
    /// `WM_CLASS` instance part (e.g. `Navigator`).
    pub wm_class_name: String,
    /// `WM_NAME` (or `_NET_WM_NAME` if available).
    pub wm_name: String,
}

/// Async trait the daemon implements; this crate owns the zbus glue.
///
/// Errors are returned as `String` so the daemon can stay free of any
/// zbus types, and so we can render them verbatim in `fdo::Error`
/// replies on the bus.
#[async_trait]
pub trait DaemonInterface: Send + Sync {
    /// Re-read `~/.config/xxkb/config.toml`, rebuild rules, repaint.
    async fn reload(&self) -> Result<(), String>;
    /// Snapshot of the live RandR outputs the daemon currently sees.
    async fn outputs(&self) -> Result<Vec<WireOutput>, String>;
    /// Snapshot of windows the daemon is currently tracking.
    async fn active_windows(&self) -> Result<Vec<WireWindow>, String>;
    /// Persist `(output_name -> (x, y))` positions into config.
    async fn save_positions(&self, positions: HashMap<String, (i32, i32)>) -> Result<(), String>;
    /// Daemon version string. Default: `CARGO_PKG_VERSION` of *this*
    /// crate, which is workspace-pinned.
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }
}

/// Non-generic exporter that zbus registers at [`DAEMON_PATH`].
///
/// See module docs for why we don't use `DaemonExporter<T>`.
pub struct DaemonService {
    inner: Arc<dyn DaemonInterface>,
}

impl DaemonService {
    /// Build a new exporter that forwards calls to `inner`.
    pub fn new(inner: Arc<dyn DaemonInterface>) -> Self {
        Self { inner }
    }
}

#[interface(name = "org.xxkb.Daemon1")]
impl DaemonService {
    /// Re-read config and repaint indicators.
    ///
    /// Always emits a `Reloaded(ok)` signal — even on failure — so
    /// the configurator can flip its "applying…" UI state regardless
    /// of outcome.
    async fn reload(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let res = self.inner.reload().await;
        // Best-effort: a failed signal emit shouldn't mask the
        // method result.
        let _ = Self::reloaded(&ctxt, res.is_ok()).await;
        res.map_err(zbus::fdo::Error::Failed)
    }

    /// Snapshot of monitors.
    async fn get_monitors(&self) -> zbus::fdo::Result<Vec<WireOutput>> {
        self.inner.outputs().await.map_err(zbus::fdo::Error::Failed)
    }

    /// Snapshot of active windows.
    async fn get_active_windows(&self) -> zbus::fdo::Result<Vec<WireWindow>> {
        self.inner
            .active_windows()
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Persist drag-saved positions and write config to disk. Emits
    /// `PositionsSaved(count)` on success only — the failure case is
    /// communicated as an `fdo::Error` reply directly.
    async fn save_current_positions(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        positions: HashMap<String, (i32, i32)>,
    ) -> zbus::fdo::Result<()> {
        let count = u32::try_from(positions.len()).unwrap_or(u32::MAX);
        self.inner
            .save_positions(positions)
            .await
            .map_err(zbus::fdo::Error::Failed)?;
        let _ = Self::positions_saved(&ctxt, count).await;
        Ok(())
    }

    /// Daemon version. Useful for `xxkb-config` to refuse to talk to a
    /// daemon it doesn't understand.
    async fn version(&self) -> String {
        self.inner.version()
    }

    /// Trivial liveness probe — returns `"pong"`. Useful as a
    /// boolean "is daemon answering?" without parsing anything.
    async fn ping(&self) -> &'static str {
        "pong"
    }

    /// Emitted on every layout switch — once per group transition.
    #[zbus(signal)]
    async fn layout_changed(
        ctxt: &SignalContext<'_>,
        group_one_based: u8,
        wid: u32,
    ) -> zbus::Result<()>;

    /// Emitted after a `Reload` call. `ok=false` means the daemon
    /// rolled the failed config back in-memory.
    #[zbus(signal)]
    async fn reloaded(ctxt: &SignalContext<'_>, ok: bool) -> zbus::Result<()>;

    /// Emitted after `SaveCurrentPositions` finishes writing the
    /// config file.
    #[zbus(signal)]
    async fn positions_saved(ctxt: &SignalContext<'_>, count: u32) -> zbus::Result<()>;
}

/// Helper that the daemon uses to push out signals from places that
/// aren't inside an `#[interface]` method handler (e.g. the X event
/// loop on layout switch, the inotify watcher on a successful reload).
///
/// Fail-soft: if the connection has already been torn down or the
/// interface ref can't be resolved, we log and swallow. Signal
/// delivery is best-effort by design — clients that *must* know about
/// the change will re-poll.
#[derive(Clone)]
pub struct Emitter {
    conn: zbus::Connection,
}

impl Emitter {
    /// Wrap a live zbus connection. Cheap; cloning shares the inner
    /// `Arc`.
    pub fn new(conn: zbus::Connection) -> Self {
        Self { conn }
    }

    async fn iface_ref(&self) -> Result<InterfaceRef<DaemonService>, DbusError> {
        let r = self
            .conn
            .object_server()
            .interface::<_, DaemonService>(DAEMON_PATH)
            .await?;
        Ok(r)
    }

    /// Emit `LayoutChanged(group_one_based, wid)`.
    pub async fn layout_changed(&self, group_one_based: u8, wid: u32) -> Result<(), DbusError> {
        let iref = self.iface_ref().await?;
        DaemonService::layout_changed(iref.signal_context(), group_one_based, wid).await?;
        Ok(())
    }

    /// Emit `Reloaded(ok)`.
    pub async fn reloaded(&self, ok: bool) -> Result<(), DbusError> {
        let iref = self.iface_ref().await?;
        DaemonService::reloaded(iref.signal_context(), ok).await?;
        Ok(())
    }

    /// Emit `PositionsSaved(count)`.
    pub async fn positions_saved(&self, count: u32) -> Result<(), DbusError> {
        let iref = self.iface_ref().await?;
        DaemonService::positions_saved(iref.signal_context(), count).await?;
        Ok(())
    }
}

/// Spin up a session-bus connection, register the interface at
/// [`DAEMON_PATH`], claim [`DAEMON_BUS`], and return both the
/// connection (drop = unregister) and an [`Emitter`] for the daemon to
/// push signals through.
pub async fn serve(
    inner: Arc<dyn DaemonInterface>,
) -> Result<(zbus::Connection, Emitter), DbusError> {
    let svc = DaemonService::new(inner);
    let conn = zbus::connection::Builder::session()?
        .name(DAEMON_BUS)
        .map_err(|e| DbusError::NameTaken(e.to_string()))?
        .serve_at(DAEMON_PATH, svc)?
        .build()
        .await?;
    let emitter = Emitter::new(conn.clone());
    Ok((conn, emitter))
}

/// True iff the daemon currently owns the well-known name on the bus.
///
/// Clients use this to render "daemon: running / not running" badges
/// and to avoid surfacing `ServiceUnknown` errors to the user when
/// the daemon is simply not started.
pub async fn is_daemon_present(conn: &zbus::Connection) -> Result<bool, zbus::Error> {
    let proxy = zbus::fdo::DBusProxy::new(conn).await?;
    Ok(proxy
        .list_names()
        .await?
        .iter()
        .any(|n| n.as_str() == DAEMON_BUS))
}

pub use proxy_internal::{DaemonProxy, LayoutChangedStream, PositionsSavedStream, ReloadedStream};

/// Wraps the `#[proxy]`-generated client trait.
///
/// The zbus `#[proxy]` macro emits private helper types (`*Stream`,
/// `*Args`, …) that are not individually documentable from outside
/// the macro. We pin them all behind a submodule with a blanket
/// `#![allow(missing_docs)]` so the surrounding crate can keep the
/// strict `missing_docs` lint without bleeding warnings into the
/// generated code.
mod proxy_internal {
    #![allow(missing_docs)]

    use std::collections::HashMap;

    use zbus::proxy;

    use super::{WireOutput, WireWindow};

    #[proxy(
        interface = "org.xxkb.Daemon1",
        default_service = "org.xxkb.Daemon1",
        default_path = "/org/xxkb/Daemon1"
    )]
    pub trait Daemon {
        fn reload(&self) -> zbus::Result<()>;
        fn get_monitors(&self) -> zbus::Result<Vec<WireOutput>>;
        fn get_active_windows(&self) -> zbus::Result<Vec<WireWindow>>;
        fn save_current_positions(
            &self,
            positions: HashMap<String, (i32, i32)>,
        ) -> zbus::Result<()>;
        fn version(&self) -> zbus::Result<String>;
        fn ping(&self) -> zbus::Result<String>;

        #[zbus(signal)]
        fn layout_changed(&self, group_one_based: u8, wid: u32) -> zbus::Result<()>;
        #[zbus(signal)]
        fn reloaded(&self, ok: bool) -> zbus::Result<()>;
        #[zbus(signal)]
        fn positions_saved(&self, count: u32) -> zbus::Result<()>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// The on-the-wire signature is part of our public ABI: external
    /// clients (D-Bus shell extensions, `dbus-send` invocations,
    /// `gjs` scripts) hard-code these strings. Any change here is a
    /// breaking change to the IPC contract.
    #[test]
    fn wire_output_signature_is_stable() {
        assert_eq!(WireOutput::signature().as_str(), "(siiuubb)");
    }

    #[test]
    fn wire_window_signature_is_stable() {
        assert_eq!(WireWindow::signature().as_str(), "(usss)");
    }

    #[test]
    fn constants_are_canonical() {
        assert_eq!(DAEMON_BUS, "org.xxkb.Daemon1");
        assert_eq!(DAEMON_PATH, "/org/xxkb/Daemon1");
        assert_eq!(DAEMON_IFACE, "org.xxkb.Daemon1");
    }

    /// `version()` falls back to the crate version when the impl
    /// doesn't override it. Guards against a future refactor that
    /// silently drops the default impl and starts returning `""`.
    #[test]
    fn default_version_uses_crate_version() {
        struct Stub;
        #[async_trait]
        impl DaemonInterface for Stub {
            async fn reload(&self) -> Result<(), String> {
                Ok(())
            }
            async fn outputs(&self) -> Result<Vec<WireOutput>, String> {
                Ok(vec![])
            }
            async fn active_windows(&self) -> Result<Vec<WireWindow>, String> {
                Ok(vec![])
            }
            async fn save_positions(&self, _: HashMap<String, (i32, i32)>) -> Result<(), String> {
                Ok(())
            }
        }
        assert_eq!(Stub.version(), env!("CARGO_PKG_VERSION"));
    }
}
