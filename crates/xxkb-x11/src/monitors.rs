//! RandR queries — convert `randr::GetScreenResources` reply into the
//! backend-agnostic [`Output`] type from `xxkb-core`.

use x11rb::{
    connection::Connection,
    protocol::randr::{self, ConnectionExt as _, GetCrtcInfoReply, GetOutputInfoReply},
    protocol::xproto::{Atom, ConnectionExt as _, Window},
};
use xxkb_core::monitors::{Output, OutputName, Rect};

use crate::errors::X11Error;

/// Initialise RandR for `root` (subscribe to screen change notifications).
pub fn init_randr<C: Connection>(conn: &C, root: Window) -> Result<(), X11Error> {
    let reply = conn
        .randr_query_version(1, 5)
        .map_err(|e| X11Error::Other(format!("randr query_version: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("randr query_version reply: {e}")))?;
    if reply.major_version < 1 || (reply.major_version == 1 && reply.minor_version < 2) {
        return Err(X11Error::MissingExtension("RandR>=1.2"));
    }
    let mask = randr::NotifyMask::SCREEN_CHANGE
        | randr::NotifyMask::OUTPUT_CHANGE
        | randr::NotifyMask::CRTC_CHANGE;
    conn.randr_select_input(root, mask)
        .map_err(|e| X11Error::Other(format!("randr select_input: {e}")))?;
    conn.flush()
        .map_err(|e| X11Error::Other(format!("randr flush: {e}")))?;
    Ok(())
}

/// Snapshot of all current outputs.
pub fn query_outputs<C: Connection>(conn: &C, root: Window) -> Result<Vec<Output>, X11Error> {
    // Use `get_screen_resources_current` because it is non-blocking — it
    // returns the cached resources without re-probing displays. The
    // `RRScreenChangeNotify` event is what tells us when to call this.
    let res = conn
        .randr_get_screen_resources_current(root)
        .map_err(|e| X11Error::Other(format!("randr get_screen_resources: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("randr get_screen_resources reply: {e}")))?;

    let primary = conn
        .randr_get_output_primary(root)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.output)
        .unwrap_or(0);

    let mut out = Vec::with_capacity(res.outputs.len());
    for &output in &res.outputs {
        let info = match conn
            .randr_get_output_info(output, res.config_timestamp)
            .map_err(|e| X11Error::Other(format!("randr get_output_info: {e}")))?
            .reply()
        {
            Ok(i) => i,
            Err(_) => continue,
        };
        let name = String::from_utf8_lossy(&info.name).into_owned();
        // Treat an output as active when it has an attached CRTC,
        // regardless of the reported connection state. Headless servers
        // (Xvfb on CI, KVM consoles, some Xephyr setups) routinely
        // expose an active CRTC on an output marked DISCONNECTED or
        // UNKNOWN, and we still want to draw on those.
        let is_active = info.crtc != 0;

        let geometry = if is_active {
            match get_crtc_rect(conn, info.crtc, res.config_timestamp) {
                Ok(r) => r,
                Err(_) => Rect::new(0, 0, 0, 0),
            }
        } else {
            Rect::new(0, 0, 0, 0)
        };

        out.push(Output {
            name: name.into(),
            geometry,
            is_primary: output == primary,
            is_active,
        });
    }

    // Headless fallback: some Xvfb / Xephyr builds report no RandR
    // outputs at all, or report every output as inactive (no CRTC).
    // Synthesize one "screen" output covering the X screen so xxkbd
    // still has a place to draw the main indicator. This keeps the
    // daemon usable on those servers and is what the xvfb_smoke
    // integration test relies on in CI.
    if !out.iter().any(|o| o.is_active) {
        if let Some(synth) = synthesize_screen_output(conn, root) {
            out.push(synth);
        }
    }

    Ok(out)
}

/// Build a single fake `Output` covering the entire X screen rooted at
/// `root`, returning `None` if even `GetGeometry` fails.
fn synthesize_screen_output<C: Connection>(conn: &C, root: Window) -> Option<Output> {
    let reply = conn.get_geometry(root).ok()?.reply().ok()?;
    Some(Output {
        name: OutputName::from("screen".to_owned()),
        geometry: Rect::new(0, 0, u32::from(reply.width), u32::from(reply.height)),
        is_primary: true,
        is_active: true,
    })
}

fn get_crtc_rect<C: Connection>(
    conn: &C,
    crtc: u32,
    config_timestamp: u32,
) -> Result<Rect, X11Error> {
    let reply: GetCrtcInfoReply = conn
        .randr_get_crtc_info(crtc, config_timestamp)
        .map_err(|e| X11Error::Other(format!("randr get_crtc_info: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("randr get_crtc_info reply: {e}")))?;
    Ok(Rect::new(
        reply.x as i32,
        reply.y as i32,
        reply.width as u32,
        reply.height as u32,
    ))
}

// Suppress the unused-import warning for OutputInfo (kept for documentation).
#[allow(dead_code)]
fn _kept_for_docs(_x: &GetOutputInfoReply, _a: Atom) {}
