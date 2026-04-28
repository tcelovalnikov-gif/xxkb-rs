# Manual test checklist

This is the human-driven QA pass we run before tagging a release.
The automated CI suite (`cargo test`, xvfb integration, Docker DE
smoke) covers the regressions; this checklist is for the things a
script can't see — visual placement, audio cue, "feels right".

Target environment for each pass: Debian 12 or Linux Mint 21+ with
GNOME / Cinnamon / Xfce / KDE on a single-monitor laptop AND a
two-monitor desktop. Two layouts: `us,ru,grp:alt_shift_toggle`.

## 0. Environment sanity

- [ ] `xkbcomp -display "$DISPLAY" - <<< ""` — server speaks XKB.
- [ ] `xrandr --query` — RandR speaks, at least one output is
  `connected primary`.
- [ ] `pactl info` (or `pw-cli info 0`) — there is an audio sink.
- [ ] `dbus-send --session --print-reply
  --dest=org.freedesktop.DBus / org.freedesktop.DBus.ListNames |
  grep -v org.xxkb` — no other `xxkbd` is already on the bus.

## 1. First run from a clean profile

- [ ] `rm -rf ~/.config/xxkb` and start `xxkbd` from a terminal.
  - Daemon prints `INFO loading config` and uses defaults.
  - No file is written until the user makes a change.
- [ ] A flag indicator appears at the top-right of the primary
  display, ~48×48 px, showing `en` (US) on first boot.
- [ ] Open a terminal — within 100 ms, a small (~15 px) `en` flag
  appears over its title bar, near the close button.
- [ ] Press `Alt+Shift` to switch to `ru`.
  - Both indicators flip to `ru` simultaneously.
  - No audible click (default `sound.mode = off`).

## 2. Per-window memory

- [ ] Open Firefox and Telegram side-by-side.
- [ ] Focus Firefox, switch to `ru`. Focus Telegram → layout
  resets to `en`. Focus Firefox again → `ru` is restored.
- [ ] Close Firefox, reopen it → starts on `en` again (memory
  is per-WID, not per-WM_CLASS, by default).

## 3. Main indicator: drag, save, mode

- [ ] Hold `Ctrl` and drag the main indicator on the primary
  output to a new position.
  - The indicator follows the pointer.
  - On release, `~/.config/xxkb/config.toml` gains
    `[main_indicator.positions]` with the new `(x, y)` keyed by
    the output name.
- [ ] Restart `xxkbd`. The indicator reappears at the dragged
  position.
- [ ] In `xxkb-config`, set `main_indicator.mode =
  primary_only`. Save. The secondary monitor's indicator
  disappears.
- [ ] Set it back to `all_displays`. Indicator reappears on the
  secondary.

## 4. Per-window indicator placement

- [ ] Resize a terminal — flag stays on the title bar (re-placed
  on `ConfigureNotify`).
- [ ] Maximise a window — flag moves to the new top-right.
- [ ] Move a window to the second monitor — flag follows, offset
  recalculated against the new screen position.
- [ ] Open a GTK4 CSD app (e.g. `gnome-text-editor`). The flag
  shows but at the configured offset (no real title bar to
  anchor to). Document, don't fail.

## 5. Hot-reload

- [ ] Edit `~/.config/xxkb/config.toml` in another editor:
  `main_indicator.size_px = 96`. Save.
  - Within ~250 ms (debounce window), the daemon logs
    `INFO config reloaded`.
  - Indicator grows to 96 px on the next repaint.
- [ ] Introduce a typo: `main_indicato.size_px = 96`. Save.
  - Daemon logs `WARN reload failed: unknown field …`.
  - Old indicator stays — daemon does not crash.
- [ ] Fix the typo. Daemon picks the change up.

## 6. Configurator GUI

- [ ] Launch `xxkb-config`. Window opens within 1 s, libadwaita
  toolbar with pages: General / Main indicator / Window
  indicator / Icons / Sound / Rules.
- [ ] Toggle `general.two_state` → row updates instantly.
- [ ] Tap **Save**. Title bar drops the dirty marker. Daemon
  picks up the change (verify via main indicator behaviour).
- [ ] Make an invalid change (`size_px = 0`). Save. A red
  `adw::Toast` appears with the validation error. File on disk
  is not modified.
- [ ] On the Rules page, click **Capture from active window**
  while a terminal is focused. A row appears with that
  terminal's `WM_CLASS`. Set it to `Ignore` and Save.
  - Restart the daemon (or rely on hot-reload). The terminal's
    per-window flag disappears.

## 7. Sound

- [ ] In `xxkb-config`, set `sound.mode = manual_only`. Save.
- [ ] Press `Alt+Shift`. Click is audible.
- [ ] Switch focus between two windows that have different
  remembered layouts — focus-driven switch, no click.
- [ ] Set `sound.mode = both`. Click is audible on both.
- [ ] Set `sound.mode = off`. Silence.
- [ ] Provide a custom file via `sound.file = "/tmp/click.ogg"`.
  - File exists → custom click plays.
  - File missing → daemon logs `WARN failed to read sound.file
    … falling back to silent player`. No panic.

## 8. D-Bus

With `xxkbd` running:

- [ ] `gdbus introspect --session --dest org.xxkb.Daemon1
  --object-path /org/xxkb/Daemon1` lists `Reload`, `GetMonitors`,
  `GetActiveWindows`, `SaveCurrentPositions`, `Version`, `Ping`.
- [ ] `gdbus call --session --dest org.xxkb.Daemon1
  --object-path /org/xxkb/Daemon1 --method
  org.xxkb.Daemon1.Ping` returns `('pong',)`.
- [ ] `gdbus call ... org.xxkb.Daemon1.Version` returns the same
  string as `xxkbd --version`.
- [ ] `gdbus monitor --session --dest org.xxkb.Daemon1` then
  press `Alt+Shift`. A `LayoutChanged` signal fires with the
  new group and the active window id.

## 9. Multi-monitor

- [ ] Connect a second display (`xrandr --output … --right-of
  …`).
  - Daemon receives `RRScreenChangeNotify`, logs `INFO outputs
    changed`.
  - A new main indicator appears on the second display (assuming
    `mode = all_displays`).
- [ ] Disconnect it. Indicator on the now-disconnected output
  vanishes (TODO: this currently leaks an indicator window;
  document the bug).

## 10. Crash and recovery

- [ ] `pkill -SEGV xxkbd` — should never happen, but verify the
  systemd user unit restarts the daemon (`Restart=on-failure`)
  within 2 s.
- [ ] Lock the screen and unlock — daemon survives, indicator
  reappears on the new session screen if your DE recreates the
  X session.

## 11. Packaging

- [ ] `sudo dpkg -i dist/xxkb-daemon_*.deb dist/xxkb-configurator_*.deb`.
  - No file conflicts.
  - `/usr/share/xxkb/sounds/click.wav`,
    `/usr/share/xxkb/icons/*.svg`,
    `/etc/xdg/autostart/xxkbd.desktop`,
    `/usr/lib/systemd/user/xxkbd.service`,
    `/usr/share/applications/xxkb-config.desktop` all installed.
- [ ] `systemctl --user enable --now xxkbd.service` →
  `Active: active (running)`.
- [ ] Log out and log back in. Daemon starts via the autostart
  desktop file (or systemd user unit, depending on DE).

---

If any of these fail, file an issue with:

* DE / WM and version (`echo $XDG_CURRENT_DESKTOP`,
  `<wm> --version`).
* `journalctl --user -u xxkbd -n 200 --no-pager` output.
* `~/.config/xxkb/config.toml` (sanitise positions if you'd
  rather not share monitor geometry).
