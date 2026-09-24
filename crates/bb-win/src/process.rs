//! Terminating a process via a handle already held by the caller.
//!
//! Breakbar keeps the OS handle a `std::process::Child` already owns (see
//! `bb-app::launcher::RunningClient::raw_handle`) and reuses it to stop the client later,
//! instead of re-opening the process by PID. Re-opening by PID has a real race: once a PID's
//! owning process exits, Windows is free to hand that PID to an unrelated process, and
//! terminating "by PID" at the wrong moment could hit that unrelated process instead.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::TerminateProcess;
use windows::core::Result;

/// Exit code recorded for a process ended via [`terminate`].
pub const TERMINATED_EXIT_CODE: u32 = 1;

/// Ends the process referenced by `handle`.
///
/// `handle` must be a still-open handle with `PROCESS_TERMINATE` access, such as the raw handle
/// of a `std::process::Child` (`std::os::windows::io::AsRawHandle::as_raw_handle`). This function
/// does not close it: whoever owns the handle keeps that responsibility. Calling it concurrently
/// with another thread waiting on the same handle (e.g. via `Child::wait`) is safe; Windows
/// handles have no thread affinity and support this without extra synchronization.
pub fn terminate(handle: isize) -> Result<()> {
    let handle = HANDLE(handle as *mut _);
    // SAFETY: `handle` is a live process handle owned by the caller for the duration of this
    // call (see doc comment); we only signal it here and never close it.
    unsafe { TerminateProcess(handle, TERMINATED_EXIT_CODE) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::Command;

    #[test]
    fn terminates_a_running_process() {
        let mut child = Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .expect("ping.exe should be available on every Windows installation");
        let handle = child.as_raw_handle() as isize;

        terminate(handle).unwrap();

        let status = child.wait().expect("wait should succeed after termination");
        assert!(!status.success());
    }
}
