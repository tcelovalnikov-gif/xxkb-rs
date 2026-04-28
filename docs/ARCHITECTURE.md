# Architecture

`xxkb-rs` is a from-scratch Rust rewrite of the classic
[`xxkb`](https://github.com/uliscat/xxkb) — a per-window keyboard
layout indicator and switcher for X11. It targets Debian 12 / Linux
Mint 21+ desktops (X11 native and XWayland).

The code is split into a Cargo workspace of small, single-purpose
crates so that each layer can be unit-tested in isolation.

## Component map

```
                      ┌─────────────────────────┐
                      │      xxkb-config        │
                      │ (TOML schema, figment,  │
                      │  validation, save/load) │
                      └───────────▲─────────────┘
                                  │
       ┌──────────────────────────┼─────────────────────────┐
       │                          │                         │
       │            ┌─────────────┴────────────┐            │
       │            │      xxkb-core           │            │
       │            │ LayoutState, Registry,   │            │
       │            │ AppRules, MonitorLayout, │            │
       │            │ IndicatorPlacement       │            │
       │            └─────────────▲────────────┘            │
       │                          │                         │
┌──────┴───────┐   ┌───────────────┼─────────────┐  ┌───────┴────────┐
│  xxkb-x11    │   │   xxkb-indicators           │  │   xxkb-sound   │
│ (x11rb,      │◀──│  resvg + tiny-skia →        │──▶│ rodio +        │
│  XKB, RandR, │   │  BGRA pixmap, IconCache,    │  │ NullPlayer     │
│  trackers,   │   │  border drawing             │  │ fallback       │
│  override-rd │   └─────────────────────────────┘  └────────────────┘
│  windows)    │
└──────▲───────┘
       │
       │  events                           ┌───────────────────────┐
       │                                   │     xxkb-dbus         │
┌──────┴───────────────────────────────┐   │  org.xxkb.Daemon1     │
│            xxkb-daemon (xxkbd)        │──▶│  iface trait,         │
│  ─ owns Tokio runtime                 │   │  Emitter, signals,    │
│  ─ wires backend + config + indicators│   │  typed DaemonProxy    │
│  ─ runs hot-reload watcher            │   └─────────▲─────────────┘
│  ─ exports D-Bus interface            │             │
│  ─ owns SoundPlayer                   │             │
└──────────────────────────────────────┘             │
                                                      │
                              ┌───────────────────────┴──────┐
                              │      xxkb-config-state       │
                              │  ConfigEditor (dirty + valid)│
                              │  blocking + async DBus client│
                              │  uses generated DaemonProxy  │
                              └───────────────────────▲──────┘
                                                      │
                                          ┌───────────┴──────────┐
                                          │  xxkb-configurator   │
                                          │  GTK4 + libadwaita   │
                                          │  bin: xxkb-config    │
                                          └──────────────────────┘
```

## Crate responsibilities

| Crate | Role | Tested |
| --- | --- | --- |
| `xxkb-core` | Pure logic, no I/O. `LayoutState`, `WindowRegistry`, `AppRules`, `MonitorLayout`, `IndicatorPlacement`. | unit + proptest |
| `xxkb-config` | TOML schema, `figment` loader (defaults + file + filtered `XXKB_*` env), atomic save, validation. | unit |
| `xxkb-config-state` | Editor-state for the GUI: `ConfigEditor` with dirty-tracking, baseline/current snapshots, validation. Blocking + async D-Bus client built on the typed `xxkb_dbus::DaemonProxy`. | unit |
| `xxkb-x11` | All X11 traffic: XKB state, RandR, override-redirect indicator windows, paint-pixbuf, passive Ctrl-drag, `_NET_FRAME_EXTENTS` + EWMH-fallback parent-walk via `QueryTree` for WMs that don't advertise the atom. Trait `Backend` is mockable. | unit + xvfb integration |
| `xxkb-indicators` | SVG/PNG → BGRA pixmap (`resvg` + `tiny-skia` + `image`). `IconCache` keyed by `(name, size, border)`. Border drawing on top of the buffer. | unit |
| `xxkb-sound` | Mode-vs-trigger policy (`should_play`), `SoundPlayer` trait, `MockPlayer` / `NullPlayer` / `RodioPlayer` (behind `rodio-playback`), `build_player()` factory. | unit |
| `xxkb-dbus` | Canonical `org.xxkb.Daemon1` definition: `DaemonInterface` trait, `DaemonService` exporter, `Emitter` for signals, typed `DaemonProxy`. `is_daemon_present()` helper. | unit + p2p integration (`tests/roundtrip.rs`) |
| `xxkb-daemon` | The `xxkbd` binary. Wires backend, config, indicators, sound, D-Bus, and hot-reload around a Tokio runtime. | unit + xvfb integration |
| `xxkb-configurator` | The `xxkb-config` binary. GTK4 + libadwaita GUI bound to `ConfigEditor`. Persists on Save and pings the daemon over D-Bus to reload. | smoke |
| `xxkb-migrate` | The `xxkb-migrate` CLI. Parses legacy `~/.xxkbrc` X-resources and emits TOML. | bin only (tests pending) |
| `xxkb-test-utils` | Shared test helpers: `MockBackend`, `tempdir` builders, etc. | n/a |

## Daemon runtime

`xxkbd` boots in `crates/xxkb-daemon/src/app.rs::run()`:

1. **Load config** (`Config::load_default`). On first boot, defaults are
   used and nothing is written to disk yet.
2. **Connect to X** via `X11Backend::connect`. This sets up XKB,
   RandR, XFixes, XInput extensions and starts an event tracker
   thread that emits `BackendEvent`s on a `crossbeam` channel.
3. **Build core state**: `LayoutState`, `WindowRegistry`,
   `MonitorLayout`, `AppRules`, `IconCache`, `GeomCache`,
   `PropsCache`.
4. **Build the sound player** via `xxkb_sound::build_player(...)`
   (`NullPlayer` if `mode = off` or no audio device; otherwise
   `RodioPlayer`).
5. **Place initial main indicators** on every active output.
6. **Export D-Bus interface** `org.xxkb.Daemon1` (typed proxy on
   the client side, signals via `Emitter`). If the well-known name
   is taken (another `xxkbd` running), the daemon logs a warning
   and continues *without* the bus — the indicators still work.
7. **Start hot-reload watcher** on `~/.config/xxkb/config.toml` via
   `notify-debouncer-mini`. The watcher captures a Tokio runtime
   handle so its callback (which runs on a non-Tokio worker thread)
   can `spawn` the async reload.
8. **Run `event_loop`** forever. Reacts to:
   * `LayoutChanged` → cycle the player click, repaint main and
     per-window indicators, emit `LayoutChanged` D-Bus signal.
   * `ActiveWindowChanged` → consult `AppRules` (Ignore /
     StartAlt / AltGroup), restore remembered group from
     `WindowRegistry`, place per-window indicator.
   * `WindowGeometryChanged` → re-place per-window indicator using
     cached geometry + frame extents.
   * `IndicatorClicked` → cycle group via `LayoutState`.
   * `IndicatorDragged` → store new position in
     `Config.main_indicator.positions[output_name]` and atomically
     save the TOML.

## D-Bus contract

Service `org.xxkb.Daemon1`, object path `/org/xxkb/Daemon1`,
interface `org.xxkb.Daemon1`.

Methods:

| Method | Args | Returns | Purpose |
| --- | --- | --- | --- |
| `Reload` | — | — | Re-read `config.toml` from disk and rewire the daemon. Emits `Reloaded(ok)` afterwards. |
| `GetMonitors` | — | `a(...)` of `WireOutput` | Active RandR outputs. |
| `GetActiveWindows` | — | `a(...)` of `WireWindow` | Recently-seen windows with `WM_CLASS` / `WM_NAME` for the rules editor. |
| `SaveCurrentPositions` | `a{s(ii)}` | — | Bulk-save `output_name → (x, y)` from the GUI. Emits `PositionsSaved(count)`. |
| `Version` | — | `s` | `CARGO_PKG_VERSION`. |
| `Ping` | — | `s` | Liveness check. Returns `"pong"`. |

Signals:

* `LayoutChanged(group_one_based: u8, wid: u32)` — emitted from the
  daemon's `event_loop` whenever `LayoutState::observe` advances.
* `Reloaded(ok: bool)` — emitted after `Reload`.
* `PositionsSaved(count: u32)` — emitted after `SaveCurrentPositions`.

The interface is generated **once** in `xxkb-dbus` via `#[zbus::proxy]`
and used as the typed `DaemonProxy` from both `xxkbd` (for testing)
and the configurator (`xxkb-config-state`). There is no hand-rolled
`Proxy::call_method` anywhere in the tree.

## Testing strategy

* **Unit tests** in every crate. Pure-logic crates (`xxkb-core`,
  `xxkb-config`, `xxkb-config-state`) have heavy coverage including
  `proptest` round-trips for the TOML schema.
* **In-process D-Bus** tests (`xxkb-dbus/tests/roundtrip.rs`) wire
  a `StubDaemon` to a `DaemonProxy` over a `tokio::net::UnixStream`
  pair using zbus's p2p mode. No system bus required.
* **xvfb integration** (`tests/xvfb/run_all.sh`) boots a real
  `xxkbd` under Xvfb, sets up a `us,ru` keymap with
  `setxkbmap`, and checks that an override-redirect indicator
  window appears at the expected size and survives an XKB switch.
  Self-skips when `$DISPLAY` / `$XXKB_TEST_XVFB` are absent so
  `cargo test --workspace` is safe on a developer laptop.
* **Docker DE smoke** (`tests/docker/`, scheduled / manual only)
  installs the built `.deb` inside Xfce / MATE / LXDE images and
  asserts the daemon starts via the autostart `.desktop` file.
* **CI** (`.github/workflows/ci.yml`): `lint` (rustfmt + clippy),
  `unit` (whole workspace minus the GUI binary), `xvfb-integration`,
  `package-deb` (cargo-deb for both packages), `smoke-de` (matrix).

## Where to start reading

* End-to-end overview: `README.md`.
* Config reference: [`docs/CONFIG.md`](CONFIG.md).
* DE/WM compatibility: [`docs/COMPATIBILITY.md`](COMPATIBILITY.md).
* Manual QA checklist: [`docs/MANUAL_TEST.md`](MANUAL_TEST.md).
