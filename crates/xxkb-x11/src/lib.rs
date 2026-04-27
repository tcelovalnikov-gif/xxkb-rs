//! X11 backend for xxkb-rs.
//!
//! Implementation lives under [`backend`]; the daemon uses [`Backend`] (a
//! trait) so we can write component tests with `MockBackend` from
//! `xxkb-test-utils`.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

pub mod backend;
pub mod errors;
pub mod events;
pub mod indicator_window;
pub mod monitors;
pub mod tracker;
pub mod xkb;

pub use backend::{Backend, X11Backend};
pub use errors::X11Error;
pub use events::{BackendEvent, IndicatorTarget, MouseButton, WindowGeom};
