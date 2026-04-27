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
//!   ([`fetch_active_windows`]).
//!
//! All helpers return `Result<_, ClientError>` instead of bubbling
//! `zbus::Error` directly so callers can render a single tidy error
//! string in the GUI.

use std::collections::HashMap;

use thiserror::Error;
use xxkb_dbus::{WireOutput, WireWindow};

const DAEMON_BUS: &str = "org.xxkb.Daemon1";
const DAEMON_PATH: &str = "/org/xxkb/Daemon1";
const DAEMON_IFACE: &str = "org.xxkb.Daemon1";

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
    if !daemon_present(&conn).await? {
        tracing::debug!("daemon not on bus; skipping reload ping");
        return Ok(());
    }
    let proxy = make_proxy(&conn).await?;
    proxy.call_method("Reload", &()).await?;
    Ok(())
}

/// Fetch a snapshot of currently-known RandR outputs.
pub async fn fetch_monitors() -> Result<Vec<WireOutput>, ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = make_proxy(&conn).await?;
    let reply = proxy.call_method("GetMonitors", &()).await?;
    let outputs: Vec<WireOutput> = reply.body().deserialize()?;
    Ok(outputs)
}

/// Fetch a snapshot of the active windows the daemon is currently
/// tracking.
pub async fn fetch_active_windows() -> Result<Vec<WireWindow>, ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = make_proxy(&conn).await?;
    let reply = proxy.call_method("GetActiveWindows", &()).await?;
    let windows: Vec<WireWindow> = reply.body().deserialize()?;
    Ok(windows)
}

/// Save a `(output_name -> (x, y))` map of positions, replacing any
/// previously-saved values. The daemon also writes them to disk so
/// the GUI doesn't need to follow up with [`ping_reload`].
pub async fn save_positions(positions: HashMap<String, (i32, i32)>) -> Result<(), ClientError> {
    let conn = zbus::Connection::session().await?;
    let proxy = make_proxy(&conn).await?;
    proxy
        .call_method("SaveCurrentPositions", &(positions,))
        .await?;
    Ok(())
}

async fn daemon_present(conn: &zbus::Connection) -> Result<bool, ClientError> {
    let proxy = zbus::fdo::DBusProxy::new(conn).await?;
    let owners = proxy.list_names().await?;
    Ok(owners.iter().any(|n| n.as_str() == DAEMON_BUS))
}

async fn make_proxy(conn: &zbus::Connection) -> Result<zbus::Proxy<'_>, ClientError> {
    Ok(zbus::Proxy::new(conn, DAEMON_BUS, DAEMON_PATH, DAEMON_IFACE).await?)
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
    /// a session bus), but we can still smoke-check the constants and
    /// the error conversions.
    #[test]
    fn constants_are_canonical() {
        assert_eq!(DAEMON_BUS, "org.xxkb.Daemon1");
        assert_eq!(DAEMON_PATH, "/org/xxkb/Daemon1");
        assert_eq!(DAEMON_IFACE, "org.xxkb.Daemon1");
    }

    #[test]
    fn fdo_errors_become_daemon_errors() {
        let fdo: zbus::fdo::Error = zbus::fdo::Error::Failed("boom".into());
        let client: ClientError = fdo.into();
        assert!(matches!(client, ClientError::Daemon(_)));
    }
}
