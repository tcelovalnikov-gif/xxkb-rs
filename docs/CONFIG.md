# Configuration reference

xxkb-rs reads its configuration from
`$XDG_CONFIG_HOME/xxkb/config.toml` (defaulting to
`~/.config/xxkb/config.toml`).

* The format is **TOML 1.0**.
* Unknown keys are **rejected** at load time
  (`#[serde(deny_unknown_fields)]`). This is intentional: a typo in
  a key like `enable_per_window` should not silently fall back to
  defaults.
* The file is **atomically rewritten** on save (`tempfile + persist`)
  so a crash mid-write cannot leave a half-baked file.
* Changes are **picked up live** by `xxkbd` via `inotify` (see
  `xxkb-daemon::hot_reload`), with a 200 ms debounce to coalesce
  editor saves.

Environment-variable overrides are supported with the prefix
`XXKB_<SECTION>__<FIELD>=...`. The `__` separator is mandatory —
plain `XXKB_FOO=...` is ignored so that test runners and unrelated
exports don't crash the daemon. Example:

```bash
XXKB_GENERAL__BASE_GROUP=2 XXKB_SOUND__MODE=both xxkbd
```

## Top-level structure

```toml
[general]
[main_indicator]
[main_indicator.border]
[main_indicator.positions]
[per_window_indicator]
[per_window_indicator.border]
[per_window_indicator.offset]
[icons]
[icons.mapping]
[sound]
[[app_rules]]
```

Every section is optional; missing sections fall back to defaults.

---

## `[general]`

Global, mode-affecting flags.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `two_state` | bool | `false` | If true, the cycle-on-click toggles only between `base_group` and `alt_group`. If false, it walks through all configured groups. |
| `base_group` | int (1..=4) | `1` | Primary group, 1-based. |
| `alt_group` | int (1..=4) | `2` | Alternative group, 1-based. |
| `cycle_modifier` | enum | `"none"` | Modifier required to cycle layouts via the legacy hotkey. Values: `none`, `shift`, `lock`, `ctrl` (alias `control`), `alt`, `mod1`..`mod5`. |
| `ignore_reverse` | bool | `false` | Inverts the meaning of `app_rules` `Ignore`: only matched windows are managed. |

```toml
[general]
two_state = false
base_group = 1
alt_group = 2
cycle_modifier = "ctrl"
ignore_reverse = false
```

---

## `[main_indicator]`

The "main" flag indicator that lives on each display.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `enable` | bool | `true` | Master switch. |
| `mode` | enum | `"all_displays"` | `"primary_only"` shows only on the RandR primary; `"all_displays"` shows on every active output. |
| `size_px` | int (>0) | `48` | Side length in pixels (square). |
| `confirm_drag_save` | bool | `false` | If `true`, after Ctrl-drag the daemon asks with `zenity` or `kdialog` before writing the file; **No** snaps the indicator back. If neither tool works, the position is still saved (see logs). |

```toml
[main_indicator]
enable = true
mode = "all_displays"
size_px = 48
confirm_drag_save = false
```

### `[main_indicator.border]`

| Key | Type | Default |
| --- | --- | --- |
| `enabled` | bool | `false` |
| `color` | string `"#RRGGBB"` or `"#RRGGBBAA"` | `"#000000"` |
| `width` | int | `1` |

The leading `#` is required; validation rejects bare hex.

```toml
[main_indicator.border]
enabled = true
color = "#202020"
width = 2
```

### `[main_indicator.positions]`

Saved positions, keyed by RandR output name (e.g. `eDP-1`, `HDMI-1`).
The daemon writes here after Ctrl-drag (unless you cancel a confirmation
dialog when `confirm_drag_save = true`). Keys for unplugged monitors may
remain until you edit the file; they do not affect placement on other
outputs.

```toml
[main_indicator.positions]
HDMI-1 = { x = 1840, y = 30 }
DP-1 = { x = 1840, y = 30 }
```

---

## `[per_window_indicator]`

The small flag drawn over each window's title bar.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `enable` | bool | `true` | Master switch. |
| `size_px` | int (>0) | `15` | Side length in pixels. |

### `[per_window_indicator.offset]`

