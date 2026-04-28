//! Layout-change sound playback.
//!
//! Mirrors `XXkb.bell.enable` from legacy xxkb but is aware of the
//! `manual` vs `auto` distinction (TZ requirement):
//!
//! * `Off` — never play.
//! * `ManualOnly` — play only when the layout switch was triggered by a
//!   user action (hotkey or click).
//! * `AutoOnly` — play only on focus-driven, programmatic switches.
//! * `Both` — always play.
//!
//! The actual rodio playback lives behind the `rodio-playback` feature
//! (enabled by default) so we can compile and unit-test the policy
//! logic on systems without `libasound2-dev`.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use xxkb_config::SoundMode;

/// Errors from the sound subsystem.
#[derive(Debug, Error)]
pub enum SoundError {
    /// I/O error opening the sound file.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Decoder error (mostly: unknown / corrupt audio container).
    #[error("decode error: {0}")]
    Decode(String),
    /// Audio sink unavailable — usually means there is no ALSA / Pulse
    /// device, or the user's session does not have access to one.
    #[error("audio sink: {0}")]
    Sink(String),
}

/// What kind of switch occurred (matches `xxkb_core::layout::SwitchKind`
/// but without the dependency to keep `xxkb-sound` slim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// User pressed a hotkey or clicked an indicator.
    Manual,
    /// Programmatic switch (focus change, daemon-initiated).
    Auto,
}

/// Pure-logic decision: should we play given mode and trigger?
#[must_use]
pub const fn should_play(mode: SoundMode, trigger: Trigger) -> bool {
    match (mode, trigger) {
        (SoundMode::Off, _) => false,
        (SoundMode::ManualOnly, Trigger::Manual) => true,
        (SoundMode::ManualOnly, Trigger::Auto) => false,
        (SoundMode::AutoOnly, Trigger::Manual) => false,
        (SoundMode::AutoOnly, Trigger::Auto) => true,
        (SoundMode::Both, _) => true,
    }
}

/// Trait so the daemon can swap a `MockPlayer` in tests.
pub trait SoundPlayer: Send + Sync {
    /// Play, taking mode and trigger into account.
    fn play(&self, mode: SoundMode, trigger: Trigger);
}

/// Test player that just records `(mode, trigger)` calls for which
/// [`should_play`] returned `true`.
#[derive(Debug, Default)]
pub struct MockPlayer {
    calls: Mutex<Vec<(SoundMode, Trigger)>>,
}

impl MockPlayer {
    /// New empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of all `(mode, trigger)` pairs for which `play` was
    /// called **and** the policy returned `true`.
    #[must_use]
    pub fn calls(&self) -> Vec<(SoundMode, Trigger)> {
        self.calls.lock().clone()
    }
}

impl SoundPlayer for MockPlayer {
    fn play(&self, mode: SoundMode, trigger: Trigger) {
        if should_play(mode, trigger) {
            self.calls.lock().push((mode, trigger));
        }
    }
}

/// No-op player. Returned by [`build_player`] when the user has set
/// `mode = "off"` or when audio device init fails on a headless box.
#[derive(Debug, Default)]
pub struct NullPlayer;

impl SoundPlayer for NullPlayer {
    fn play(&self, _: SoundMode, _: Trigger) {}
}

/// Built-in click — a 50 ms / 22.05 kHz / 16-bit mono sine envelope
/// generated at repo bake time. About 2.2 KB. Decoded by rodio's
/// `symphonia-wav` integration; no system C deps required.
///
/// Exposed as a `&[u8]` so callers don't have to know the file path.
pub const BUILTIN_CLICK_WAV: &[u8] = include_bytes!("../../../assets/sounds/click.wav");

