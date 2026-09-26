//! Terminating a process via a handle already held by the caller.
//!
//! Breakbar keeps the OS handle a `std::process::Child` already owns (see
//! `bb-app::launcher::RunningClient::raw_handle`) and reuses it to stop the client later,
//! instead of re-opening the process by PID. Re-opening by PID has a real race: once a PID's
//! owning process exits, Windows is free to hand that PID to an unrelated process, and
//! terminating "by PID" at the wrong moment could hit that unrelated process instead.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};
use windows::core::Result;

/// Exit code recorded for a process ended via [`terminate`].
pub const TERMINATED_EXIT_CODE: u32 = 1;

/// Returns the process ids of all running processes whose executable file name matches
/// `exe_name` (case-insensitive), such as `"Gw2-64.exe"`.
///
/// Used to find Guild Wars 2 clients Breakbar didn't itself launch (e.g. already running from a
/// previous session), so their single-instance mutex can be closed too.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn find_processes_by_name(exe_name: &str) -> Result<Vec<u32>> {
    Ok(snapshot()?
        .into_iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case(exe_name))
        .map(|entry| entry.pid)
        .collect())
}

/// Returns the process ids of all running processes named `exe_name` (case-insensitive) that
/// were started by process `parent_pid`.
///
/// The parent may already have exited: Windows keeps the recorded parent id, so a client that
/// restarts itself (after updating its own executable) can still be found through the process it
/// replaced. A reused parent id could in theory match an unrelated process, but only one with the
/// same executable name that was started by a process with that very id.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn find_child_processes(parent_pid: u32, exe_name: &str) -> Result<Vec<u32>> {
    Ok(snapshot()?
        .into_iter()
        .filter(|entry| entry.parent_pid == parent_pid && entry.name.eq_ignore_ascii_case(exe_name))
        .map(|entry| entry.pid)
        .collect())
}

struct ProcessEntry {
    pid: u32,
    parent_pid: u32,
    name: String,
}

fn snapshot() -> Result<Vec<ProcessEntry>> {
    // SAFETY: no preconditions; the returned handle is closed via the guard below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;
    let snapshot = SnapshotHandle(snapshot);

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry.dwSize` is set as required; `entry` is valid for the duration of the call.
    let mut result = unsafe { Process32FirstW(snapshot.0, &raw mut entry) };

    let mut entries = Vec::new();
    loop {
        match result {
            Ok(()) => entries.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent_pid: entry.th32ParentProcessID,
                name: exe_file_name(&entry.szExeFile),
            }),
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(error) => return Err(error),
        }
        // SAFETY: `entry` is valid for the duration of the call, matching `Process32FirstW`.
        result = unsafe { Process32NextW(snapshot.0, &raw mut entry) };
    }
    Ok(entries)
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
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn terminate(handle: isize) -> Result<()> {
    let handle = HANDLE(handle as *mut _);
    // SAFETY: `handle` is a live process handle owned by the caller for the duration of this
    // call (see doc comment); we only signal it here and never close it.
    unsafe { TerminateProcess(handle, TERMINATED_EXIT_CODE) }
}

/// A process the caller holds a handle to: one it spawned (from its `Child`) or one it opened
/// by id. Unlike `Child`, it can also stand for a process somebody else started.
#[derive(Debug)]
pub struct Process(OwnedHandle);

impl From<OwnedHandle> for Process {
    /// `handle` needs `PROCESS_TERMINATE`, `SYNCHRONIZE` and `PROCESS_QUERY_LIMITED_INFORMATION`
    /// access, which the handle of a spawned `Child` has (`OwnedHandle::from(child)`).
    fn from(handle: OwnedHandle) -> Self {
        Self(handle)
    }
}

impl Process {
    /// Opens the running process `pid` for waiting on it and terminating it.
    ///
    /// # Errors
    ///
    /// Returns the Windows error if the underlying call fails.
    pub fn open(pid: u32) -> Result<Self> {
        // SAFETY: no preconditions; the returned handle is owned by the `OwnedHandle` below.
        let handle = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )
        }?;
        // SAFETY: `handle` was just returned by `OpenProcess` and nobody else owns it.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle.0) }))
    }

    /// The raw handle, valid as long as this value lives (see [`terminate`]).
    #[must_use]
    pub fn raw_handle(&self) -> isize {
        self.0.as_raw_handle() as isize
    }

    /// The process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        // SAFETY: the handle is a live process handle owned by `self`.
        unsafe { GetProcessId(self.handle()) }
    }

    /// The exit code if the process has exited, `None` while it still runs.
    ///
    /// # Errors
    ///
    /// Returns the Windows error if the underlying call fails.
    pub fn try_wait(&self) -> Result<Option<u32>> {
        // SAFETY: the handle is a live process handle with `SYNCHRONIZE` access.
        if unsafe { WaitForSingleObject(self.handle(), 0) } == WAIT_OBJECT_0 {
            self.exit_code().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Blocks until the process has exited and returns its exit code.
    ///
    /// # Errors
    ///
    /// Returns the Windows error if the underlying call fails.
    pub fn wait(&self) -> Result<u32> {
        // SAFETY: the handle is a live process handle with `SYNCHRONIZE` access; waiting on it
        // from several threads is fine.
        unsafe { WaitForSingleObject(self.handle(), INFINITE) };
        self.exit_code()
    }

    fn exit_code(&self) -> Result<u32> {
        let mut code = 0;
        // SAFETY: the handle has `PROCESS_QUERY_LIMITED_INFORMATION` access; `code` is valid.
        unsafe { GetExitCodeProcess(self.handle(), &raw mut code) }?;
        Ok(code)
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
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
    fn finds_children_by_parent_and_name() {
        let mut child = Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .unwrap();

        let mine = find_child_processes(std::process::id(), "ping.exe").unwrap();
        let other = find_child_processes(std::process::id(), "breakbar-not-running.exe").unwrap();

        terminate(child.as_raw_handle() as isize).unwrap();
        let _ = child.wait();
        assert!(mine.contains(&child.id()));
        assert!(other.is_empty());
    }

    #[test]
    fn a_process_can_be_opened_waited_for_and_stopped() {
        let mut child = Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .unwrap();
        let process = Process::open(child.id()).unwrap();
        assert_eq!(process.pid(), child.id());
        assert_eq!(process.try_wait().unwrap(), None);

        terminate(process.raw_handle()).unwrap();

        assert_eq!(process.wait().unwrap(), TERMINATED_EXIT_CODE);
        assert_eq!(process.try_wait().unwrap(), Some(TERMINATED_EXIT_CODE));
        let _ = child.wait();
    }

    #[test]
    fn finds_no_process_for_an_unused_name() {
        let pids = find_processes_by_name("breakbar-definitely-not-running.exe").unwrap();
        assert!(pids.is_empty());
    }
}
