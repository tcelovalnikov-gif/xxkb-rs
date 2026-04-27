//! `xxkb-migrate` CLI: convert `~/.xxkbrc` into `~/.config/xxkb/config.toml`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(false))
        .init();

    let mut args = std::env::args().skip(1);
    let input: PathBuf = args
        .next()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".xxkbrc")))
        .context("no input path and no $HOME")?;
    let output: PathBuf = args
        .next()
        .map(PathBuf::from)
        .or_else(|| xxkb_config::config_path().ok())
        .context("no output path and config_path failed")?;

    if !input.exists() {
        anyhow::bail!("input does not exist: {}", input.display());
    }

    tracing::info!(?input, ?output, "migrating");
    let cfg = xxkb_migrate::migrate_file(&input)?;
    cfg.save_to(&output)?;
    tracing::info!("done");
    Ok(())
}
