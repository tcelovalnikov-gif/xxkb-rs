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
//! so we can compile and unit-test the policy logic without `libasound`.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use parking_lot::Mutex;
use thiserror::Error;
use xxkb_config::SoundMode;

/// Errors from the sound subsystem.
#[derive(Debug, Error)]
pub enum SoundError {
    /// I/O error opening the sound file.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Decoder error.
    #[error("decode error: {0}")]
    Decode(String),
    /// Audio sink unavailable.
    #[error("audio sink: {0}")]
    Sink(String),
}

/// What kind of switch occurred (matches `xxkb_core::layout::SwitchKind`
/// but without the dependency to keep `xxkb-sound` slim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// User pressed a hotkey or clicked an indicator.
    Manual,
    /// Programmatic switch (focus change).
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
pub trait SoundPlayer: Send {
    /// Play, taking mode and trigger into account.
    fn play(&self, mode: SoundMode, trigger: Trigger);
}

/// Test player that just records calls.
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

    /// Snapshot of all `(mode, trigger)` pairs for which `play` was called
    /// **and** the policy returned true.
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

#[cfg(feature = "rodio-playback")]
mod rodio_player {
    use std::{io::Cursor, path::PathBuf};

    use parking_lot::Mutex;

    use super::{should_play, SoundError, SoundPlayer, Trigger};
    use xxkb_config::SoundMode;

    /// Real player, holds a `rodio::OutputStream` for the lifetime of the daemon.
    pub struct RodioPlayer {
        stream: Mutex<rodio::OutputStream>,
        _stream_handle: rodio::OutputStreamHandle,
        sound_bytes: Vec<u8>,
    }

    impl RodioPlayer {
        /// Build, loading sound bytes from `path` (or built-in default if empty).
        pub fn new(path: Option<PathBuf>) -> Result<Self, SoundError> {
            let (stream, handle) =
                rodio::OutputStream::try_default().map_err(|e| SoundError::Sink(e.to_string()))?;
            let sound_bytes = if let Some(p) = path {
                std::fs::read(p)?
            } else {
                BUILTIN_CLICK.to_vec()
            };
            Ok(Self {
                stream: Mutex::new(stream),
                _stream_handle: handle,
                sound_bytes,
            })
        }
    }

    impl SoundPlayer for RodioPlayer {
        fn play(&self, mode: SoundMode, trigger: Trigger) {
            if !should_play(mode, trigger) {
                return;
            }
            let cursor = Cursor::new(self.sound_bytes.clone());
            let _stream = self.stream.lock();
            // Re-create handle each time; rodio::Sink owns playback.
            if let Ok((_, handle)) = rodio::OutputStream::try_default() {
                if let Ok(decoder) = rodio::Decoder::new(cursor) {
                    if let Ok(sink) = rodio::Sink::try_new(&handle) {
                        sink.append(decoder);
                        sink.detach();
                    }
                }
            }
        }
    }

    // 1024 bytes of silence at 44.1 kHz mono — replaced at build time.
    const BUILTIN_CLICK: &[u8] = include_bytes!("../../../assets/sounds/click.ogg");
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
}
