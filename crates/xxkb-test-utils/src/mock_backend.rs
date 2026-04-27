//! Mock implementation of [`xxkb_x11::Backend`].

use std::sync::{
    mpsc::{self, Receiver, Sender},
    Arc,
};

use async_trait::async_trait;
use parking_lot::Mutex;
use xxkb_core::{layout::Group, monitors::Output, registry::WindowId, Point};
use xxkb_indicators::PixelBuffer;
use xxkb_x11::{backend::Backend, errors::X11Error, events::BackendEvent};

/// Mock backend; lets tests inject events and inspect calls.
pub struct MockBackend {
    /// Recorded calls. Inspect with [`Self::calls`].
    calls: Arc<Mutex<Vec<MockCall>>>,
    current_group: Arc<Mutex<Group>>,
    outputs: Arc<Mutex<Vec<Output>>>,
    rx: Option<Receiver<BackendEvent>>,
    /// Public sender so tests can `send(BackendEvent)` to drive the daemon.
    pub event_tx: Sender<BackendEvent>,
}

/// Recorded backend call.
///
/// We capture only the call _shape_ (e.g. which output was painted at
/// which size), not the full pixel data — that would make assertions
/// awful and the buffer is opaque to the daemon anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockCall {
    /// `connect` was called.
    Connect,
    /// `set_group(group)`.
    SetGroup(Group),
    /// `place_main_indicator(name, point, size)`.
    PlaceMain(String, Point, u32),
    /// `place_window_indicator(wid, point, size)`.
    PlaceForWindow(WindowId, Point, u32),
    /// `remove_window_indicator(wid)`.
    RemoveForWindow(WindowId),
    /// `paint_main_indicator(name, width, height)`.
    PaintMain(String, u32, u32),
    /// `paint_window_indicator(wid, width, height)`.
    PaintForWindow(WindowId, u32, u32),
}

/// Builder for [`MockBackend`].
pub struct MockBackendBuilder {
    initial_group: Group,
    outputs: Vec<Output>,
}

impl MockBackendBuilder {
    /// Build with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            initial_group: Group::new(0, 4).unwrap(),
            outputs: Vec::new(),
        }
    }

    /// Set the initial group.
    #[must_use]
    pub fn with_group(mut self, g: Group) -> Self {
        self.initial_group = g;
        self
    }

    /// Set the initial outputs.
    #[must_use]
    pub fn with_outputs(mut self, outputs: Vec<Output>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Finish.
    #[must_use]
    pub fn build(self) -> MockBackend {
        let (tx, rx) = mpsc::channel();
        MockBackend {
            calls: Arc::new(Mutex::new(Vec::new())),
            current_group: Arc::new(Mutex::new(self.initial_group)),
            outputs: Arc::new(Mutex::new(self.outputs)),
            rx: Some(rx),
            event_tx: tx,
        }
    }
}

impl Default for MockBackendBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    /// All recorded calls.
    #[must_use]
    pub fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().clone()
    }

    /// Replace the outputs (e.g. to simulate monitor unplug).
    pub fn set_outputs(&self, outputs: Vec<Output>) {
        *self.outputs.lock() = outputs;
    }

    /// Inject an event into the daemon's main loop.
    pub fn inject(&self, ev: BackendEvent) {
        let _ = self.event_tx.send(ev);
    }
}

#[async_trait]
impl Backend for MockBackend {
    async fn connect(&mut self) -> Result<(), X11Error> {
        self.calls.lock().push(MockCall::Connect);
        Ok(())
    }

    async fn set_group(&mut self, group: Group) -> Result<(), X11Error> {
        self.calls.lock().push(MockCall::SetGroup(group));
        *self.current_group.lock() = group;
        Ok(())
    }

    async fn current_group(&mut self) -> Result<Group, X11Error> {
        Ok(*self.current_group.lock())
    }

    async fn outputs(&mut self) -> Result<Vec<Output>, X11Error> {
        Ok(self.outputs.lock().clone())
    }

    async fn place_main_indicator(
        &mut self,
        output_name: &str,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        self.calls
            .lock()
            .push(MockCall::PlaceMain(output_name.to_owned(), point, size));
        Ok(())
    }

    async fn place_window_indicator(
        &mut self,
        wid: WindowId,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        self.calls
            .lock()
            .push(MockCall::PlaceForWindow(wid, point, size));
        Ok(())
    }

    async fn remove_window_indicator(&mut self, wid: WindowId) -> Result<(), X11Error> {
        self.calls.lock().push(MockCall::RemoveForWindow(wid));
        Ok(())
    }

    async fn paint_main_indicator(
        &mut self,
        output_name: &str,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error> {
        self.calls.lock().push(MockCall::PaintMain(
            output_name.to_owned(),
            buf.width,
            buf.height,
        ));
        Ok(true)
    }

    async fn paint_window_indicator(
        &mut self,
        wid: WindowId,
        buf: Arc<PixelBuffer>,
    ) -> Result<bool, X11Error> {
        self.calls
            .lock()
            .push(MockCall::PaintForWindow(wid, buf.width, buf.height));
        Ok(true)
    }

    fn take_event_rx(&mut self) -> Option<mpsc::Receiver<BackendEvent>> {
        self.rx.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tiny sync executor so we don't pull tokio into test-utils.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::{
            sync::Arc,
            task::{Context, Poll, Wake, Waker},
        };
        struct N;
        impl Wake for N {
            fn wake(self: Arc<Self>) {}
        }
        let waker: Waker = Arc::new(N).into();
        let mut cx = Context::from_waker(&waker);
        let mut pin = Box::pin(f);
        loop {
            if let Poll::Ready(v) = pin.as_mut().poll(&mut cx) {
                return v;
            }
            // Mock futures complete immediately; we never need to park.
            std::thread::yield_now();
        }
    }

    #[test]
    fn records_calls_in_order() {
        block_on(async {
            let mut be = MockBackendBuilder::new().build();
            be.connect().await.unwrap();
            be.set_group(Group::new(2, 4).unwrap()).await.unwrap();
            assert_eq!(
                be.calls(),
                vec![
                    MockCall::Connect,
                    MockCall::SetGroup(Group::new(2, 4).unwrap())
                ]
            );
        });
    }

    #[test]
    fn records_paint_dimensions_only() {
        block_on(async {
            let mut be = MockBackendBuilder::new().build();
            let buf = Arc::new(PixelBuffer::solid(32, 32, [255, 0, 0, 255]));
            let wid = WindowId(0xdead_beef);
            assert!(be.paint_main_indicator("DP-1", buf.clone()).await.unwrap());
            assert!(be.paint_window_indicator(wid, buf).await.unwrap());
            assert_eq!(
                be.calls(),
                vec![
                    MockCall::PaintMain("DP-1".to_owned(), 32, 32),
                    MockCall::PaintForWindow(wid, 32, 32),
                ]
            );
        });
    }
}
