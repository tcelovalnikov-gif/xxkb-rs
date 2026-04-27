//! Indicator-window manager.
//!
//! We use **override-redirect** windows so the WM keeps its hands off:
//! * `_NET_WM_WINDOW_TYPE_DOCK` (skip taskbar/pager)
//! * always-on-top via `_NET_WM_STATE_ABOVE`
//! * input-only-ish: we receive button presses for drag, but we don't
//!   want focus
//!
//! This file owns the bookkeeping (which X window is the indicator for
//! which output / managed window) plus thin wrappers around `xproto`
//! calls. Rendering happens elsewhere (`xxkb-indicators`); we just
//! create / move / destroy the windows and accept the rendered pixmap
//! that's set as our background.

use std::collections::HashMap;

use x11rb::{
    connection::Connection,
    protocol::xproto::{
        ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
        ImageFormat, Screen, Window, WindowClass,
    },
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE,
};
use xxkb_core::{registry::WindowId, Point};
use xxkb_indicators::PixelBuffer;

use crate::{errors::X11Error, events::IndicatorTarget};

/// Tracks per-output and per-window indicator windows.
pub struct IndicatorWindowMgr {
    main_per_output: HashMap<String, Window>,
    per_window: HashMap<WindowId, Window>,
}

impl IndicatorWindowMgr {
    /// Build empty.
    #[must_use]
    pub fn new() -> Self {
        Self {
            main_per_output: HashMap::new(),
            per_window: HashMap::new(),
        }
    }

    /// Place (creating if necessary) the main indicator on `output_name`.
    pub fn place_main<C: Connection>(
        &mut self,
        conn: &C,
        screen: &Screen,
        output_name: &str,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        if let Some(&w) = self.main_per_output.get(output_name) {
            move_resize(conn, w, point, size)?;
        } else {
            let w = create_indicator_window(conn, screen, point, size)?;
            self.main_per_output.insert(output_name.to_owned(), w);
        }
        Ok(())
    }

    /// Place (creating if necessary) the indicator overlaid on a managed
    /// window.
    pub fn place_for_window<C: Connection>(
        &mut self,
        conn: &C,
        screen: &Screen,
        wid: WindowId,
        point: Point,
        size: u32,
    ) -> Result<(), X11Error> {
        if let Some(&w) = self.per_window.get(&wid) {
            move_resize(conn, w, point, size)?;
        } else {
            let w = create_indicator_window(conn, screen, point, size)?;
            self.per_window.insert(wid, w);
        }
        Ok(())
    }

    /// Destroy the per-window indicator (called on `WindowDestroyed`).
    pub fn remove_for_window<C: Connection>(
        &mut self,
        conn: &C,
        wid: WindowId,
    ) -> Result<(), X11Error> {
        if let Some(w) = self.per_window.remove(&wid) {
            conn.destroy_window(w)
                .map_err(|e| X11Error::Other(format!("destroy_window: {e}")))?;
            conn.flush()
                .map_err(|e| X11Error::Other(format!("flush: {e}")))?;
        }
        Ok(())
    }

