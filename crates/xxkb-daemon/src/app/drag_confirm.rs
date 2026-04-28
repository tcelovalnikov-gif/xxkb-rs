//! Optional desktop confirmation before persisting a Ctrl-dragged main-indicator position.
//!
//! When [`xxkb_config::MainIndicatorConfig::confirm_drag_save`] is enabled, the daemon
//! asks via `zenity` or `kdialog` before writing `config.toml`. If neither tool is in
//! `PATH`, we log a warning and **allow** the save so positions are not lost on minimal
//! systems.

use std::process::Command;

/// Ask the user to confirm persisting `new_origin` for `output_name`.
///
/// Returns `true` if the save should proceed, `false` if the user declined or closed
/// the dialog without accepting.
#[must_use]
pub fn confirm_save_dragged_position(output_name: &str, x: i32, y: i32) -> bool {
    let text = format!(
        "Save new main-indicator position for {output_name} at screen coordinates ({x}, {y})?"
    );
    // If the tool exists, its exit status is authoritative — do not fall
    // through to a second dialog when the user clicked "No" in zenity.
    if let Ok(status) = Command::new("zenity")
        .args(["--question", "--no-wrap", "--text", &text])
        .status()
    {
        return status.success();
    }
    if let Ok(status) = Command::new("kdialog").args(["--yesno", &text]).status() {
        return status.success();
    }
    tracing::warn!(
        "main_indicator.confirm_drag_save is on but neither zenity nor kdialog could be run; \
         saving position anyway (install zenity or kdialog for a confirmation dialog)"
    );
    true
}
