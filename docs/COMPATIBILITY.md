# Compatibility matrix

`xxkb-rs` is an **X11** application. On Wayland sessions it runs
through XWayland; on pure-Wayland compositors that do not ship
XWayland (e.g. recent GNOME on a session without legacy support) it
will **not** start.

The matrix below records what we have confirmed to work, what is
known to be broken, and what we have not yet exercised. "Tested" =
xvfb integration test or human QA on real hardware. "Untested" =
expected to work but no one has tried it yet — please open an
issue if you do.

## Distributions

| Distro | Version | Status | Notes |
| --- | --- | --- | --- |
| Debian | 12 (Bookworm) | Tested in CI (`ubuntu-24.04` is the build host, deb-packaged for Bookworm) | Primary target. |
| Linux Mint | 21+ | Tested manually | Cinnamon and MATE editions. |
| Ubuntu | 22.04 | Builds, runs | GTK 4.6 / libadwaita 1.0 are too old for the GUI: the daemon and CLI work, but `xxkb-config` requires Ubuntu 24.04+ or a backported GTK. |
| Ubuntu | 24.04 | Tested in CI | Build / lint / unit / xvfb. |
| Fedora | 39+ | Untested | No reason it shouldn't work; PRs welcome. |
| Arch | Rolling | Untested | Likely fine, GTK 4.14+ is current. |

## Desktop environments / window managers

| DE / WM | Per-window indicator | Main indicator | Drag (Ctrl) | Notes |
| --- | --- | --- | --- | --- |
| Xfce 4.18 | ✅ Tested | ✅ Tested | ✅ Tested | The reference DE for the Docker smoke matrix. |
| MATE | ✅ Tested | ✅ Tested | ✅ Tested | Marco WM. |
| Cinnamon | ✅ Tested | ✅ Tested | ✅ Tested | Muffin WM. |
| LXDE | 🟡 Mostly | ✅ Tested | ✅ Tested | Openbox places indicators slightly off when client-side decorations are mixed in; CSD windows fall back to a fixed offset. |
| LXQt | 🟡 Untested | 🟡 Untested | 🟡 Untested | Should work via XWayland. |
| GNOME (X11) | 🟡 Tested manually | ✅ Tested | ✅ Tested | Mutter is fine. Header-bar-heavy GTK apps have inconsistent `_NET_FRAME_EXTENTS`; per-window placement can drift. |
| KDE Plasma 5 (X11) | 🟡 Tested manually | ✅ Tested | ✅ Tested | KWin sometimes lies about frame extents during animation; we re-place on `ConfigureNotify`. |
| KDE Plasma 6 (Wayland + XWayland) | 🟡 Tested manually | ✅ Tested | 🟡 Limited | Indicators show, but only for XWayland clients. Native-Wayland windows don't get a flag. |
| i3 / sway-i3-mode (X11) | ✅ Tested | ✅ Tested | ✅ Tested | i3 reports clean frame extents. |
| sway (Wayland) | ❌ Not supported | ❌ Not supported | n/a | sway has no XWayland window list available to a foreign X client; we don't get focus events for native-Wayland windows. |
| GNOME (pure Wayland, no XWayland) | ❌ Not supported | ❌ Not supported | n/a | No X server to attach to. |

## Display server

| Server | Status | Notes |
| --- | --- | --- |
| X.Org server 1.20+ | ✅ Tested | Required extensions: XKB, RandR, XFixes, XInput, XShape, XRender. |
| Xvfb | ✅ Tested in CI | Some outputs report `crtc=0` and we synthesise a virtual `screen` output as a fallback (see `xxkb-x11/src/monitors.rs`). |
| XWayland (current) | 🟡 Tested manually | XWayland clients only — see DE table. |

## Hardware / multi-monitor

| Setup | Status | Notes |
| --- | --- | --- |
| Single monitor | ✅ Tested | The default case. |
| Two monitors (extended) | ✅ Tested | Indicators are placed per-output via `MonitorLayout`; positions are saved keyed by output name. |
| HiDPI (200%) | 🟡 Limited | Indicator size is configured in pixels; GTK auto-scaling is not yet applied. Workaround: bump `main_indicator.size_px` to e.g. `96`. |
| Output hot-plug | 🟡 Partial | RandR `ScreenChangeNotify` triggers a layout refresh, but indicators on the disconnected output are not currently destroyed. Will be fixed under TODO `#04`. |

## Audio

| Backend | Status | Notes |
| --- | --- | --- |
| PulseAudio (via `rodio` → `cpal`) | ✅ Tested | The default on most desktops. |
| PipeWire (Pulse-compat shim) | ✅ Tested | Works transparently. |
| Bare ALSA | ✅ Tested | When PA / PW are absent. |
| Headless / no audio device | ✅ Tested | `xxkb_sound::build_player` falls back to `NullPlayer` and logs a `WARN`. |

## Keyboard layouts

| Setup | Status | Notes |
| --- | --- | --- |
| 2 layouts (e.g. `us,ru`) | ✅ Tested | Default in CI's xvfb. |
| 3 layouts (e.g. `us,ru,ua`) | ✅ Tested | `LayoutState` walks all groups when `general.two_state = false`. |
| 4 layouts | ✅ Tested | Maximum. XKB groups are 0..=3. |
| `grp:alt_shift_toggle`, `grp:caps_toggle`, etc. | ✅ Tested | Toggle hotkey is owned by XKB itself, we just observe `XkbStateNotify`. |

## Known limitations

* **Wayland-native windows** never get a per-window indicator; we
  rely on X11 properties (`WM_CLASS`, `_NET_FRAME_EXTENTS`) that
  Wayland clients do not expose. This is fundamental to the X11
  approach — a separate Wayland frontend would be a different
  product.
* **CSD-only apps** (GTK4 native-decoration apps) report
  `_NET_FRAME_EXTENTS = 0,0,0,0` and our placement falls back to
  the configured offset. The flag still appears, but on top of the
  client area rather than the title bar.
* **Multi-DPI sessions** (e.g. 1× external + 2× internal) use a
  single `size_px` for all outputs. Per-output scaling is on the
  TODO list.
* **Tiling WMs without decorations** (i3, awesome) have no title
  bar to anchor onto. The per-window indicator floats at the
  configured offset relative to the window's top-right corner.
