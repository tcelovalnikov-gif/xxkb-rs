//! X11 event loop: subscribe to XKB state changes, RandR screen changes,
//! property changes on the root window (for `_NET_ACTIVE_WINDOW`) and on
//! managed windows (for `WM_NAME`, `WM_CLASS`, `_NET_FRAME_EXTENTS`).

use std::sync::{mpsc, Arc};

use indexmap::IndexMap;
use parking_lot::Mutex;
use x11rb::{
    connection::Connection,
    protocol::{
        randr,
        xproto::{
            Atom, ButtonPressEvent, ButtonReleaseEvent, ConnectionExt as _, EventMask, GrabMode,
            KeyButMask, MotionNotifyEvent, PropMode, Window,
        },
        Event,
    },
    rust_connection::RustConnection,
    CURRENT_TIME, NONE,
};
use xxkb_core::{
    layout::SwitchKind, placement::FrameExtents, registry::WindowId, rules::WindowProps, Point,
};

use crate::{
    errors::X11Error,
    events::{BackendEvent, IndicatorTarget, MouseButton, WindowGeom},
    indicator_window::IndicatorWindowMgr,
    monitors::query_outputs,
    xkb::XkbConn,
};

/// Bookkeeping for an in-progress Ctrl+Button1 drag of one of our
/// indicator windows.
#[derive(Debug, Clone)]
struct DragSession {
    target: IndicatorTarget,
    indicator_window: Window,
    /// Cursor offset *inside* the indicator window at press time.
    offset_in_window: (i16, i16),
    /// Latest root-coords origin of the window as we've been moving it.
    last_origin: Point,
    /// `true` once any motion has been observed — used to avoid emitting
    /// `IndicatorDragged` for a Ctrl-press with no movement.
    moved: bool,
}

/// Owns the read-side of the X connection on a worker thread.
pub struct EventLoop {
    conn: Arc<RustConnection>,
    screen_num: usize,
    _xkb: XkbConn,
    windows: Arc<Mutex<IndicatorWindowMgr>>,
    tx: mpsc::Sender<BackendEvent>,
    atoms: Atoms,
    /// Cache of WindowProps for tracked windows.
    tracked_props: IndexMap<Window, WindowProps>,
    /// In-progress drag, if any.
    drag: Option<DragSession>,
}

impl EventLoop {
    /// Build.
    pub fn new(
        conn: Arc<RustConnection>,
        screen_num: usize,
        xkb: XkbConn,
        windows: Arc<Mutex<IndicatorWindowMgr>>,
        tx: mpsc::Sender<BackendEvent>,
    ) -> Self {
        let atoms = Atoms::intern(&*conn).unwrap_or_else(|_| Atoms::default());
        Self {
            conn,
            screen_num,
            _xkb: xkb,
            windows,
            tx,
            atoms,
            tracked_props: IndexMap::new(),
            drag: None,
        }
    }

    /// Run forever (until the connection is closed).
    pub fn run(&mut self) -> Result<(), X11Error> {
        let root = {
            let setup = self.conn.setup();
            setup.roots[self.screen_num].root
        };

        // Subscribe to PropertyNotify on the root window for active-window changes.
        let aux = x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::PROPERTY_CHANGE | EventMask::SUBSTRUCTURE_NOTIFY);
        self.conn
            .change_window_attributes(root, &aux)
            .map_err(|e| X11Error::Other(format!("change_window_attributes root: {e}")))?;

        // Subscribe to RandR screen change events.
        crate::monitors::init_randr(&*self.conn, root)?;
        self.conn
            .flush()
            .map_err(|e| X11Error::Other(format!("flush: {e}")))?;

