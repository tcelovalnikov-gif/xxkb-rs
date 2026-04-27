//! Backend trait and the real `x11rb`-based implementation.
//!
//! Most of the daemon doesn't depend on x11rb directly — it depends on
//! [`Backend`]. This is what makes it testable: in `xxkb-test-utils` we
//! provide a `MockBackend` that the daemon's main loop can drive
//! synchronously.

use std::sync::{mpsc, Arc};

use async_trait::async_trait;
use xxkb_core::{layout::Group, monitors::Output, registry::WindowId, Point};
use xxkb_indicators::PixelBuffer;

use crate::{errors::X11Error, events::BackendEvent};

/// What the daemon needs from any X-like backend.
#[async_trait]
pub trait Backend: Send {
    /// Connect to the server and announce the extensions we need.
    async fn connect(&mut self) -> Result<(), X11Error>;

    /// Tell the X server to switch to `group` immediately.
    async fn set_group(&mut self, group: Group) -> Result<(), X11Error>;

    /// Read the current group (0-based) from the server.
    async fn current_group(&mut self) -> Result<Group, X11Error>;

    /// Snapshot of all RandR outputs.
    async fn outputs(&mut self) -> Result<Vec<Output>, X11Error>;

    /// Move (or create) the main indicator on `output_name` to `point`.
    async fn place_main_indicator(
        &mut self,
        output_name: &str,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error>;

    /// Move (or create) the per-window indicator overlaid on `wid`.
    async fn place_window_indicator(
        &mut self,
        wid: WindowId,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error>;

    /// Destroy the per-window indicator overlaid on `wid` (if any).
    async fn remove_window_indicator(&mut self, wid: WindowId) -> Result<(), X11Error>;

    /// Upload `buf` and set it as the background of the main indicator
    /// for `output_name`. Returns `Ok(false)` if no indicator exists
    /// for that output yet.
    async fn paint_main_indicator(
        &mut self,
        output_name: &str,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error>;

    /// Upload `buf` and set it as the background of the per-window
    /// indicator overlaying `wid`.
    async fn paint_window_indicator(
        &mut self,
        wid: WindowId,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error>;

    /// Get a receiver of [`BackendEvent`]s.
    ///
    /// Calling this twice is undefined; it should be called exactly once
    /// at startup.
    fn take_event_rx(&mut self) -> Option<mpsc::Receiver<BackendEvent>>;
}

/// Real backend.
///
/// Implementation detail lives in this module; it spawns a worker thread
/// owning the `x11rb` connection and an `mpsc::Sender<BackendEvent>` is
/// fed from there.
pub struct X11Backend {
    inner: Option<inner::Worker>,
    rx: Option<mpsc::Receiver<BackendEvent>>,
}

impl X11Backend {
    /// Build a new (disconnected) backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: None,
            rx: None,
        }
    }
}

impl Default for X11Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for X11Backend {
    async fn connect(&mut self) -> Result<(), X11Error> {
        let (worker, rx) = inner::Worker::spawn()?;
        self.inner = Some(worker);
        self.rx = Some(rx);
        Ok(())
    }

    async fn set_group(&mut self, group: Group) -> Result<(), X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .set_group(group)
    }

    async fn current_group(&mut self) -> Result<Group, X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .current_group()
    }

    async fn outputs(&mut self) -> Result<Vec<Output>, X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .outputs()
    }

    async fn place_main_indicator(
        &mut self,
        output_name: &str,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .place_main_indicator(output_name, point, size)
    }

    async fn place_window_indicator(
        &mut self,
        wid: WindowId,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .place_window_indicator(wid, point, size)
    }

    async fn remove_window_indicator(&mut self, wid: WindowId) -> Result<(), X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .remove_window_indicator(wid)
    }

    async fn paint_main_indicator(
        &mut self,
        output_name: &str,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .paint_main_indicator(output_name, &buf)
    }

    async fn paint_window_indicator(
        &mut self,
        wid: WindowId,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error> {
        self.inner
            .as_mut()
            .ok_or(X11Error::Other("not connected".into()))?
            .paint_window_indicator(wid, &buf)
    }

    fn take_event_rx(&mut self) -> Option<mpsc::Receiver<BackendEvent>> {
        self.rx.take()
    }
}

