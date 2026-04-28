//! In-process integration tests for the `org.xxkb.Daemon1` interface.
//!
//! These tests do *not* require a session bus to be present. We
//! connect a server-side `DaemonService` and a client-side
//! `DaemonProxy` over a `UnixStream` pair using zbus' p2p mode, so
//! the entire test runs inside one tokio runtime without any
//! external `dbus-daemon` process.
//!
//! That makes the suite trivially CI-friendly: no `dbus-launch`, no
//! `XDG_RUNTIME_DIR` setup, no orphaned daemons. We exercise:
//!
//! 1. method roundtrips with realistic payloads (`GetMonitors`,
//!    `GetActiveWindows`, `SaveCurrentPositions`),
//! 2. `Reload` propagation of failures back to the caller as
//!    `fdo::Error::Failed("...")`,
//! 3. `Reloaded` / `PositionsSaved` / `LayoutChanged` signal
//!    delivery (subscribe, fire, observe).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::StreamExt;
use parking_lot::Mutex;
use tokio::net::UnixStream as TokioUnixStream;
use xxkb_dbus::{
    DaemonInterface, DaemonProxy, DaemonService, Emitter, WireOutput, WireWindow, DAEMON_PATH,
};
use zbus::Guid;

/// Stub `DaemonInterface` impl with a per-test toggle for `Reload`
/// success and a counter for invocations. Everything is `Mutex<>`-
/// protected so the test can flip behaviour and inspect state from
/// the outside without `&mut self`.
struct StubDaemon {
    monitors: Vec<WireOutput>,
    windows: Vec<WireWindow>,
    saved_positions: Mutex<Vec<HashMap<String, (i32, i32)>>>,
    reload_should_fail: Mutex<bool>,
    reload_calls: Mutex<u32>,
}

impl StubDaemon {
    fn new() -> Self {
        Self {
            monitors: vec![
                WireOutput {
                    name: "DP-1".into(),
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    is_primary: true,
                    is_active: true,
                },
                WireOutput {
                    name: "HDMI-A-1".into(),
                    x: 1920,
                    y: 0,
                    width: 1280,
                    height: 1024,
                    is_primary: false,
                    is_active: true,
                },
            ],
            windows: vec![WireWindow {
                wid: 42,
                wm_class_class: "Firefox".into(),
                wm_class_name: "Navigator".into(),
                wm_name: "Cargo - rust-lang/cargo".into(),
            }],
            saved_positions: Mutex::new(Vec::new()),
            reload_should_fail: Mutex::new(false),
            reload_calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl DaemonInterface for StubDaemon {
    async fn reload(&self) -> Result<(), String> {
        *self.reload_calls.lock() += 1;
        if *self.reload_should_fail.lock() {
            Err("synthetic failure".into())
        } else {
            Ok(())
        }
    }
    async fn outputs(&self) -> Result<Vec<WireOutput>, String> {
        Ok(self.monitors.clone())
    }
    async fn active_windows(&self) -> Result<Vec<WireWindow>, String> {
        Ok(self.windows.clone())
    }
    async fn save_positions(&self, positions: HashMap<String, (i32, i32)>) -> Result<(), String> {
        self.saved_positions.lock().push(positions);
        Ok(())
    }
}

/// Test harness: spin up the server and a connected client. Returns
/// both connections (drop them last to keep the link alive) plus the
/// stub for state introspection and a typed proxy for calls.
struct Harness {
    _server: zbus::Connection,
    client: zbus::Connection,
    stub: Arc<StubDaemon>,
}

impl Harness {
    async fn spin_up() -> zbus::Result<Self> {
        let (a, b) = TokioUnixStream::pair()?;
        let stub = Arc::new(StubDaemon::new());

        // Critical: both ends of a p2p auth handshake have to make
        // progress concurrently, otherwise `build()` on one side
        // blocks reading bytes the *other* side has not yet sent.
        // zbus' own e2e tests use `try_join!` for the same reason —
        // a sequential `await` would deadlock.
        let server_build = zbus::connection::Builder::unix_stream(a)
            .p2p()
            .server(Guid::generate())?
            .serve_at(DAEMON_PATH, DaemonService::new(stub.clone()))?
            .build();
        let client_build = zbus::connection::Builder::unix_stream(b).p2p().build();

        let (server, client) = futures::try_join!(server_build, client_build)?;

        Ok(Self {
            _server: server,
            client,
            stub,
        })
    }

    /// Build a typed proxy over the p2p client connection.
    ///
    /// In p2p there's no bus name to address; we set the destination
    /// to an arbitrary non-empty placeholder because the
    /// `#[proxy(default_service = "...")]` attribute hard-codes one
    /// and zbus refuses an empty destination on a regular `Proxy`.
    /// In p2p mode the destination header is sent but ignored by
    /// the peer, so any value works.
    async fn proxy(&self) -> zbus::Result<DaemonProxy<'_>> {
        DaemonProxy::builder(&self.client)
            .destination(xxkb_dbus::DAEMON_BUS)?
            .path(DAEMON_PATH)?
            .build()
            .await
    }
}

#[tokio::test]
async fn version_and_ping_round_trip() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    assert_eq!(p.ping().await?, "pong");
    assert_eq!(p.version().await?, env!("CARGO_PKG_VERSION"));
    Ok(())
}

#[tokio::test]
async fn get_monitors_returns_full_payload() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    let mons = p.get_monitors().await?;
    assert_eq!(mons.len(), 2);
    assert_eq!(mons[0].name, "DP-1");
    assert!(mons[0].is_primary);
    assert_eq!(mons[1].name, "HDMI-A-1");
    assert!(!mons[1].is_primary);
    Ok(())
}

