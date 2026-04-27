//! Editor state for the `xxkb-config` GUI.
//!
//! This crate is intentionally **GTK-free** so that all the business
//! logic of the configurator (loading, mutating, validating and
//! persisting [`Config`], plus talking to the running daemon over
//! D-Bus) is unit-testable on a headless CI without GTK4 or
//! libadwaita.
//!
//! The `xxkb-configurator` GUI binary owns one [`ConfigEditor`] and
//! binds GTK widgets to its setters. On "Save" it calls
//! [`ConfigEditor::save_to_default`] (which writes
//! `~/.config/xxkb/config.toml` atomically) and optionally
//! [`dbus_client::ping_reload`] to nudge the running daemon.
//!
//! [`Config`]: xxkb_config::Config

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

mod editor;
mod validation;

pub mod dbus_client;

pub use editor::ConfigEditor;
pub use validation::ValidationError;
