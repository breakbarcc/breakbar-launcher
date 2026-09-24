//! Console helpers for the GUI-subsystem binary.

use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// Attaches to the console of the parent process (e.g. a terminal the user started
/// Breakbar from), so CLI output is visible even though release builds use the
/// GUI subsystem.
///
/// Returns `false` if the parent process has no console.
pub fn attach_parent_console() -> bool {
    // SAFETY: AttachConsole has no memory-safety preconditions.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS).is_ok() }
}
