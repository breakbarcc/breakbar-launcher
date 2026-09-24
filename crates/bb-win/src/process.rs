//! Terminating a process via a handle already held by the caller.
//!
//! Breakbar keeps the OS handle a `std::process::Child` already owns (see
//! `bb-app::launcher::RunningClient::raw_handle`) and reuses it to stop the client later,
//! instead of re-opening the process by PID. Re-opening by PID has a real race: once a PID's
//! owning process exits, Windows is free to hand that PID to an unrelated process, and
//! terminating "by PID" at the wrong moment could hit that unrelated process instead.

use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::TerminateProcess;
use windows::core::Result;

/// Exit code recorded for a process ended via [`terminate`].
pub const TERMINATED_EXIT_CODE: u32 = 1;

/// Returns the process ids of all running processes whose executable file name matches
/// `exe_name` (case-insensitive), such as `"Gw2-64.exe"`.
///
/// Used to find Guild Wars 2 clients Breakbar didn't itself launch (e.g. already running from a
/// previous session), so their single-instance mutex can be closed too.
pub fn find_processes_by_name(exe_name: &str) -> Result<Vec<u32>> {
    // SAFETY: no preconditions; the returned handle is closed via the guard below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;
    let snapshot = SnapshotHandle(snapshot);

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry.dwSize` is set as required; `entry` is valid for the duration of the call.
    let mut result = unsafe { Process32FirstW(snapshot.0, &mut entry) };

    let mut pids = Vec::new();
    loop {
        match result {
            Ok(()) => {
                if exe_file_name(&entry.szExeFile).eq_ignore_ascii_case(exe_name) {
                    pids.push(entry.th32ProcessID);
                }
            }
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(error) => return Err(error),
        }
        // SAFETY: `entry` is valid for the duration of the call, matching `Process32FirstW`.
        result = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    Ok(pids)
}

/// Decodes a NUL-terminated, NUL-padded wide string from a `PROCESSENTRY32W::szExeFile` buffer.
fn exe_file_name(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

struct SnapshotHandle(HANDLE);

impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a handle this guard owns exclusively.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

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

    #[test]
    fn finds_a_running_process_by_name() {
        let mut child = Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .expect("ping.exe should be available on every Windows installation");

        let pids = find_processes_by_name("ping.exe").unwrap();

        terminate(child.as_raw_handle() as isize).unwrap();
        let _ = child.wait();

        assert!(
            pids.contains(&child.id()),
            "{pids:?} should contain {}",
            child.id()
        );
    }

    #[test]
    fn finds_no_process_for_an_unused_name() {
        let pids = find_processes_by_name("breakbar-definitely-not-running.exe").unwrap();
        assert!(pids.is_empty());
    }
}
