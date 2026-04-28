# Changelog

All notable changes to this project are documented in this file.

## 0.2.0 — 2026-04-28

### Added

- `main_indicator.confirm_drag_save` — optional `zenity` / `kdialog`
  confirmation before persisting a Ctrl-dragged main-indicator position;
  declining restores the previous coordinates.
- D-Bus `PositionsSaved(1)` after each successful drag-save (in addition
  to the existing signal from `SaveCurrentPositions`).
- `xxkb-config`: background listener for `PositionsSaved` + toast; GUI
  toggle for `confirm_drag_save`.
- `xxkb-migrate`: file-backed fixtures under `tests/fixtures/*.xxkbrc` and
  integration tests in `tests/fixture_files.rs`.
- `MonitorLayout` unit test for orphan `positions` keys vs active outputs.
- GitHub Actions: `workflow_dispatch` on the CI workflow so **DE smoke**
  can be run manually (job still gated: not on every push).

### Documentation

- `CONFIG.md`, `MANUAL_TEST.md`: drag confirmation, toasts, output names,
  orphan keys, Docker smoke / workflow dispatch.

## 0.1.0 — earlier

Initial public snapshot (workspace, daemon, configurator, D-Bus, tests).
