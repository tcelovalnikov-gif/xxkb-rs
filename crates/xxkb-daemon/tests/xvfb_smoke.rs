//! End-to-end smoke test that boots the real `xxkbd` binary against
//! the X server pointed to by `$DISPLAY` (typically Xvfb in CI).
//!
//! Goals:
//! * confirm the daemon connects, opens RandR/XKB, and creates at least
//!   one override-redirect indicator window of the configured size;
//! * confirm the rendering pipeline reaches the X server — i.e. the
//!   daemon survived past the `XPutImage` / `ChangeWindowAttributes`
//!   calls without an X error.
//!
//! The test self-skips when:
//! * `DISPLAY` is unset, or
//! * `XXKB_TEST_XVFB` is not `1`.
//!
//! That keeps `cargo test` on developer machines fast and harmless;
//! the dedicated CI job sets both env vars under `xvfb-run`.
//!
//! See `tests/xvfb/run_all.sh` for the runner that wires this up.

use std::{
    env,
    ffi::OsStr,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, Window},
    rust_connection::RustConnection,
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Default indicator size in `xxkb-config::MainIndicator`. Keep this
/// in sync with the config defaults.
const DEFAULT_SIZE_PX: u16 = 48;

fn skip_if_disabled() -> bool {
    if env::var_os("XXKB_TEST_XVFB").as_deref() != Some(OsStr::new("1")) {
        eprintln!("skipping xvfb_smoke: XXKB_TEST_XVFB != 1");
        return true;
    }
    if env::var_os("DISPLAY").is_none() {
        eprintln!("skipping xvfb_smoke: DISPLAY unset");
        return true;
    }
    false
}

/// Path to the daemon binary that cargo just built.
fn daemon_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xxkbd"))
}

/// Walk every window in the X tree (DFS) and call `f` on each.
/// Returns `Ok(true)` as soon as `f` returns `true`.
fn walk_tree<C: Connection, F: FnMut(Window) -> bool>(
    conn: &C,
    root: Window,
    f: &mut F,
) -> Result<bool, x11rb::errors::ReplyError> {
    let reply = conn.query_tree(root)?.reply()?;
    for &w in &reply.children {
        if f(w) {
            return Ok(true);
        }
        if walk_tree(conn, w, f)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Always-killed child guard so the daemon doesn't outlive the test.
struct DaemonGuard(Child);
impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn xvfb_smoke_creates_indicator_window() {
    if skip_if_disabled() {
        return;
    }

    let bin = daemon_binary();
    assert!(bin.exists(), "xxkbd binary not found at {}", bin.display());

    // Use a private XDG_CONFIG_HOME so we don't pick up the developer's
    // own config and so a saved-position write during the test doesn't
    // pollute their dotfiles.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_home = tmp.path().join("config");
    std::fs::create_dir_all(&cfg_home).unwrap();

    let child = Command::new(&bin)
        .env("XDG_CONFIG_HOME", &cfg_home)
        .env("RUST_LOG", "xxkb=info,xxkb_daemon=info,xxkb_x11=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn xxkbd");
    let mut guard = DaemonGuard(child);

    // Open our own X connection and poll until we see an override-redirect
    // child of root, of the configured indicator size.
    let (conn, screen_num) = RustConnection::connect(None).expect("x connect");
    let root = conn.setup().roots[screen_num].root;

    let started = Instant::now();
    let mut found: Option<Window> = None;
    while started.elapsed() < STARTUP_TIMEOUT {
        // Daemon may have died. Surface its output instead of timing out.
        if let Ok(Some(status)) = guard.0.try_wait() {
            dump_child_output(&mut guard.0);
            panic!("xxkbd exited prematurely with status {status:?}");
        }

        let mut hit: Option<Window> = None;
        let _ = walk_tree(&conn, root, &mut |w| {
            let attrs = match conn.get_window_attributes(w) {
                Ok(c) => c.reply().ok(),
                Err(_) => None,
            };
            if let Some(a) = attrs {
                if a.override_redirect {
                    let geom = match conn.get_geometry(w) {
                        Ok(c) => c.reply().ok(),
                        Err(_) => None,
                    };
                    if let Some(g) = geom {
                        if g.width == DEFAULT_SIZE_PX && g.height == DEFAULT_SIZE_PX {
                            hit = Some(w);
                            return true;
                        }
                    }
                }
            }
            false
        });
        if let Some(w) = hit {
            found = Some(w);
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }

    let Some(indicator) = found else {
        dump_child_output(&mut guard.0);
        panic!(
            "no {DEFAULT_SIZE_PX}x{DEFAULT_SIZE_PX} override-redirect indicator window \
             appeared within {STARTUP_TIMEOUT:?}"
        );
    };

    // Round-trip a clear_area on the indicator. If the daemon's render →
    // XPutImage → ChangeWindowAttributes sequence had failed, this would
    // either generate an X error or produce nothing visible. We accept
    // "no error" as the smoke confirmation.
    conn.clear_area(false, indicator, 0, 0, 0, 0)
        .expect("clear_area")
        .check()
        .expect("clear_area X error");
    eprintln!("xvfb_smoke: ok — indicator window {indicator:#x} of {DEFAULT_SIZE_PX}^2 alive");
}

fn dump_child_output(child: &mut Child) {
    let _ = child.kill();
    let stdout = child.stdout.take().map(read_to_string).unwrap_or_default();
    let stderr = child.stderr.take().map(read_to_string).unwrap_or_default();
    eprintln!("--- xxkbd stdout ---\n{stdout}");
    eprintln!("--- xxkbd stderr ---\n{stderr}");
}

fn read_to_string<R: std::io::Read>(mut r: R) -> String {
    let mut buf = String::new();
    let _ = r.read_to_string(&mut buf);
    buf
}
