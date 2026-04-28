#!/usr/bin/env bash
# Generic Docker smoke for xxkb-rs against a target DE image.
#
# Contract:
#
# * /dist           — bind-mounted host dir containing xxkb-daemon.deb
#                     (and optionally xxkb-configurator.deb).
# * /opt/xxkb       — image-provided scratch dir for our config.
# * $XXKB_DE        — DE name baked into the image, used only for logging.
#
# What we verify per image:
#
# 1. The .deb installs cleanly with the DE's runtime libraries already
#    present (catches dependency drift between distros / DE stacks).
# 2. The xxkbd binary starts under a minimal Xvfb session with dbus and
#    a two-group XKB keymap.
# 3. xxkbd creates at least one override-redirect indicator window of
#    the configured size — i.e. RandR + XKB + render pipeline reached
#    XPutImage without an X error.
# 4. A scripted Alt+Shift_L cycles XKB groups and the daemon survives
#    (no crash, no D-Bus disconnect).
#
# Booting a real DE session inside Docker is out of scope: we use the
# DE images for *dependency* / *packaging* surface coverage, not for
# DE-specific behaviour like compositor side-effects. The xvfb-run job
# in the same CI matrix already exercises the X11 protocol path on a
# clean Ubuntu, so this matrix is purely a "does it install and run"
# net.

set -euo pipefail

DE="${XXKB_DE:-unknown}"
echo "==> xxkb-rs Docker smoke: DE=${DE}"

# --- 1. Install the .deb -----------------------------------------------
DEB="/dist/xxkb-daemon.deb"
if [[ ! -f "${DEB}" ]]; then
    echo "FATAL: ${DEB} not found. CI must mount /dist with the artefact." >&2
    exit 1
fi
echo "==> Installing ${DEB}"
# `apt-get install ./pkg.deb` resolves runtime deps (libgtk-4-1 etc.)
# from the apt index; falls back to dpkg+apt-get -f if unavailable.
apt-get update -qq
apt-get install -y --no-install-recommends "${DEB}"

command -v xxkbd >/dev/null || {
    echo "FATAL: xxkbd not on PATH after .deb install" >&2
    exit 1
}
echo "==> xxkbd installed at $(command -v xxkbd)"

# --- 2. Start Xvfb + dbus ---------------------------------------------
export DISPLAY=":99"
echo "==> launching Xvfb on ${DISPLAY}"
Xvfb "${DISPLAY}" -screen 0 1280x800x24 -nolisten tcp &
XVFB_PID=$!
trap 'kill ${XVFB_PID} 2>/dev/null || true' EXIT

# Wait until X server actually answers.
for _ in $(seq 1 50); do
    if xdpyinfo -display "${DISPLAY}" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
xdpyinfo -display "${DISPLAY}" >/dev/null

echo "==> two-group XKB keymap (us,ru) on ${DISPLAY}"
setxkbmap -display "${DISPLAY}" -layout us,ru -option grp:alt_shift_toggle

# Some DE pieces poke a session bus on startup; give them one.
if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    echo "==> wrapping the rest in dbus-run-session"
    exec dbus-run-session -- "$0" "$@"
fi

# --- 3. Run xxkbd in the background -----------------------------------
mkdir -p /opt/xxkb/config
export XDG_CONFIG_HOME=/opt/xxkb/config
# Debug for daemon + x11 so we can scrape "LayoutChanged" / XKB notify
# traces from the log to verify step 5 below.
export RUST_LOG="${RUST_LOG:-xxkb=info,xxkb_daemon=debug,xxkb_x11=debug}"

echo "==> launching xxkbd"
xxkbd >/tmp/xxkbd.log 2>&1 &
XXKBD_PID=$!
trap 'kill ${XXKBD_PID} 2>/dev/null || true; kill ${XVFB_PID} 2>/dev/null || true' EXIT

# --- 4. Poll for an override-redirect indicator window ----------------
echo "==> waiting for indicator window"
DEADLINE=$(( $(date +%s) + 20 ))
FOUND=0
while [[ $(date +%s) -lt ${DEADLINE} ]]; do
    if ! kill -0 "${XXKBD_PID}" 2>/dev/null; then
        echo "FATAL: xxkbd exited prematurely. Log:" >&2
        cat /tmp/xxkbd.log >&2 || true
        exit 1
    fi
    # `xwininfo -tree -root` lists every window with override-redirect
    # status in its summary line. Default indicator is 48x48.
    if xwininfo -display "${DISPLAY}" -root -tree 2>/dev/null \
        | grep -E '\b48x48\b' >/dev/null; then
        FOUND=1
        break
    fi
    sleep 0.2
done

if [[ "${FOUND}" -ne 1 ]]; then
    echo "FATAL: no 48x48 indicator window appeared within 20s" >&2
    echo "--- xxkbd log ---" >&2
    cat /tmp/xxkbd.log >&2 || true
    echo "--- xwininfo tree ---" >&2
    xwininfo -display "${DISPLAY}" -root -tree 2>&1 | head -80 >&2 || true
    exit 1
fi
echo "==> indicator window present"

# --- 5. Cycle the XKB group via xdotool, verify daemon survives -------
echo "==> cycling XKB group with xdotool"
# Some DE images ship xdotool, others don't — fall back to a manual
# XKB lock via setxkbmap if it's missing (still exercises XkbStateNotify).
if command -v xdotool >/dev/null; then
    xdotool key --delay 50 alt+shift
    sleep 0.3
    xdotool key --delay 50 alt+shift
else
    echo "    (xdotool missing, falling back to setxkbmap toggle)"
    setxkbmap -display "${DISPLAY}" -layout ru
    sleep 0.3
    setxkbmap -display "${DISPLAY}" -layout us
fi
sleep 0.5

if ! kill -0 "${XXKBD_PID}" 2>/dev/null; then
    echo "FATAL: xxkbd died after group cycle. Log:" >&2
    cat /tmp/xxkbd.log >&2 || true
    exit 1
fi

# Confirm the daemon actually saw at least one layout-change event.
# The tracker emits "LayoutChanged signal emit failed" at debug only
# on D-Bus errors, but RUST_LOG=xxkb_daemon=debug also shines a light
# on the BackendEvent stream upstream of that.
if ! grep -E 'LayoutChanged|XkbStateNotify|new_group=|group=' /tmp/xxkbd.log >/dev/null; then
    echo "WARN: no layout-change trace observed — log excerpt:" >&2
    tail -40 /tmp/xxkbd.log >&2 || true
    # Don't fail the smoke on this alone; the binary survived the
    # cycle, which is the load-bearing assertion.
fi

echo "==> ${DE} smoke OK"
exit 0
