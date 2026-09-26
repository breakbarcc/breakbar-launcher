//! Starting a program automatically when the user signs in, via the classic
//! `HKCU\...\Run` registry key (no admin rights needed, unlike a scheduled task or service).

use std::io;
use std::path::Path;

use crate::registry::{self, Root};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether `name` has a `Run` entry.
#[must_use]
pub fn is_enabled(name: &str) -> bool {
    registry::read_string(Root::CurrentUser, RUN_KEY, name).is_some()
}

/// Adds (or replaces) the `Run` entry for `name`, pointing at `exe`, quoted so a path
/// containing spaces still runs correctly.
///
/// # Errors
///
/// Returns the OS error if the registry can't be written.
pub fn enable(name: &str, exe: &Path) -> io::Result<()> {
    let command = format!("\"{}\"", exe.display());
    registry::write_string(Root::CurrentUser, RUN_KEY, name, &command)
}

/// Removes the `Run` entry for `name`, if any.
///
/// # Errors
///
/// Returns the OS error if the registry can't be written.
pub fn disable(name: &str) -> io::Result<()> {
    registry::delete_value(Root::CurrentUser, RUN_KEY, name)
}
