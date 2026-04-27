# Xvfb integration tests

This directory wires up the headless X11 smoke tests that exercise the
real `xxkbd` binary against an isolated `Xvfb` server.

## Quick start

```bash
sudo apt-get install -y xvfb x11-xkb-utils xkb-data dbus-x11 x11-utils
xvfb-run -a --server-args='-screen 0 1920x1080x24' bash tests/xvfb/run_all.sh
```

The runner:

1. Verifies it has a `$DISPLAY` (i.e. it's invoked under `xvfb-run`).
2. Configures a two-group XKB keymap (`us,ru`) so XKB has something to
   switch between.
3. Wraps itself in `dbus-run-session` if no session bus is already
   present.
4. Sets `XXKB_TEST_XVFB=1` and runs `cargo test -p xxkb-daemon
   --test xvfb_smoke`.

The Rust test (`crates/xxkb-daemon/tests/xvfb_smoke.rs`) self-skips when
`XXKB_TEST_XVFB` is not set, so it stays a no-op for ordinary
`cargo test` runs on developer machines.

## What's verified

The current smoke is intentionally minimal:

* the daemon spawns and stays alive past startup;
* it creates at least one override-redirect window of the configured
  indicator size (default 48 × 48);
* round-tripping `ClearArea` on that window is X-error-free, which
  confirms the render → `XPutImage` → `ChangeWindowAttributes` chain
  ran to completion.

Future tests in this directory will cover:

* manual layout switches via `xdotool key alt+shift_l` cycling through
  groups and re-painting the indicator;
* drag-with-Ctrl moving the indicator and rewriting `config.toml`;
* RandR output add/remove via `xrandr --addmode`.
