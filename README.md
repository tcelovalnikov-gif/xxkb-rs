# xxkb-rs

Современный rewrite классического [`xxkb`](https://github.com/uliscat/xxkb) на
Rust + GTK4 — индикатор и переключатель раскладки клавиатуры с флагом
**на каждом окне** для X11 / XWayland.

> **Статус: alpha / work-in-progress.** Костяк собран, на CI зеленый,
> но часть функций ТЗ ещё не доведена. Подробности — в разделе
> [«Что сделано / Что осталось»](#что-сделано--что-осталось).

---

## Зачем это нужно

Старый `xxkb` (C + Xlib) написан больше 20 лет назад под Xt/X11 и
плохо живёт на современных Debian 12+ / Linux Mint 21+:

* не работает per-window индикатор у части GTK3/GTK4-приложений;
* сборка тянет древние `Xaw`/`libXt`;
* конфиг в формате X-resources, без hot-reload;
* отдельной GUI-настройки нет вообще, всё руками в `~/.xxkbrc`.

Этот rewrite сохраняет идею — «маленький флаг текущей раскладки на
каждом окне» — и закрывает технический долг:

* чистый Rust, X11-протокол через [`x11rb`](https://crates.io/crates/x11rb)
  без `libXt`;
* отрисовка через `resvg` + `tiny-skia` (SVG → BGRA pixmap), без `cairo`/
  `librsvg` runtime-deps;
* TOML-конфиг с hot-reload (`inotify` / `notify-debouncer-mini`);
* отдельный GUI-настройщик на GTK4 + libadwaita;
* D-Bus сервис `org.xxkb.Daemon1` для интеграции с настройщиком и
  сторонними утилитами;
* поддержка >2 раскладок с автоподхватом из системных настроек XKB.

## Архитектура

Воркспейс из десяти крейтов:

| Крейт | Назначение |
| --- | --- |
| `xxkb-core` | Чистая логика без I/O: `LayoutState`, `WindowRegistry`, `AppRules`, `MonitorLayout`, `IndicatorPlacement`. |
| `xxkb-config` | TOML-схема (`Config`), загрузка/сохранение через `figment`+`serde`, валидация. |
| `xxkb-config-state` | Editor-state для конфигуратора: `ConfigEditor` с dirty-tracking и валидацией + D-Bus-клиент. Без GTK, тестируется в headless CI. |
| `xxkb-x11` | X11-бэкенд: XKB-события, RandR, paint-pixbuf, override-redirect окна, Ctrl-drag, `_NET_FRAME_EXTENTS`. |
| `xxkb-indicators` | SVG/PNG-рендер флагов, `IconCache` по `(name, size, border)`, отрисовка окантовки. |
| `xxkb-sound` | Логика «когда играть»: `SoundMode × Trigger → bool`. `RodioPlayer` за фичей `rodio-playback`. |
| `xxkb-dbus` | Описание интерфейса `org.xxkb.Daemon1` + `serve()`-хелпер. |
| `xxkb-daemon` | Бинарник `xxkbd`. Связывает x11 + конфиг + индикаторы + sound + dbus + hot-reload. |
| `xxkb-configurator` | Бинарник `xxkb-config`. GTK4 + libadwaita GUI поверх `ConfigEditor`. |
| `xxkb-migrate` | Бинарник `xxkb-migrate`. Конвертирует legacy `~/.xxkbrc` → TOML. |
| `xxkb-test-utils` | `MockBackend` и общие утилиты для тестов. |

## Что сделано / Что осталось

Чек-лист по пунктам ТЗ + задачам из todo-листа.

### ✅ Готово

* **Воркспейс и CI.** `cargo fmt --check`, `cargo clippy -D warnings` и
  `cargo test --workspace` (84 теста на момент первого коммита) — зелёные
  на GitHub Actions (`ubuntu-22.04`).
* **TOML-схема конфига** (`xxkb-config`). `figment` (defaults + file +
  `XXKB_*` env), атомарная запись через `tempfile`, валидация
  кросс-полей, отказ от `unknown_fields`. Round-trip-тест.
* **Доменная логика** (`xxkb-core`). `LayoutState` (включая two-state и
  >2 групп), `WindowRegistry` (запоминание раскладки на окно),
  `AppRules` (компилируемый glob-матчер на `WM_CLASS`/`WM_NAME`),
  `MonitorLayout` (RandR-аутпуты), `IndicatorPlacement` (рассчёт
  позиции индикатора с учётом `_NET_FRAME_EXTENTS`).
* **X11-бэкенд** (`xxkb-x11`). XKB state-notify → `LayoutChanged`,
  RandR-аутпуты, override-redirect окна, `XPutImage` пиксельных
  буферов, passive Ctrl+Button1 drag главного индикатора + click для
  цикла раскладок, чтение `_NET_FRAME_EXTENTS` и геометрии в
  root-координатах, `ConfigureNotify`/`PropertyNotify` →
  `WindowGeometryChanged`.
* **Per-window индикатор.** Real placement через
  `IndicatorPlacement::compute` с реальными origin/width/frame, кэш
  геометрии и свойств окна, повторное позиционирование на каждое
  изменение геометрии или фрейм-extents.
* **Рендер флагов** (`xxkb-indicators`). Pure-Rust SVG → BGRA
  (`resvg` + `tiny-skia`), PNG (`image`), `IconCache` по
  `(name, size, border)`, отрисовка окантовки поверх pixel-буфера.
* **Bundled-флажки** в `assets/icons/`: `en, ru, ua, by, kz, de, fr`.
* **Hot-reload.** `notify-debouncer-mini` следит за
  `~/.config/xxkb/config.toml`, `tokio::runtime::Handle` корректно
  пробрасывается из не-tokio worker-thread в главный рантайм.
  Юнит-тесты на «срабатывает на запись цели» и «игнорирует соседний
  файл».
* **Главный индикатор: cycle-by-click + save position on drag.** Демон
  слушает `IndicatorClicked`/`IndicatorDragged`, циклирует группу через
  `LayoutState`, сохраняет позицию в `Config.main_indicator.positions`,
  пишет TOML.
* **GUI-конфигуратор** (`xxkb-config`) на GTK4 + libadwaita. Все
  страницы (`General` / `Main indicator` / `Window indicator` / `Icons` /
  `Sound` / `Rules`) биндят виджеты к `ConfigEditor`. Save → атомарная
  запись + ping `org.xxkb.Daemon1.Reload` через D-Bus в отдельном
  worker-потоке. Ошибки валидации показываются как `adw::Toast`.
  Реальная бизнес-логика (валидация, dirty-tracking, save/discard) —
  в `xxkb-config-state` и **полностью покрыта 19 юнит-тестами**.
* **xvfb-смок-тест.** Поднимает реальный `xxkbd` под Xvfb, проверяет
  что демон создал override-redirect окно нужного размера и не упал
  на `XPutImage`. Self-skips без `$DISPLAY`/`XXKB_TEST_XVFB=1`.
* **D-Bus-скелет.** Интерфейс `org.xxkb.Daemon1` формализован: типизированный
  `DaemonProxy` (zbus `#[proxy]`), методы `Reload` / `GetMonitors` /
  `GetActiveWindows` / `SaveCurrentPositions` / `Version` / `Ping`,
  сигналы `LayoutChanged(group, wid)` / `Reloaded(ok)` /
  `PositionsSaved(count)`. `DaemonHandle` в `xxkbd` эмитит сигналы из
  `event_loop` через `Emitter`. In-process p2p-интеграционный тест в
  `xxkb-dbus/tests/roundtrip.rs` гоняет полный round-trip
  client↔server без реального DBus-демона. D-Bus-клиент в
  `xxkb-config-state` (async + blocking) переведён на типизированный
  proxy.
* **Sound: реальный rodio-плеер.** `xxkbd` собирает плеер через
  `xxkb_sound::build_player()` на старте: при `mode = off` или
  отсутствии аудио-устройства возвращается `NullPlayer`, иначе —
  `RodioPlayer` с одним долгоживущим `OutputStream` и кэшированным
  буфером. `Trigger::Manual/Auto` пробрасывается из `BackendEvent::
  LayoutChanged`. Встроенный 50 ms / 22.05 kHz / 16-bit click
  (`assets/sounds/click.wav`, ~2.2 KB) embedded через `include_bytes!`
  и пакуется в `.deb` в `/usr/share/xxkb/sounds/`.
* **Packaging.** `assets/desktop/xxkbd.desktop` (autostart) +
  `assets/desktop/xxkb-config.desktop` (Settings menu),
  `packaging/systemd/xxkbd.service` (user unit, `WantedBy=
  graphical-session.target`). `cargo deb` для обоих пакетов
  ungated в CI.

### 🟡 Частично

* **`xxkb-x11`.** Помечен в todo как `in_progress` — основная
  функциональность есть, но не покрыты edge-кейсы:
  multi-screen-layout-переключения на лету, нестандартные WM-ы без
  `_NET_FRAME_EXTENTS`, EWMH-fallback на старых KWin / Mutter.
* **Главный индикатор.** Cycle и save-position работают, но **режимы
  отображения ("primary only" vs "all displays") и save-dialog при
  драге** в самом X11-бэкенде ещё не реализованы — пока всегда
  `all_displays`.
* **`xxkb-migrate`.** ~350 строк парсера legacy X-resources формата +
  CLI. Скомпилирован, но **0 тестов** — ни на один реальный
  `~/.xxkbrc` не прогонялся, корректность маппинга не валидирована.

### 🔴 Осталось сделать

Пункты, по которым ничего не написано или только заглушка:

* **`#06` — Display indicator: режимы + save-dialog + привязка к
  output_name.** Нужно реально применять `MainIndicatorMode::PrimaryOnly`/
  `AllDisplays`, спрашивать «сохранить новую позицию?» через GTK
  message-dialog после drop-а драга.
* **`#14` — Docker smoke под Xfce / MATE / LXDE.** В `tests/docker/`
  пока пусто. Нужно три `Dockerfile.<de>` + `smoke.sh`, гонять
  установленный `.deb` под каждым DE.
* **`#16` — Migration tool:** написать тесты на реальные конфиги (есть
  пара примеров в репо upstream, плюс распространённые случаи с
  `app_list.*`).
* **`#17` — Документация.** ✅ Готово. Полный референс конфига и
  архитектуры — в [`docs/`](docs/):
  [`ARCHITECTURE.md`](docs/ARCHITECTURE.md),
  [`CONFIG.md`](docs/CONFIG.md),
  [`COMPATIBILITY.md`](docs/COMPATIBILITY.md),
  [`MANUAL_TEST.md`](docs/MANUAL_TEST.md).

## Сборка

В `ubuntu-22.04` / Debian 12 / Linux Mint 21+:

```bash
sudo apt install \
    libgtk-4-dev libadwaita-1-dev \
    librsvg2-dev libcairo2-dev libglib2.0-dev \
    libdbus-1-dev libxkbcommon-dev libxcb1-dev \
    libxcb-randr0-dev libxcb-xkb-dev libxcb-xfixes0-dev \
    libxcb-shape0-dev libxcb-render0-dev libxcb-xinput-dev \
    libasound2-dev libpulse-dev pkg-config

cargo build --release
```

Бинарники окажутся в `target/release/`:

* `xxkbd` — демон (запускать в начале сессии);
* `xxkb-config` — GUI-настройщик;
* `xxkb-migrate` — конвертер legacy `~/.xxkbrc`.

## Тестирование

```bash
# Юнит и интеграционные тесты, кроме GUI-бинарника
# (тот требует libgtk-4-dev/libadwaita-1-dev и сейчас исключён по
# умолчанию из CI-теста).
cargo test --workspace --exclude xxkb-configurator

# Смок-тест демона под Xvfb (требует xvfb-run + xkb-data + dbus-x11).
xvfb-run -a --server-args='-screen 0 1920x1080x24' \
    bash tests/xvfb/run_all.sh
```

CI поверх `ubuntu-22.04` гоняет:

1. `lint` — `cargo fmt --check` и `cargo clippy --all-targets -- -D warnings`
   на весь воркспейс (включая GUI, т.к. в CI стоят системные GTK-deps).
2. `unit` — `cargo test --workspace --exclude xxkb-configurator`.
3. `xvfb-integration` — `xxkbd` под Xvfb + `xdotool` + `wmctrl`.
4. `package-deb` — `cargo deb` для `xxkbd` и `xxkb-config`.
5. `smoke-de` (по расписанию / dispatch) — Docker smoke под
   Xfce/MATE/LXDE. **Сейчас не работает: нет Dockerfile-ов.**

## Лицензия

MIT — см. [LICENSE](LICENSE). Оригинальный xxkb upstream также под MIT.

## Связь с оригиналом

Этот репозиторий — **не fork** оригинального `xxkb`. Это переписанный
с нуля проект, использующий ТЗ оригинала и сохраняющий имя «xxkb»
по сути функции, но архитектурно ничем не связанный с C/Xlib-кодом
upstream. Поведение конфига и app-rules максимально совместимо;
для миграции есть отдельный конвертер.