Pixel offset relative to the title bar. Negative `x` measures from
the right edge.

| Key | Type | Default |
| --- | --- | --- |
| `x` | int | `-60` |
| `y` | int | `7` |

### `[per_window_indicator.border]`

Same shape as `[main_indicator.border]`.

```toml
[per_window_indicator]
enable = true
size_px = 15

[per_window_indicator.offset]
x = -60
y = 7

[per_window_indicator.border]
enabled = false
color = "#000000"
width = 1
```

---

## `[icons]`

Flag icon configuration.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `prefer_svg` | bool | `true` | Prefer SVG over raster when both are available. |
| `search_paths` | array of strings | see below | Search paths for icons. The literals `"system"` and `"builtin"` are recognised special values and are replaced with the system-wide path (`/usr/share/xxkb/icons`) and the bundled SVGs respectively. |

```toml
[icons]
prefer_svg = true
search_paths = [
    "~/.local/share/icons/xxkb",
    "system",
    "builtin",
]
```

### `[icons.mapping]`

Maps `1`-based group to icon name. Keys are stringified ints because
TOML tables require string keys.

| Group | Default name |
| --- | --- |
| `"1"` | `"en"` |
| `"2"` | `"ru"` |
| `"3"` | `"ua"` |
| `"4"` | `"by"` |

```toml
[icons.mapping]
"1" = "en"
"2" = "ru"
"3" = "de"
```

The bundled set is `en`, `ru`, `ua`, `by`, `kz`, `de`, `fr` (in
`assets/icons/*.svg`). To add your own, drop a `<name>.svg` or
`<name>.png` into one of the `search_paths`.

---

## `[sound]`

Layout-change click.

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `mode` | enum | `"off"` | `off`, `manual_only`, `auto_only`, `both`. |
| `file` | string | `""` | Optional path to a custom WAV/OGG/MP3. Empty → built-in 50 ms click. |

`manual_only` plays only on user-initiated switches (hotkey / click);
`auto_only` plays only on programmatic switches (focus change /
daemon-initiated). `both` plays for everything.

```toml
[sound]
mode = "manual_only"
file = ""
```

If `xxkbd` was built with `--no-default-features` (no
`rodio-playback`), or there is no audio device available
(headless container, no PulseAudio session), the player silently
degrades to a no-op — the daemon stays usable.

---

## `[[app_rules]]`

Per-application overrides. Mirrors the legacy
`XXkb.app_list.<property>.<action>` X-resource.

Each rule has:

| Key | Type | Notes |
| --- | --- | --- |
| `match_` | object | One of `wm_class_class`, `wm_class_name`, `wm_name`. The value is a glob pattern (`*`, `?`, `[abc]`). |
| `action` | object | One of `ignore`, `start_alt`, `alt_group = N` (1-based group). |

```toml
[[app_rules]]
match_ = { wm_class_class = "Firefox" }
action = "start_alt"

[[app_rules]]
match_ = { wm_class_name = "gnome-terminal-server" }
action = { alt_group = 2 }

[[app_rules]]
match_ = { wm_name = "*Telegram*" }
action = "ignore"
```

Glob semantics: `*` matches any number of characters, `?` matches
one. Patterns are case-sensitive (legacy xxkb is too).

---

## Migration from legacy `~/.xxkbrc`

Run `xxkb-migrate < ~/.xxkbrc > ~/.config/xxkb/config.toml` (the
binary ships with the daemon). The migrator tries to preserve all
`XXkb.*` keys it understands; unknown keys are reported on stderr.

> **Note**: as of this writing the migrator is unit-tested only on
> synthetic inputs. If you have a real `~/.xxkbrc` from upstream
> xxkb and the migration produces a wrong TOML, please open an
> issue with the original file attached.

---

## Validation rules

The daemon **refuses to start** if any of these are violated:

* `general.base_group ∈ 1..=4`.
* `general.alt_group ∈ 1..=4`.
* `main_indicator.size_px > 0`.
* `per_window_indicator.size_px > 0`.
* `main_indicator.border.color` starts with `#`.

The configurator GUI shows the same errors as `adw::Toast` before
they reach disk.