    /// Paint `buf` onto the existing main indicator for `output_name`.
    /// Returns `Ok(false)` if no such indicator exists yet.
    pub fn paint_main<C: Connection>(
        &self,
        conn: &C,
        screen: &Screen,
        output_name: &str,
        buf: &PixelBuffer,
    ) -> Result<bool, X11Error> {
        if let Some(&w) = self.main_per_output.get(output_name) {
            paint_pixbuf(conn, screen, w, buf)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Paint `buf` onto the existing per-window indicator for `wid`.
    pub fn paint_for_window<C: Connection>(
        &self,
        conn: &C,
        screen: &Screen,
        wid: WindowId,
        buf: &PixelBuffer,
    ) -> Result<bool, X11Error> {
        if let Some(&w) = self.per_window.get(&wid) {
            paint_pixbuf(conn, screen, w, buf)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Iterate (output_name, window) for main indicators.
    pub fn main_iter(&self) -> impl Iterator<Item = (&str, Window)> {
        self.main_per_output.iter().map(|(k, &v)| (k.as_str(), v))
    }

    /// Iterate (managed window, indicator window) for per-window indicators.
    pub fn window_iter(&self) -> impl Iterator<Item = (WindowId, Window)> + '_ {
        self.per_window.iter().map(|(&k, &v)| (k, v))
    }

    /// Map an X window id (any of the indicator windows we created) to
    /// the higher-level [`IndicatorTarget`] it represents.
    ///
    /// Returns `None` if the window is not one of ours.
    #[must_use]
    pub fn lookup_target(&self, w: Window) -> Option<IndicatorTarget> {
        for (name, &iw) in &self.main_per_output {
            if iw == w {
                return Some(IndicatorTarget::Main(name.clone()));
            }
        }
        for (&wid, &iw) in &self.per_window {
            if iw == w {
                return Some(IndicatorTarget::Window(wid));
            }
        }
        None
    }

    /// Move the indicator window directly to `point` without touching the
    /// indicator-target mapping. Used during interactive drag.
    pub fn move_window<C: Connection>(
        &self,
        conn: &C,
        ind_win: Window,
        point: Point,
    ) -> Result<(), X11Error> {
        let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
            .x(i32::from(clamp_i16(point.x)))
            .y(i32::from(clamp_i16(point.y)));
        conn.configure_window(ind_win, &aux)
            .map_err(|e| X11Error::Other(format!("configure_window: {e}")))?;
        conn.flush()
            .map_err(|e| X11Error::Other(format!("flush: {e}")))?;
        Ok(())
    }
}

impl Default for IndicatorWindowMgr {
    fn default() -> Self {
        Self::new()
    }
}

fn create_indicator_window<C: Connection>(
    conn: &C,
    screen: &Screen,
    point: Point,
    size: u32,
) -> Result<Window, X11Error> {
    let wid = conn
        .generate_id()
        .map_err(|e| X11Error::Other(format!("generate_id: {e}")))?;
    let aux = CreateWindowAux::new()
        .override_redirect(1)
        .background_pixel(screen.white_pixel)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::BUTTON1_MOTION
                | EventMask::ENTER_WINDOW
                | EventMask::LEAVE_WINDOW,
        );
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        wid,
        screen.root,
        clamp_i16(point.x),
        clamp_i16(point.y),
        clamp_u16(size),
        clamp_u16(size),
        0, // border width
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &aux,
    )
    .map_err(|e| X11Error::Other(format!("create_window: {e}")))?;
    conn.map_window(wid)
        .map_err(|e| X11Error::Other(format!("map_window: {e}")))?;
    conn.flush()
        .map_err(|e| X11Error::Other(format!("flush: {e}")))?;
    let _ = (CURRENT_TIME, NONE); // silence unused-imports lint
    Ok(wid)
}

fn move_resize<C: Connection>(
    conn: &C,
    wid: Window,
    point: Point,
    size: u32,
) -> Result<(), X11Error> {
    let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
        .x(i32::from(clamp_i16(point.x)))
        .y(i32::from(clamp_i16(point.y)))
        .width(u32::from(clamp_u16(size)))
        .height(u32::from(clamp_u16(size)));
    conn.configure_window(wid, &aux)
        .map_err(|e| X11Error::Other(format!("configure_window: {e}")))?;
    conn.flush()
        .map_err(|e| X11Error::Other(format!("flush: {e}")))?;
    let _ = ChangeWindowAttributesAux::new();
    Ok(())
}

fn clamp_i16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn clamp_u16(v: u32) -> u16 {
    v.clamp(1, u32::from(u16::MAX)) as u16
}

/// Upload a `PixelBuffer` to the X server, set it as the window's
/// `background_pixmap`, and force a redraw with `clear_area`.
///
/// We currently use `screen.root_depth` (typically 24) which matches
/// the visual the indicator window itself was created on. The alpha
/// byte in our BGRA layout is therefore ignored by the server — that's
/// fine for opaque flag rendering. True ARGB indicators (depth 32)
/// require a separate compositor-friendly visual; that's a follow-up.
pub fn paint_pixbuf<C: Connection>(
    conn: &C,
    screen: &Screen,
    window: Window,
    buf: &PixelBuffer,
) -> Result<(), X11Error> {
    let w = clamp_u16(buf.width);
    let h = clamp_u16(buf.height);
    let depth = screen.root_depth;

    let pixmap = conn
        .generate_id()
        .map_err(|e| X11Error::Other(format!("generate_id pixmap: {e}")))?;
    conn.create_pixmap(depth, pixmap, screen.root, w, h)
        .map_err(|e| X11Error::Other(format!("create_pixmap: {e}")))?;

    let gc = conn
        .generate_id()
        .map_err(|e| X11Error::Other(format!("generate_id gc: {e}")))?;
    conn.create_gc(gc, pixmap, &CreateGCAux::new())
        .map_err(|e| X11Error::Other(format!("create_gc: {e}")))?;

    conn.put_image(
        ImageFormat::Z_PIXMAP,
        pixmap,
        gc,
        w,
        h,
        0,
        0,
        0,
        depth,
        &buf.data,
    )
    .map_err(|e| X11Error::Other(format!("put_image: {e}")))?;

    let aux = ChangeWindowAttributesAux::new().background_pixmap(pixmap);
    conn.change_window_attributes(window, &aux)
        .map_err(|e| X11Error::Other(format!("change_window_attributes: {e}")))?;
    conn.clear_area(false, window, 0, 0, w, h)
        .map_err(|e| X11Error::Other(format!("clear_area: {e}")))?;

    conn.free_gc(gc)
        .map_err(|e| X11Error::Other(format!("free_gc: {e}")))?;
    // The server keeps the pixmap alive as long as it's a window
    // background, so it's safe to free our handle now.
    conn.free_pixmap(pixmap)
        .map_err(|e| X11Error::Other(format!("free_pixmap: {e}")))?;
    conn.flush()
        .map_err(|e| X11Error::Other(format!("flush: {e}")))?;
    Ok(())
}
