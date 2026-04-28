//! D-Bus client helpers for talking to a running `xxkbd` daemon.
//!
//! The configurator GUI calls these from background threads (or from
//! the GTK main loop via `glib::MainContext::spawn_local`) to:
//!
//! * notify the daemon that the config file changed
//!   ([`ping_reload`] — also useful when the GUI just saved and we
//!   want to bypass the inotify debounce);
//! * fetch the live monitor list to render the position editor
//!   ([`fetch_monitors`]);
//! * fetch the active windows for the rules editor's "Capture" button
//!   ([`fetch_active_windows`]);
//! * push dragged-then-confirmed positions back onto the daemon
//!   ([`save_positions`]).
//!
//! Internally we now use the typed [`xxkb_dbus::DaemonProxy`]
//! generated from `#[zbus::proxy]`, instead of stringly-typed
//! `Proxy::call_method("Foo", &(...))` calls. That gives us:
//!
//! * a single source of truth for method names + signatures (the
//!   `pub trait Daemon` in `xxkb-dbus`),
//! * compile-time verification that the GUI and daemon agree on the
//!   wire types (changes to `WireOutput`/`WireWindow` propagate
//!   automatically),
//! * easy access to signal streams for future
//!   "auto-refresh on `LayoutChanged`" UI work.

use std::collections::HashMap;

use futures::StreamExt;
use thiserror::Error;
use xxkb_dbus::{DaemonProxy, WireOutput, WireWindow};

/// Errors from the D-Bus client.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Underlying zbus error (connect, call, deserialise).
    #[error(transparent)]
    Zbus(#[from] zbus::Error),
    /// The daemon returned an `fdo` error string.
    #[error("daemon returned error: {0}")]
    Daemon(String),
}

impl From<zbus::fdo::Error> for ClientError {
    fn from(e: zbus::fdo::Error) -> Self {
        Self::Daemon(e.to_string())
    }
}

/// Connect to the session bus and call `Reload` on the daemon.
///
/// Returns `Ok(())` if the call completed successfully *or* if the
/// daemon is not currently registered on the bus — the GUI uses this
/// to mean "best-effort nudge", and a missing daemon is not actually
/// an error from the user's perspective.
pub async fn ping_reload() -> Result<(), ClientError> {
    let conn = zbus::Connection::session().await?;
    if !xxkb_dbus::is_daemon_present(&conn).await? {
        tracing::debug!("daemon not on bus; skipping reload ping");
        return Ok(());
    }
    let proxy = DaemonProxy::new(&conn).await?;
    proxy.reload().await?;
    Ok(())
}

/// Fetch a snapshot of currently-known RandR outputs.
pub async fn fetch_monitors() -> Result<Vec<WireOutput>, ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = DaemonProxy::new(&conn).await?;
    Ok(proxy.get_monitors().await?)
}

/// Fetch a snapshot of the active windows the daemon is currently
/// tracking.
pub async fn fetch_active_windows() -> Result<Vec<WireWindow>, ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = DaemonProxy::new(&conn).await?;
    Ok(proxy.get_active_windows().await?)
}

/// Save a `(output_name -> (x, y))` map of positions, replacing any
/// previously-saved values. The daemon also writes them to disk so
/// the GUI doesn't need to follow up with [`ping_reload`].
pub async fn save_positions(positions: HashMap<String, (i32, i32)>) -> Result<(), ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = DaemonProxy::new(&conn).await?;
    proxy.save_current_positions(positions).await?;
    Ok(())
}

/// Liveness probe — returns the daemon's `Version()` reply when the
/// service answers, `Ok(None)` when the well-known name is not on
/// the bus. Exposed so the configurator can render a
/// "daemon: 0.1.0 / not running" badge without surfacing
/// `ServiceUnknown` errors.
pub async fn daemon_version() -> Result<Option<String>, ClientError> {
    let conn = zbus::Connection::session().await?;
    if !xxkb_dbus::is_daemon_present(&conn).await? {
        return Ok(None);
    }
    let proxy = DaemonProxy::new(&conn).await?;
    Ok(Some(proxy.version().await?))
}

