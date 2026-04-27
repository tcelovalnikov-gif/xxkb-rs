//! Validation errors raised by [`ConfigEditor`](crate::ConfigEditor) setters.
//!
//! Each variant carries the offending value so the GUI can surface a
//! useful toast — e.g. "size must be > 0, got 0" rather than a generic
//! "invalid input".

use thiserror::Error;

/// Why a [`ConfigEditor`](crate::ConfigEditor) setter rejected an input.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// A pixel size was zero (or otherwise out of range).
    #[error("size must be in 1..={max}, got {got}")]
    OutOfRange {
        /// What the user requested.
        got: u32,
        /// Largest accepted value (inclusive).
        max: u32,
    },
    /// A border color string was not in `#RRGGBB` / `#RRGGBBAA` form.
    #[error("color must be #RRGGBB or #RRGGBBAA, got {0:?}")]
    BadColor(String),
    /// An app-rule glob pattern failed to compile.
    #[error("invalid glob pattern {pattern:?}: {reason}")]
    BadGlob {
        /// The offending pattern.
        pattern: String,
        /// `globset`'s error message.
        reason: String,
    },
    /// An index was outside the addressable range of the rules list.
    #[error("index {got} out of range (len = {len})")]
    BadIndex {
        /// The bad index.
        got: usize,
        /// Current list length.
        len: usize,
    },
    /// A 1-based group identifier was not in 1..=4.
    #[error("group must be in 1..=4, got {0}")]
    BadGroup(u8),
}

impl ValidationError {
    pub(crate) fn check_color(s: &str) -> Result<(), Self> {
        // Accept '#RRGGBB' and '#RRGGBBAA' only; we are deliberately
        // strict so the daemon's later `border::parse_rgba` doesn't
        // have to second-guess. The daemon tolerates a missing '#' for
        // backwards compat but the GUI should always emit canonical form.
        if !s.starts_with('#') {
            return Err(Self::BadColor(s.to_owned()));
        }
        let hex = &s[1..];
        if hex.len() != 6 && hex.len() != 8 {
            return Err(Self::BadColor(s.to_owned()));
        }
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Self::BadColor(s.to_owned()));
        }
        Ok(())
    }

    pub(crate) fn check_size(got: u32, max: u32) -> Result<(), Self> {
        if got == 0 || got > max {
            return Err(Self::OutOfRange { got, max });
        }
        Ok(())
    }

    pub(crate) fn check_group(g: u8) -> Result<(), Self> {
        if !(1..=4).contains(&g) {
            return Err(Self::BadGroup(g));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_must_be_hash_prefixed() {
        assert!(matches!(
            ValidationError::check_color("000000"),
            Err(ValidationError::BadColor(_))
        ));
        assert!(ValidationError::check_color("#000000").is_ok());
        assert!(ValidationError::check_color("#0011AAff").is_ok());
        assert!(matches!(
            ValidationError::check_color("#abc"),
            Err(ValidationError::BadColor(_))
        ));
        assert!(matches!(
            ValidationError::check_color("#GGGGGG"),
            Err(ValidationError::BadColor(_))
        ));
    }

    #[test]
    fn size_zero_rejected() {
        assert!(matches!(
            ValidationError::check_size(0, 256),
            Err(ValidationError::OutOfRange { got: 0, max: 256 })
        ));
    }

    #[test]
    fn size_above_max_rejected() {
        assert!(matches!(
            ValidationError::check_size(1024, 256),
            Err(ValidationError::OutOfRange {
                got: 1024,
                max: 256
            })
        ));
    }

    #[test]
    fn group_outside_range_rejected() {
        assert!(matches!(
            ValidationError::check_group(0),
            Err(ValidationError::BadGroup(0))
        ));
        assert!(matches!(
            ValidationError::check_group(5),
            Err(ValidationError::BadGroup(5))
        ));
        assert!(ValidationError::check_group(1).is_ok());
        assert!(ValidationError::check_group(4).is_ok());
    }
}
