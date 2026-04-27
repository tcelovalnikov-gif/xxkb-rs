//! D-Bus service `org.xxkb.Daemon1` on the session bus.
//!
//! Used by `xxkb-config` (the GUI configurator) to:
//! * pin the daemon to reload its config (`Reload`),
//! * fetch the live monitor list to render position editors (`GetMonitors`),
//! * fetch active windows for the rules editor (`GetActiveWindows`),
//! * push a "save these positions" payload after a drag (`SaveCurrentPositions`).
//!
//! The daemon also emits a `LayoutChanged` signal on every group switch.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms, missing_docs)]

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zbus::{interface, zvariant::Type};

/// Errors from the D-Bus subsystem.
#[derive(Debug, Error)]
pub enum DbusError {
    /// Underlying zbus error.
    #[error(transparent)]
    Zbus(#[from] zbus::Error),
    /// Failed to acquire the bus name.
    #[error("could not acquire org.xxkb.Daemon1 (already taken?): {0}")]
    NameTaken(String),
}

/// Wire-format of an output sent over the bus.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct WireOutput {
    /// Stable RandR name.
    pub name: String,
    /// X coordinate of the top-left corner.
    pub x: i32,
    /// Y coordinate of the top-left corner.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Primary?
    pub is_primary: bool,
    /// Active?
    pub is_active: bool,
}

/// Wire-format of an active window for the rules editor.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct WireWindow {
    /// X11 window id.
    pub wid: u32,
    /// `WM_CLASS` class part.
    pub wm_class_class: String,
    /// `WM_CLASS` instance part.
    pub wm_class_name: String,
    /// `WM_NAME` (or `_NET_WM_NAME`).
    pub wm_name: String,
}

/// Async trait implemented by the daemon to handle D-Bus calls.
///
/// We split this from the actual zbus interface so the daemon's logic
/// stays free of zbus types.
#[async_trait]
pub trait DaemonInterface: Send + Sync {
    /// Reload `~/.config/xxkb/config.toml`.
    async fn reload(&self) -> Result<(), String>;
    /// Snapshot of all RandR outputs.
    async fn outputs(&self) -> Result<Vec<WireOutput>, String>;
    /// Active windows currently tracked.
    async fn active_windows(&self) -> Result<Vec<WireWindow>, String>;
    /// Persist `(output_name -> (x, y))` map into config.
    async fn save_positions(&self, positions: HashMap<String, (i32, i32)>) -> Result<(), String>;
}

/// Adapter struct that zbus exports.
pub struct DaemonExporter<T: DaemonInterface + 'static> {
    inner: std::sync::Arc<T>,
}

impl<T: DaemonInterface + 'static> DaemonExporter<T> {
    /// Build.
    pub fn new(inner: std::sync::Arc<T>) -> Self {
        Self { inner }
    }
}

#[interface(name = "org.xxkb.Daemon1")]
impl<T: DaemonInterface + 'static> DaemonExporter<T> {
    /// Reload config.
    async fn reload(&self) -> zbus::fdo::Result<()> {
        self.inner.reload().await.map_err(zbus::fdo::Error::Failed)
    }

    /// Snapshot of monitors.
    async fn get_monitors(&self) -> zbus::fdo::Result<Vec<WireOutput>> {
        self.inner.outputs().await.map_err(zbus::fdo::Error::Failed)
    }

    /// Snapshot of active windows.
    async fn get_active_windows(&self) -> zbus::fdo::Result<Vec<WireWindow>> {
        self.inner
            .active_windows()
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Persist positions.
    async fn save_current_positions(
        &self,
        positions: HashMap<String, (i32, i32)>,
    ) -> zbus::fdo::Result<()> {
        self.inner
            .save_positions(positions)
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Emitted on every layout switch.
    #[zbus(signal)]
    async fn layout_changed(
        ctxt: &zbus::object_server::SignalContext<'_>,
        group_one_based: u8,
        wid: u32,
    ) -> zbus::Result<()>;
}

/// Spin up the D-Bus connection and register the interface at
/// `/org/xxkb/Daemon1`.
pub async fn serve<T: DaemonInterface + 'static>(
    iface: std::sync::Arc<T>,
) -> Result<zbus::Connection, DbusError> {
    let exporter = DaemonExporter::new(iface);
    let conn = zbus::connection::Builder::session()?
        .name("org.xxkb.Daemon1")
        .map_err(|e| DbusError::NameTaken(e.to_string()))?
        .serve_at("/org/xxkb/Daemon1", exporter)?
        .build()
        .await?;
    Ok(conn)
}
