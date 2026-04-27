//! Test fixtures and mocks for xxkb-rs.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

pub mod mock_backend;

pub use mock_backend::{MockBackend, MockBackendBuilder};
