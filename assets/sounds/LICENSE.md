# Sound assets

## click.wav

A short procedurally-generated click tone used as the default
audio cue when the keyboard layout changes.

* Format: WAV / PCM, 16-bit, mono, 22050 Hz, ~50 ms.
* Source: synthesised from a damped sine envelope by a small Python
  generator (no external audio source, no third-party samples).
* Author: xxkb-rs contributors.
* License: same as the project (see top-level `LICENSE`). You may
  redistribute and/or modify it under those terms.

The file is embedded into the `xxkb-sound` crate via `include_bytes!`
and ships with the `.deb` package at `/usr/share/xxkb/sounds/click.wav`.

If you want a different click, point `sound.file` in
`~/.config/xxkb/config.toml` at any WAV/OGG/MP3 the
[`rodio`](https://crates.io/crates/rodio) decoder can read.