/// Build the sound player to use based on the parsed config.
///
/// Resolution order for the audio buffer:
/// 1. `cfg.file` if non-empty (read from disk),
/// 2. otherwise [`BUILTIN_CLICK_WAV`].
///
/// On systems without an audio device — CI runners, headless KVM
/// guests, GitHub Actions — `RodioPlayer::new` fails at
/// `try_default()` and we fall back to [`NullPlayer`] with a single
/// `WARN` line. The daemon stays usable; only the bell goes silent.
///
/// On `--no-default-features` builds (no `rodio-playback` feature)
/// this function is still callable but always returns
/// [`NullPlayer`].
pub fn build_player(cfg_mode: SoundMode, cfg_file: &str) -> Arc<dyn SoundPlayer> {
    if matches!(cfg_mode, SoundMode::Off) {
        tracing::debug!("sound mode = off, using NullPlayer");
        return Arc::new(NullPlayer);
    }
    #[cfg(feature = "rodio-playback")]
    {
        let path: Option<&std::path::Path> = if cfg_file.is_empty() {
            None
        } else {
            Some(std::path::Path::new(cfg_file))
        };
        match rodio_player::RodioPlayer::new(path) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                tracing::warn!(error = %e, "audio sink unavailable; falling back to silent player");
                Arc::new(NullPlayer)
            }
        }
    }
    #[cfg(not(feature = "rodio-playback"))]
    {
        let _ = cfg_file;
        tracing::warn!("xxkb-sound built without rodio-playback feature; sound will not play");
        Arc::new(NullPlayer)
    }
}

#[cfg(feature = "rodio-playback")]
mod rodio_player {
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::{
        mpsc::{self, Sender},
        Arc,
    };
    use std::thread::{self, JoinHandle};

    use parking_lot::Mutex;
    use rodio::OutputStream;
    use xxkb_config::SoundMode;

    use super::{should_play, SoundError, SoundPlayer, Trigger, BUILTIN_CLICK_WAV};

    /// Real player. The actual `cpal::Stream` (held by rodio's
    /// `OutputStream`) is `!Send + !Sync` on Linux's ALSA / Pulse
    /// backends, which is incompatible with [`SoundPlayer: Send +
    /// Sync`]. So `RodioPlayer` does *not* hold the stream itself;
    /// instead it spawns a dedicated audio thread that owns the
    /// stream and listens on an `mpsc` channel for play requests.
    /// `play()` just signals the thread.
    ///
    /// This also keeps `RodioPlayer` cheap to share across the
    /// async runtime: the daemon clones an `Arc<dyn SoundPlayer>`
    /// to every async task, and the lock contention on the sender
    /// is negligible (one message per layout switch).
    pub struct RodioPlayer {
        /// Send (cloned) end into the audio thread. Wrapped in
        /// `Mutex` so the type is `Sync` (`mpsc::Sender` is `Send`
        /// but `!Sync`). Lock is held for microseconds at a time —
        /// just long enough to push one message — so contention is
        /// not a concern.
        tx: Mutex<Sender<()>>,
        /// Audio worker. Held only so its `Drop` joins the thread
        /// when the player is dropped — we never poll it.
        _thread: JoinHandle<()>,
    }

    impl RodioPlayer {
        /// Build, optionally loading sound bytes from `path`. Falls
        /// back to the built-in click if `path` is `None`. Returns
        /// `Err` when no audio device can be opened *at all* — the
        /// daemon then falls back to [`super::NullPlayer`].
        pub fn new(path: Option<&Path>) -> Result<Self, SoundError> {
            // Pre-flight: open an output stream synchronously so we
            // can surface "no audio device" as a real error instead
            // of a silent thread that died on init. The probe is
            // dropped immediately; the worker opens its own.
            {
                let _probe =
                    OutputStream::try_default().map_err(|e| SoundError::Sink(e.to_string()))?;
            }

            let bytes: Arc<Vec<u8>> = Arc::new(match path {
                Some(p) => std::fs::read(p)?,
                None => BUILTIN_CLICK_WAV.to_vec(),
            });

            let (tx, rx) = mpsc::channel::<()>();
            let worker_bytes = bytes;
            let thread = thread::Builder::new()
                .name("xxkb-sound".to_owned())
                .spawn(move || worker_loop(rx, worker_bytes))
                .map_err(|e| SoundError::Sink(format!("spawn audio thread: {e}")))?;

            Ok(Self {
                tx: Mutex::new(tx),
                _thread: thread,
            })
        }
    }