mod inner {
    //! Synchronous wrapper around an `x11rb::RustConnection` running on
    //! its own thread. The async trait above defers all the work into
    //! these blocking calls.
    //!
    //! NOTE: in the first integration milestone, only [`Worker::spawn`] and
    //! [`Worker::current_group`] / [`Worker::set_group`] / [`Worker::outputs`]
    //! are fully implemented. The indicator-window placement helpers are
    //! intentionally simple (create, move, destroy) and the event loop
    //! (XKB / RandR / property change subscriptions) lives in
    //! `super::tracker::EventLoop`.

    use std::{
        sync::{mpsc, Arc},
        thread,
    };

    use parking_lot::Mutex;
    use x11rb::{connection::Connection, rust_connection::RustConnection};
    use xxkb_core::{layout::Group, monitors::Output, registry::WindowId, Point};
    use xxkb_indicators::PixelBuffer;

    use super::X11Error;
    use crate::{
        events::BackendEvent, indicator_window::IndicatorWindowMgr, monitors::query_outputs,
        tracker::EventLoop, xkb::XkbConn,
    };

    pub struct Worker {
        conn: Arc<RustConnection>,
        screen_num: usize,
        windows: Arc<Mutex<IndicatorWindowMgr>>,
        // Worker thread handle (joined on drop).
        _thread: thread::JoinHandle<()>,
    }

    impl Worker {
        pub fn spawn() -> Result<(Self, mpsc::Receiver<BackendEvent>), X11Error> {
            let (conn, screen_num) = RustConnection::connect(None)?;
            let conn = Arc::new(conn);

            let xkb = XkbConn::init(&conn)?;
            let windows = Arc::new(Mutex::new(IndicatorWindowMgr::new()));

            let (tx, rx) = mpsc::channel();
            let event_conn = conn.clone();
            let event_windows = windows.clone();
            let thread = thread::Builder::new()
                .name("xxkb-x11-events".into())
                .spawn(move || {
                    let mut ev = EventLoop::new(event_conn, screen_num, xkb, event_windows, tx);
                    if let Err(e) = ev.run() {
                        tracing::error!(error = %e, "x11 event loop terminated");
                    }
                })
                .map_err(|e| X11Error::Other(format!("spawn thread: {e}")))?;

            Ok((
                Self {
                    conn,
                    screen_num,
                    windows,
                    _thread: thread,
                },
                rx,
            ))
        }

        pub fn current_group(&self) -> Result<Group, X11Error> {
            crate::xkb::current_group(&self.conn)
        }

        pub fn set_group(&self, g: Group) -> Result<(), X11Error> {
            crate::xkb::set_group(&self.conn, g)
        }

        pub fn outputs(&self) -> Result<Vec<Output>, X11Error> {
            let screen = &self.conn.setup().roots[self.screen_num];
            query_outputs(&*self.conn, screen.root)
        }

        pub fn place_main_indicator(
            &self,
            output_name: &str,
            point: Point,
            size: u32,
        ) -> Result<(), X11Error> {
            let screen = &self.conn.setup().roots[self.screen_num];
            self.windows
                .lock()
                .place_main(&*self.conn, screen, output_name, point, size)
        }

        pub fn place_window_indicator(
            &self,
            wid: WindowId,
            point: Point,
            size: u32,
        ) -> Result<(), X11Error> {
            let screen = &self.conn.setup().roots[self.screen_num];
            self.windows
                .lock()
                .place_for_window(&*self.conn, screen, wid, point, size)
        }

        pub fn remove_window_indicator(&self, wid: WindowId) -> Result<(), X11Error> {
            self.windows.lock().remove_for_window(&*self.conn, wid)
        }

        pub fn paint_main_indicator(
            &self,
            output_name: &str,
            buf: &PixelBuffer,
        ) -> Result<bool, X11Error> {
            let screen = &self.conn.setup().roots[self.screen_num];
            self.windows
                .lock()
                .paint_main(&*self.conn, screen, output_name, buf)
        }

        pub fn paint_window_indicator(
            &self,
            wid: WindowId,
            buf: &PixelBuffer,
        ) -> Result<bool, X11Error> {
            let screen = &self.conn.setup().roots[self.screen_num];
            self.windows
                .lock()
                .paint_for_window(&*self.conn, screen, wid, buf)
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            // The worker thread will exit when the connection is dropped
            // (poll_for_event returns None / errors). We rely on the
            // implicit drop ordering rather than an explicit shutdown
            // signal; this is fine for a process-lifetime worker.
            let _ = self.conn.flush();
        }
    }
}
