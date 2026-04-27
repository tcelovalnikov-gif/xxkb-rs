//! xxkbd entry point.

mod app;
mod flag;
mod hot_reload;

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn install_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    install_tracing();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting xxkbd");
    app::run().await
}
