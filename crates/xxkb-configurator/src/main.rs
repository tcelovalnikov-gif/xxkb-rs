//! xxkb-config — GUI configurator entry point.
//!
//! NOTE: GTK4 + libadwaita require a running display server. The
//! application code lives in `app.rs` and uses gtk4-rs. Without
//! `libgtk-4-dev` and `libadwaita-1-dev` installed at build time, this
//! crate will not compile — that's expected and documented in README.

mod app;

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn install_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();
}

fn main() -> Result<()> {
    install_tracing();
    app::run()
}
