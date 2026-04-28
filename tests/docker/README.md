# DE smoke-test images

Per-DE Dockerfiles that validate the `xxkb-daemon.deb` artefact
installs and runs against the runtime stacks of the major Linux Mint
spins:

* `Dockerfile.xfce` — Xfce 4 (Mint XFCE)
* `Dockerfile.mate` — MATE (Mint MATE)
* `Dockerfile.lxde` — LXDE (Mint LMDE-LXQt-adjacent / lightweight)

## What we verify

The smoke is intentionally narrow:

1. The `.deb` resolves and installs against the DE's typical
   runtime libraries (catches missing/wrong runtime depends).
2. The installed `xxkbd` binary boots under a minimal Xvfb session.
3. xxkbd creates at least one 48×48 override-redirect indicator
   window (RandR + XKB + render pipeline reached XPutImage).
4. A scripted `Alt+Shift_L` cycles XKB groups and xxkbd survives.

We **do not** boot a full DE session inside the container — that is
brittle in CI and the X11 protocol path is already covered by the
`xvfb-integration` job. The DE images exist to catch packaging
breakage (wrong `Depends:` line, conflicting libgtk versions,
missing `libpulse0`, etc.).

## Local invocation

```bash
# Build the .deb first; cargo-deb places it in dist/.
cargo deb -p xxkb-daemon --output dist/xxkb-daemon.deb

docker build -f tests/docker/Dockerfile.xfce -t xxkb-smoke-xfce .
docker run --rm -v "$PWD/dist:/dist" xxkb-smoke-xfce /smoke.sh
```

Same shape for `mate` and `lxde`.

## CI

`.github/workflows/ci.yml::smoke-de` runs the full matrix, gated to
`workflow_dispatch` and `schedule` triggers — these images each pull
~600 MB of DE packages, so we don't run them on every push.