        loop {
            let event = match self.conn.wait_for_event() {
                Ok(e) => e,
                Err(e) => {
                    return Err(X11Error::Other(format!("wait_for_event: {e}")));
                }
            };
            self.handle_event(event, root)?;
        }
    }

    fn handle_event(&mut self, event: Event, root: Window) -> Result<(), X11Error> {
        match event {
            Event::PropertyNotify(ev) => {
                if ev.window == root && ev.atom == self.atoms.net_active_window {
                    self.handle_active_window_change(root)?;
                } else if self.tracked_props.contains_key(&ev.window)
                    && ev.atom == self.atoms.net_frame_extents
                {
                    self.emit_geometry(ev.window);
                }
                Ok(())
            }
            Event::ConfigureNotify(ev) => {
                // Ignore SubstructureNotify on root; only react to
                // moves/resizes of windows we're already tracking.
                if self.tracked_props.contains_key(&ev.window) {
                    self.emit_geometry(ev.window);
                }
                Ok(())
            }
            Event::ButtonPress(ev) => self.handle_button_press(ev),
            Event::MotionNotify(ev) => self.handle_motion(ev),
            Event::ButtonRelease(ev) => self.handle_button_release(ev),
            Event::XkbStateNotify(ev) => {
                let new_group: u8 = ev.group.into();
                let kind = if ev.keycode != 0 {
                    SwitchKind::Keyboard
                } else {
                    SwitchKind::Auto
                };
                let _ = self
                    .tx
                    .send(BackendEvent::LayoutChanged { new_group, kind });
                Ok(())
            }
            Event::DestroyNotify(ev) => {
                self.tracked_props.shift_remove(&ev.window);
                let _ = self
                    .windows
                    .lock()
                    .remove_for_window(&*self.conn, WindowId(ev.window));
                let _ = self.tx.send(BackendEvent::WindowDestroyed {
                    wid: WindowId(ev.window),
                });
                Ok(())
            }
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => {
                let outputs = query_outputs(&*self.conn, root)?;
                let _ = self.tx.send(BackendEvent::MonitorsChanged { outputs });
                Ok(())
            }
            Event::Error(e) => {
                tracing::warn!(?e, "x server error");
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn emit_geometry(&self, w: Window) {
        match read_window_geom(
            &*self.conn,
            self.root_for_screen(),
            w,
            self.atoms.net_frame_extents,
        ) {
            Ok(geom) => {
                let _ = self.tx.send(BackendEvent::WindowGeometryChanged {
                    wid: WindowId(w),
                    geom,
                });
            }
            Err(e) => {
                tracing::trace!(window = w, error = %e, "geometry probe failed");
            }
        }
    }

    fn root_for_screen(&self) -> Window {
        self.conn.setup().roots[self.screen_num].root
    }

    fn handle_button_press(&mut self, ev: ButtonPressEvent) -> Result<(), X11Error> {
        let target = match self.windows.lock().lookup_target(ev.event) {
            Some(t) => t,
            // Not one of our windows; ignore.
            None => return Ok(()),
        };
        let ctrl = state_has(ev.state, KeyButMask::CONTROL);
        let shift = state_has(ev.state, KeyButMask::SHIFT);
        let button = mouse_button_from_detail(ev.detail);

        if ctrl && ev.detail == 1 {
            // Begin drag. Grab the pointer so we still get motion events
            // even if the cursor leaves the small indicator window.
            let _ = self.conn.grab_pointer(
                /* owner_events */ true,
                ev.event,
                EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                NONE,
                ev.time,
            );
            let _ = self.conn.flush();
            let origin_x = i32::from(ev.root_x) - i32::from(ev.event_x);
            let origin_y = i32::from(ev.root_y) - i32::from(ev.event_y);
            self.drag = Some(DragSession {
                target,
                indicator_window: ev.event,
                offset_in_window: (ev.event_x, ev.event_y),
                last_origin: Point::new(origin_x, origin_y),
                moved: false,
            });
            return Ok(());
        }

        let _ = self.tx.send(BackendEvent::IndicatorClicked {
            target,
            button,
            ctrl,
            shift,
        });
        Ok(())
    }

    fn handle_motion(&mut self, ev: MotionNotifyEvent) -> Result<(), X11Error> {
        let drag = match self.drag.as_mut() {
            Some(d) => d,
            None => return Ok(()),
        };
        let new_x = i32::from(ev.root_x) - i32::from(drag.offset_in_window.0);
        let new_y = i32::from(ev.root_y) - i32::from(drag.offset_in_window.1);
        let new_origin = Point::new(new_x, new_y);
        if new_origin != drag.last_origin {
            drag.moved = true;
            drag.last_origin = new_origin;
            let _ = self
                .windows
                .lock()
                .move_window(&*self.conn, drag.indicator_window, new_origin);
        }
        Ok(())
    }

    fn handle_button_release(&mut self, ev: ButtonReleaseEvent) -> Result<(), X11Error> {
        let drag = match self.drag.take() {
            Some(d) => d,
            None => return Ok(()),
        };
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.flush();
        let _ = ev; // event coordinates aren't needed once `last_origin` is current

        if drag.moved {
            let _ = self.tx.send(BackendEvent::IndicatorDragged {
                target: drag.target,
                new_origin: drag.last_origin,
            });
        }
        Ok(())
    }

    fn handle_active_window_change(&mut self, root: Window) -> Result<(), X11Error> {
        let active = self.read_window_atom(root, self.atoms.net_active_window)?;
        if active == 0 {
            let _ = self.tx.send(BackendEvent::ActiveWindowChanged {
                wid: None,
                props: None,
                geom: None,
            });
            return Ok(());
        }
        let props = self.read_window_props(active).unwrap_or_default();
        // Make sure we get notified about future title changes, frame
        // extents updates, and configure events on this window.
        let aux = x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY);
        let _ = self.conn.change_window_attributes(active, &aux);
        let _ = self.conn.flush();

        let geom = read_window_geom(&*self.conn, root, active, self.atoms.net_frame_extents).ok();

        self.tracked_props.insert(active, props.clone());
        let _ = self.tx.send(BackendEvent::ActiveWindowChanged {
            wid: Some(WindowId(active)),
            props: Some(props),
            geom,
        });
        Ok(())
    }

    fn read_window_atom(&self, w: Window, atom: Atom) -> Result<Window, X11Error> {
        let reply = self
            .conn
            .get_property(
                false,
                w,
                atom,
                x11rb::protocol::xproto::AtomEnum::WINDOW,
                0,
                1,
            )
            .map_err(|e| X11Error::Other(format!("get_property: {e}")))?
            .reply()
            .map_err(|e| X11Error::Other(format!("get_property reply: {e}")))?;
        if reply.value_len == 0 {
            return Ok(0);
        }
        let mut bytes = reply.value;
        if bytes.len() < 4 {
            return Ok(0);
        }
        let arr: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        bytes.clear();
        Ok(u32::from_ne_bytes(arr))
    }

    fn read_window_props(&self, w: Window) -> Result<WindowProps, X11Error> {
        let class = self.read_string_property(w, self.atoms.wm_class)?;
        let (cls_class, cls_name) = split_wm_class(&class);
        let name = self.read_string_property(w, self.atoms.wm_name)?;
        let net_wm_name = self.read_string_property(w, self.atoms.net_wm_name)?;
        let final_name = if net_wm_name.is_empty() {
            name
        } else {
            net_wm_name
        };
        Ok(WindowProps {
            wm_class_class: cls_class,
            wm_class_name: cls_name,
            wm_name: final_name,
        })
    }

    fn read_string_property(&self, w: Window, atom: Atom) -> Result<String, X11Error> {
        let reply = self
            .conn
            .get_property(
                false,
                w,
                atom,
                x11rb::protocol::xproto::AtomEnum::ANY,
                0,
                1024,
            )
            .map_err(|e| X11Error::Other(format!("get_property: {e}")))?
            .reply()
            .map_err(|e| X11Error::Other(format!("get_property reply: {e}")))?;
        Ok(String::from_utf8_lossy(&reply.value).into_owned())
    }
}

fn state_has(state: KeyButMask, bit: KeyButMask) -> bool {
    let s: u16 = state.into();
    let b: u16 = bit.into();
    s & b != 0
}

fn mouse_button_from_detail(detail: u8) -> MouseButton {
    match detail {
        1 => MouseButton::Left,
        2 => MouseButton::Middle,
        _ => MouseButton::Right,
    }
}

/// Query a window's root-coords geometry and `_NET_FRAME_EXTENTS`.
///
/// Returns the *client* origin (top-left of the inner area, in root coords),
/// width, height, and frame extents. If the window doesn't have
/// `_NET_FRAME_EXTENTS` set, frame defaults to all zeros.
fn read_window_geom<C: Connection>(
    conn: &C,
    root: Window,
    w: Window,
    net_frame_extents: Atom,
) -> Result<WindowGeom, X11Error> {
    let geom = conn
        .get_geometry(w)
        .map_err(|e| X11Error::Other(format!("get_geometry: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("get_geometry reply: {e}")))?;

    let translated = conn
        .translate_coordinates(w, root, 0, 0)
        .map_err(|e| X11Error::Other(format!("translate_coordinates: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("translate_coordinates reply: {e}")))?;

    let frame = read_frame_extents(conn, w, net_frame_extents).unwrap_or_default();

    Ok(WindowGeom {
        origin: Point::new(i32::from(translated.dst_x), i32::from(translated.dst_y)),
        width: u32::from(geom.width),
        height: u32::from(geom.height),
        frame,
    })
}

/// Read the WM-set `_NET_FRAME_EXTENTS` (CARDINAL[4] = left/right/top/bottom).
fn read_frame_extents<C: Connection>(
    conn: &C,
    w: Window,
    atom: Atom,
) -> Result<FrameExtents, X11Error> {
    let reply = conn
        .get_property(
            false,
            w,
            atom,
            x11rb::protocol::xproto::AtomEnum::CARDINAL,
            0,
            4,
        )
        .map_err(|e| X11Error::Other(format!("get_property frame_extents: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("get_property frame_extents reply: {e}")))?;
    if reply.format != 32 || reply.value_len < 4 {
        return Ok(FrameExtents::default());
    }
    let raw = reply.value;
    if raw.len() < 16 {
        return Ok(FrameExtents::default());
    }
    let read_u32 =
        |i: usize| u32::from_ne_bytes([raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2], raw[i * 4 + 3]]);
    Ok(FrameExtents {
        left: read_u32(0),
        right: read_u32(1),
        top: read_u32(2),
        bottom: read_u32(3),
    })
}

fn split_wm_class(raw: &str) -> (String, String) {
    // WM_CLASS is two NUL-terminated strings: instance, then class.
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let instance = parts.next().unwrap_or("").to_owned();
    let class = parts.next().unwrap_or("").to_owned();
    (class, instance)
}

#[allow(dead_code)]
#[derive(Default)]
struct Atoms {
    net_active_window: Atom,
    net_wm_name: Atom,
    net_frame_extents: Atom,
    wm_name: Atom,
    wm_class: Atom,
    utf8_string: Atom,
}

impl Atoms {
    fn intern<C: Connection>(conn: &C) -> Result<Self, X11Error> {
        fn intern<C: Connection>(conn: &C, name: &str) -> Result<Atom, X11Error> {
            Ok(conn
                .intern_atom(false, name.as_bytes())
                .map_err(|e| X11Error::Other(format!("intern_atom: {e}")))?
                .reply()
                .map_err(|e| X11Error::Other(format!("intern_atom reply: {e}")))?
                .atom)
        }
        Ok(Self {
            net_active_window: intern(conn, "_NET_ACTIVE_WINDOW")?,
            net_wm_name: intern(conn, "_NET_WM_NAME")?,
            net_frame_extents: intern(conn, "_NET_FRAME_EXTENTS")?,
            wm_name: intern(conn, "WM_NAME")?,
            wm_class: intern(conn, "WM_CLASS")?,
            utf8_string: intern(conn, "UTF8_STRING")?,
        })
    }
}

// Help compilers see we use these vendored items even if some paths
// branch around them.
#[allow(dead_code)]
fn _unused(_p: PropMode) {
    let _ = SwitchKind::Initial;
    let _ = randr::NotifyMask::SCREEN_CHANGE;
}