    /// Audio worker thread. Owns the (`!Send`) `OutputStream` for
    /// its entire lifetime; receives play requests over `rx` and
    /// hands a freshly decoded source to a detached `Sink` for each
    /// click. Exits cleanly when all `Sender` clones are dropped.
    fn worker_loop(rx: mpsc::Receiver<()>, bytes: Arc<Vec<u8>>) {
        let (_stream, handle) = match OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                // We pre-flighted in `new()`, so this only fires
                // if the device disappeared between probe and
                // worker init. Log once and exit; further `play()`
                // calls become no-ops because the channel has no
                // receiver.
                tracing::warn!(error = %e, "audio worker: OutputStream unavailable");
                return;
            }
        };
        while rx.recv().is_ok() {
            // Cursor needs `T: AsRef<[u8]>`. `Arc<Vec<u8>>` doesn't
            // satisfy that directly, so we deref+clone the inner
            // Vec here. The buffer is ~2 KB so the copy is cheap.
            let cursor = Cursor::new(bytes.as_ref().clone());
            let decoder = match rodio::Decoder::new(cursor) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to decode click sample");
                    continue;
                }
            };
            match rodio::Sink::try_new(&handle) {
                Ok(sink) => {
                    sink.append(decoder);
                    // Detach so dropping the local `sink` doesn't
                    // truncate playback at the closing brace.
                    sink.detach();
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to acquire audio sink");
                }
            }
        }
    }

    impl SoundPlayer for RodioPlayer {
        fn play(&self, mode: SoundMode, trigger: Trigger) {
            if !should_play(mode, trigger) {
                return;
            }
            // If the worker is gone (shutdown / device disappeared)
            // the send fails silently — that's the correct
            // behaviour: no audio device, no click.
            let _ = self.tx.lock().send(());
        }
    }

    /// Compile-time guarantee that we don't accidentally regress
    /// to a `RodioPlayer` that holds a non-Send `cpal::Stream`
    /// directly. If a future refactor leaks a `!Send` type back
    /// into the struct, this will fail to compile.
    #[cfg(test)]
    const _: fn() = || {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RodioPlayer>();
    };
}

#[cfg(feature = "rodio-playback")]
pub use rodio_player::RodioPlayer;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn off_never_plays() {
        let p = MockPlayer::new();
        p.play(SoundMode::Off, Trigger::Manual);
        p.play(SoundMode::Off, Trigger::Auto);
        assert!(p.calls().is_empty());
    }

    #[test]
    fn manual_only_plays_only_on_manual() {
        let p = MockPlayer::new();
        p.play(SoundMode::ManualOnly, Trigger::Manual);
        p.play(SoundMode::ManualOnly, Trigger::Auto);
        assert_eq!(p.calls(), vec![(SoundMode::ManualOnly, Trigger::Manual)]);
    }

    #[test]
    fn auto_only_plays_only_on_auto() {
        let p = MockPlayer::new();
        p.play(SoundMode::AutoOnly, Trigger::Manual);
        p.play(SoundMode::AutoOnly, Trigger::Auto);
        assert_eq!(p.calls(), vec![(SoundMode::AutoOnly, Trigger::Auto)]);
    }

    #[test]
    fn both_plays_for_everything() {
        let p = MockPlayer::new();
        p.play(SoundMode::Both, Trigger::Manual);
        p.play(SoundMode::Both, Trigger::Auto);
        assert_eq!(p.calls().len(), 2);
    }

    /// `build_player` with `mode=Off` MUST short-circuit to
    /// [`NullPlayer`] without ever touching the audio device. This
    /// is what lets us run the daemon in `Off` mode on a headless
    /// container without `WARN`s.
    #[test]
    fn build_player_off_returns_null() {
        let p = build_player(SoundMode::Off, "");
        // Smoke: calling .play() should be a no-op and not panic.
        p.play(SoundMode::Off, Trigger::Manual);
        p.play(SoundMode::Both, Trigger::Manual);
        // No public observability on NullPlayer — but if there's
        // ever a regression that returns a real player, an audio
        // device init attempt will be visible in tracing logs.
    }

    /// Sanity-check that the bundled click is a plausible WAV: it
    /// starts with `RIFF` and contains the `WAVE` form id at offset
    /// 8. If a future commit accidentally clobbers the asset with
    /// an empty file or a different format this test catches it
    /// before the daemon segfaults at runtime.
    #[test]
    fn builtin_click_is_a_wav_file() {
        assert!(
            BUILTIN_CLICK_WAV.len() > 100,
            "click.wav looks suspiciously small: {} bytes",
            BUILTIN_CLICK_WAV.len()
        );
        assert_eq!(&BUILTIN_CLICK_WAV[0..4], b"RIFF");
        assert_eq!(&BUILTIN_CLICK_WAV[8..12], b"WAVE");
    }
}
