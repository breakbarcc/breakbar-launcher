//! Auto-starting Discord alongside the first account, if the user wants that: a plain global
//! setting in [`bb_store::Config`], not tied to any particular account. Breakbar never closes
//! Discord again; the user does that themselves.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use bb_win::registry::{self, Root};

/// File name of Discord's stable launcher exe (the one at the install root, not one of the
/// versioned `app-x.y.z` subfolders it replaces on every auto-update).
pub const EXE_NAME: &str = "Discord.exe";

/// Discord installs per-user, so only `HKCU` has its uninstall entry.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Discord";

/// Finds Discord's launcher exe via its per-user uninstall registry entry.
#[must_use]
pub fn detect() -> Option<PathBuf> {
    let install_location =
        registry::read_string(Root::CurrentUser, UNINSTALL_KEY, "InstallLocation")?;
    let exe = PathBuf::from(install_location).join(EXE_NAME);
    exe.is_file().then_some(exe)
}

/// Starts `exe`, unless Discord is already running (its main process and its helper processes all
/// run as `Discord.exe`, the same as Chrome's multi-process model, so any match means it's up).
///
/// # Errors
///
/// Returns the OS error if `exe` could not be spawned.
pub fn start_if_not_running(exe: &Path) -> io::Result<()> {
    if is_running() {
        return Ok(());
    }
    Command::new(exe).spawn().map(|_| ())
}

fn is_running() -> bool {
    !bb_win::process::find_processes_by_name(EXE_NAME)
        .unwrap_or_default()
        .is_empty()
}
