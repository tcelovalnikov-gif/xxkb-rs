//! Error types for the X11 backend.

use thiserror::Error;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};

/// Top-level error type.
#[derive(Debug, Error)]
pub enum X11Error {
    /// Failed to connect to the X server.
    #[error("X11 connect failed: {0}")]
    Connect(#[from] ConnectError),

    /// Connection-level error after we were connected.
    #[error("X11 connection error: {0}")]
    Connection(#[from] ConnectionError),

    /// Server-side reply error (e.g. BadWindow).
    #[error("X11 reply error: {0}")]
    Reply(#[from] ReplyError),

    /// Reply error on a request that needed a generated id.
    #[error("X11 reply-or-id error: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),

    /// A required X extension is missing.
    #[error("required X extension missing or wrong version: {0}")]
    MissingExtension(&'static str),

    /// Property had an unexpected encoding.
    #[error("property {name} has unexpected format: {detail}")]
    BadProperty {
        /// Property atom name.
        name: &'static str,
        /// Human-readable description.
        detail: String,
    },

    /// Generic protocol error from x11rb.
    #[error("x11rb error: {0}")]
    Other(String),
}