#[tokio::test]
async fn get_active_windows_returns_props() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    let wins = p.get_active_windows().await?;
    assert_eq!(wins.len(), 1);
    assert_eq!(wins[0].wid, 42);
    assert_eq!(wins[0].wm_class_class, "Firefox");
    assert_eq!(wins[0].wm_name, "Cargo - rust-lang/cargo");
    Ok(())
}

#[tokio::test]
async fn save_current_positions_persists_and_signals() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;

    let mut signals = p.receive_positions_saved().await?;

    let mut pos = HashMap::new();
    pos.insert("DP-1".to_owned(), (100, 200));
    pos.insert("HDMI-A-1".to_owned(), (1920, 0));
    p.save_current_positions(pos.clone()).await?;

    // Wait for the signal — bounded so a regression doesn't hang CI.
    let sig = tokio::time::timeout(std::time::Duration::from_secs(2), signals.next())
        .await
        .expect("PositionsSaved signal not delivered within 2s")
        .expect("signal stream ended unexpectedly");
    assert_eq!(sig.args()?.count, 2);

    let saved = h.stub.saved_positions.lock();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0], pos);
    Ok(())
}

#[tokio::test]
async fn reload_failure_is_propagated_and_signalled() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    let mut signals = p.receive_reloaded().await?;

    *h.stub.reload_should_fail.lock() = true;
    let err = p.reload().await.expect_err("reload should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("synthetic failure"),
        "expected fdo error string to carry the daemon's reason, got: {msg}"
    );

    let sig = tokio::time::timeout(std::time::Duration::from_secs(2), signals.next())
        .await
        .expect("Reloaded signal not delivered within 2s")
        .expect("signal stream ended unexpectedly");
    assert!(
        !sig.args()?.ok,
        "Reloaded(ok=true) should NOT fire on failure"
    );
    assert_eq!(*h.stub.reload_calls.lock(), 1);
    Ok(())
}

#[tokio::test]
async fn reload_success_fires_reloaded_true() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    let mut signals = p.receive_reloaded().await?;

    p.reload().await?;
    let sig = tokio::time::timeout(std::time::Duration::from_secs(2), signals.next())
        .await
        .expect("Reloaded signal not delivered within 2s")
        .expect("signal stream ended unexpectedly");
    assert!(sig.args()?.ok);
    Ok(())
}

#[tokio::test]
async fn emitter_pushes_layout_changed() -> zbus::Result<()> {
    let h = Harness::spin_up().await?;
    let p = h.proxy().await?;
    let mut signals = p.receive_layout_changed().await?;

    // Build an emitter wrapped around the *server* side connection
    // and push a synthetic group switch. This mirrors what the daemon
    // does from inside its X event loop.
    let emitter = Emitter::new(h._server.clone());
    emitter.layout_changed(2, 0xdead_beef).await.unwrap();

    let sig = tokio::time::timeout(std::time::Duration::from_secs(2), signals.next())
        .await
        .expect("LayoutChanged signal not delivered within 2s")
        .expect("signal stream ended unexpectedly");
    let args = sig.args()?;
    assert_eq!(args.group_one_based, 2);
    assert_eq!(args.wid, 0xdead_beef);
    Ok(())
}
