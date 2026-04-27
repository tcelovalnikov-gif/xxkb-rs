#!/usr/bin/env bash
# Driver for the xxkb-rs Xvfb integration tests.
#
# Expected to be invoked under `xvfb-run` (which provides DISPLAY) by CI.
# Example:
#
#     xvfb-run -a --server-args='-screen 0 1920x1080x24' \
#         bash tests/xvfb/run_all.sh
#
# Prerequisites on the host: xvfb, x11-xkb-utils (setxkbmap),
# xkb-data, dbus-x11, and a built rust toolchain.
set -euo pipefail

cd "$(dirname "$0")/../.."

if [[ -z "${DISPLAY:-}" ]]; then
    echo "FATAL: DISPLAY is not set — invoke under xvfb-run." >&2
    exit 1
fi

echo "==> DISPLAY=${DISPLAY}"
xdpyinfo -display "${DISPLAY}" >/dev/null || {
    echo "FATAL: cannot connect to ${DISPLAY}" >&2
    exit 1
}

# Multi-group keymap so XKB has something to switch to (otherwise XKB
# only reports group 0 forever and Manual switches are a no-op).
echo "==> setting up two-group XKB keymap (us,ru)"
setxkbmap -display "${DISPLAY}" -layout us,ru -option grp:alt_shift_toggle

# Some daemons want a session bus available. Run the rest under
# `dbus-run-session` if we don't already have one.
if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]] && command -v dbus-run-session >/dev/null; then
    echo "==> wrapping in dbus-run-session"
    exec dbus-run-session -- "$0" "$@"
fi

export XXKB_TEST_XVFB=1

echo "==> running cargo test xvfb_smoke"
cargo test \
    -p xxkb-daemon \
    --test xvfb_smoke \
    -- \
    --nocapture \
    --test-threads=1