/// Spawn a background thread that listens for `PositionsSaved` on the session
/// bus and invokes `on_saved(count)` for each signal.
///
/// Intended for the GTK configurator: wrap `on_saved` in
/// `glib::MainContext::default().invoke` so toasts run on the UI thread.
///
/// If the session bus is unavailable or the stream ends, the thread exits
/// quietly (errors are logged at `debug` / `warn`).
pub fn spawn_positions_saved_listener<F>(on_saved: F)
where
    F: Fn(u32) + Send + 'static,
{
    let r = std::thread::Builder::new()
        .name("xxkb-positions-saved".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "tokio runtime for PositionsSaved listener");
                    return;
                }
            };
            if let Err(e) = rt.block_on(async {
                let conn = zbus::Connection::session().await?;
                let proxy = DaemonProxy::new(&conn).await?;
                let mut stream = proxy.receive_positions_saved().await?;
                while let Some(msg) = stream.next().await {
                    match msg.args() {
                        Ok(args) => on_saved(args.count),
                        Err(err) => {
                            tracing::warn!(error = %err, "positions_saved signal args");
                        }
                    }
                }
                Ok::<(), ClientError>(())
            }) {
                tracing::debug!(error = %e, "PositionsSaved listener stopped");
            }
        });
    if let Err(e) = r {
        tracing::warn!(error = %e, "could not spawn PositionsSaved listener thread");
    }
}

/// Convenience wrappers that block the calling thread until the call
/// completes. The configurator runs them on a dedicated worker thread
/// so they never freeze the GTK main loop.
pub mod blocking {
    use std::collections::HashMap;

    use xxkb_dbus::{WireOutput, WireWindow};

    use super::ClientError;

    /// Blocking version of [`super::ping_reload`].
    pub fn ping_reload() -> Result<(), ClientError> {
        with_runtime(super::ping_reload())
    }

    /// Blocking version of [`super::fetch_monitors`].
    pub fn fetch_monitors() -> Result<Vec<WireOutput>, ClientError> {
        with_runtime(super::fetch_monitors())
    }

    /// Blocking version of [`super::fetch_active_windows`].
    pub fn fetch_active_windows() -> Result<Vec<WireWindow>, ClientError> {
        with_runtime(super::fetch_active_windows())
    }

    /// Blocking version of [`super::save_positions`].
    pub fn save_positions(positions: HashMap<String, (i32, i32)>) -> Result<(), ClientError> {
        with_runtime(super::save_positions(positions))
    }

    /// Blocking version of [`super::daemon_version`].
    pub fn daemon_version() -> Result<Option<String>, ClientError> {
        with_runtime(super::daemon_version())
    }

    fn with_runtime<F: std::future::Future>(f: F) -> F::Output {
        // Build a small dedicated runtime per call. The configurator
        // does at most one D-Bus call per user click, so the cost is
        // negligible compared to dragging a tokio runtime through the
        // GTK main loop.
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!(error = %e, "could not build tokio runtime for d-bus call");
                std::process::abort();
            }
        };
        rt.block_on(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// We cannot actually exercise the bus in unit tests (CI rarely has
    /// a session bus, and we don't want to spawn `dbus-launch`), but
    /// we can still smoke-check the error conversion path that the
    /// GUI relies on.
    #[test]
    fn fdo_errors_become_daemon_errors() {
        let fdo: zbus::fdo::Error = zbus::fdo::Error::Failed("boom".into());
        let client: ClientError = fdo.into();
        match client {
            ClientError::Daemon(msg) => assert!(msg.contains("boom")),
            _ => panic!("expected ClientError::Daemon"),
        }
    }

    /// Compile-time check that the typed proxy from xxkb-dbus is
    /// actually re-importable here. If a future refactor of
    /// `xxkb-dbus` accidentally drops the public `DaemonProxy`
    /// re-export, this test will fail to compile, which is exactly
    /// the early signal we want.
    #[allow(dead_code)]
    fn _proxy_is_in_scope() {
        fn _accepts_proxy<'a>(_: DaemonProxy<'a>) {}
    }
}
