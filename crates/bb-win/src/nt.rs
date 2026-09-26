//! Closing a specific named handle inside another process.
//!
//! This is how Breakbar removes Guild Wars 2's single-instance guard (see
//! [`crate::mutex::kill_gw2_mutex`]) without touching the process otherwise: GW2 keeps running,
//! it just loses the one handle that made it refuse a second instance.
//!
//! The building blocks are undocumented NT internals (not part of any public Windows API, and
//! not covered by `windows-rs`'s metadata), so this module declares its own raw bindings via
//! [`windows::core::link!`] and hand-written struct layouts. Every struct and field here was
//! cross-checked against System Informer's public `phnt` headers
//! (<https://github.com/winsiderss/systeminformer>), which is the standard reference other
//! GW2 multi-launchers (and tools like Process Hacker/Explorer) rely on for the same structs.
//!
//! # Approach
//!
//! 1. Learn this boot's `ObjectTypeIndex` for mutex ("Mutant") objects by querying a mutex we
//!    just created ourselves ([`mutant_type_index`]). This never touches another process, so it
//!    cannot hang.
//! 2. Snapshot the target process's handle table in one call
//!    (`NtQueryInformationProcess(ProcessHandleInformation)`), rather than the whole system's
//!    handle table (`NtQuerySystemInformation(SystemHandleInformation)`, the classic approach
//!    older multi-launchers use, which scans every process on the machine).
//! 3. Skip everything whose `ObjectTypeIndex` doesn't match the mutex type index — a plain
//!    integer comparison, no syscall. In particular, this means we never call `NtQueryObject` on
//!    a handle we haven't already identified as a mutex: querying the *name* of some handle
//!    types (most notably synchronous named pipes) is known to block indefinitely, which is
//!    exactly the failure mode this ordering avoids.
//! 4. For the few handles that do match, duplicate them into our own process (safe: duplicating
//!    never blocks) and confirm the name via `NtQueryObject(ObjectNameInformation)` on our own
//!    copy.
//! 5. On a match, close the *original* handle in the target process with
//!    `DuplicateHandle(..., DUPLICATE_CLOSE_SOURCE)`.

use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_CLOSE_SOURCE, DuplicateHandle, HANDLE, NTSTATUS, UNICODE_STRING,
};
use windows::Win32::Security::GENERIC_MAPPING;
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION,
};
use windows::core::{Error, HRESULT, HSTRING, Result, link};

/// `PROCESSINFOCLASS::ProcessHandleInformation` (undocumented, stable since Windows 8).
const PROCESS_HANDLE_INFORMATION: u32 = 51;
/// `OBJECT_INFORMATION_CLASS::ObjectTypeInformation`.
const OBJECT_TYPE_INFORMATION_CLASS: u32 = 2;
/// `OBJECT_INFORMATION_CLASS::ObjectNameInformation`.
const OBJECT_NAME_INFORMATION_CLASS: u32 = 1;
/// `STATUS_INFO_LENGTH_MISMATCH`: the supplied buffer was too small.
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
/// Upper bound on how large a handle-table snapshot we're willing to allocate for.
const MAX_HANDLE_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
/// Fixed-size buffer for `NtQueryObject`: both the type name and the object's full path are a
/// few dozen characters at most, so this comfortably covers the struct plus trailing string.
const QUERY_OBJECT_BUFFER_BYTES: usize = 4096;

// SAFETY (link!): each of these mirrors a documented-by-convention ntdll export (part of the
// stable, if undocumented, NT syscall ABI used by every process on the system); the signatures
// below match the reference headers cited in the module doc comment.
link!("ntdll.dll" "system" fn NtQueryInformationProcess(
    processhandle: HANDLE,
    processinformationclass: u32,
    processinformation: *mut core::ffi::c_void,
    processinformationlength: u32,
    returnlength: *mut u32,
) -> NTSTATUS);
link!("ntdll.dll" "system" fn NtQueryObject(
    handle: HANDLE,
    objectinformationclass: u32,
    objectinformation: *mut core::ffi::c_void,
    objectinformationlength: u32,
    returnlength: *mut u32,
) -> NTSTATUS);

/// Mirrors `PROCESS_HANDLE_TABLE_ENTRY_INFO` (`ntpsapi.h`).
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessHandleTableEntryInfo {
    handle_value: HANDLE,
    handle_count: usize,
    pointer_count: usize,
    granted_access: u32,
    object_type_index: u32,
    handle_attributes: u32,
    reserved: u32,
}

/// Mirrors the fixed header of `PROCESS_HANDLE_SNAPSHOT_INFORMATION` (`ntpsapi.h`); the entries
/// making up its flexible `Handles[]` array follow immediately after in memory.
#[repr(C)]
struct ProcessHandleSnapshotInformation {
    number_of_handles: usize,
    _reserved: usize,
}

/// Mirrors `OBJECT_TYPE_INFORMATION` (`ntobapi.h`). Only `type_index` is used; the struct is
/// still declared in full so Rust computes the same field offsets as the real one.
#[repr(C)]
#[derive(Clone, Copy)]
struct ObjectTypeInformation {
    type_name: UNICODE_STRING,
    total_number_of_objects: u32,
    total_number_of_handles: u32,
    total_paged_pool_usage: u32,
    total_non_paged_pool_usage: u32,
    total_name_pool_usage: u32,
    total_handle_table_usage: u32,
    high_water_number_of_objects: u32,
    high_water_number_of_handles: u32,
    high_water_paged_pool_usage: u32,
    high_water_non_paged_pool_usage: u32,
    high_water_name_pool_usage: u32,
    high_water_handle_table_usage: u32,
    invalid_attributes: u32,
    generic_mapping: GENERIC_MAPPING,
    valid_access_mask: u32,
    security_required: u8,
    maintain_handle_count: u8,
    type_index: u8,
    reserved_byte: i8,
    pool_type: u32,
    default_paged_pool_charge: u32,
    default_non_paged_pool_charge: u32,
}

/// Mirrors `OBJECT_NAME_INFORMATION` (`ntobapi.h`).
#[repr(C)]
struct ObjectNameInformation {
    name: UNICODE_STRING,
}

/// Closes Breakbar's own duplicate/handle when dropped, so an early `?` return never leaks it.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is a handle this guard owns exclusively; nothing else closes it.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn nt_error(status: NTSTATUS, context: &str) -> Error {
    // `HRESULT_FROM_NT`: NT status codes are mapped into the HRESULT space by setting the
    // "customer" N bit (0x1000_0000), the standard, documented conversion.
    Error::new(HRESULT(status.0 | 0x1000_0000), context)
}

/// Finds a handle named `object_name` among `pid`'s open mutex handles and closes it there.
///
/// Returns `Ok(true)` if a match was found and closed, `Ok(false)` if `pid`'s handle table has
/// no mutex with that name (nothing to do). `pid` should be a process Breakbar itself started;
/// this function needs `PROCESS_QUERY_INFORMATION | PROCESS_DUP_HANDLE` access to it, which
/// Windows only grants for processes running as the same user (no elevation required).
pub fn close_named_mutex_in_process(pid: u32, object_name: &str) -> Result<bool> {
    let mutant_type_index = mutant_type_index()?;

    // SAFETY: `pid` is validated by OpenProcess itself; no other preconditions.
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_DUP_HANDLE, false, pid) }?;
    let process = OwnedHandle(process);

    let snapshot = query_process_handles(process.0)?;
    // SAFETY: `snapshot` was filled by a successful NtQueryInformationProcess(
    // ProcessHandleInformation) call, so it holds a valid header followed by that many
    // (defensively re-clamped below) PROCESS_HANDLE_TABLE_ENTRY_INFO entries.
    let entries = unsafe { handle_entries(&snapshot) };

    for entry in entries {
        if entry.object_type_index != mutant_type_index {
            continue;
        }

        // Only mutex-type handles reach here, so querying their name is safe (see module docs).
        let Ok(duplicate) = duplicate_for_local_query(process.0, entry.handle_value) else {
            continue;
        };
        let duplicate = OwnedHandle(duplicate);

        // `NtQueryObject` returns the object's full kernel path, e.g.
        // `\Sessions\1\BaseNamedObjects\<name>` (or `\BaseNamedObjects\<name>` in session 0),
        // not the bare name passed to `CreateMutexW`/`OpenMutexW` — those resolve it against
        // the calling process's session automatically. Match on the last path component.
        // A mutex whose name can't be read (too long for the buffer, say) is simply not ours:
        // it must not stop the search for the one that is.
        let matches = object_name_of(duplicate.0)
            .ok()
            .flatten()
            .is_some_and(|full_name| last_path_component(&full_name) == object_name);
        if !matches {
            continue;
        }

        // Close the *original* handle inside the target process (not our duplicate above).
        // SAFETY: `process.0` and `entry.handle_value` both remain valid for this call, which
        // is made while `process` is still held (its guard hasn't dropped yet).
        unsafe {
            DuplicateHandle(
                process.0,
                entry.handle_value,
                HANDLE::default(),
                std::ptr::null_mut(),
                0,
                false,
                DUPLICATE_CLOSE_SOURCE,
            )
        }?;
        return Ok(true);
    }

    Ok(false)
}

/// Learns this boot's `ObjectTypeIndex` for mutexes from a mutex Breakbar creates and owns
/// itself, so the lookup never touches a handle belonging to another process.
fn mutant_type_index() -> Result<u32> {
    let name = HSTRING::from(format!("Breakbar-TypeProbe-{}", std::process::id()));
    // SAFETY: `name` is a valid wide string that outlives the call.
    let handle = unsafe { CreateMutexW(None, false, &name) }?;
    let handle = OwnedHandle(handle);

    let mut buffer = aligned_buffer(QUERY_OBJECT_BUFFER_BYTES);
    let mut used = 0u32;
    // SAFETY: `buffer` is `QUERY_OBJECT_BUFFER_BYTES` long, matching the length passed in;
    // `handle.0` is a live handle we just created.
    let status = unsafe {
        NtQueryObject(
            handle.0,
            OBJECT_TYPE_INFORMATION_CLASS,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<u64>()) as u32,
            &mut used,
        )
    };
    if status.0 < 0 {
        return Err(nt_error(status, "NtQueryObject(ObjectTypeInformation)"));
    }

    // SAFETY: the successful call above filled at least `size_of::<ObjectTypeInformation>()`
    // bytes of `buffer` (querying that class always writes the fixed struct before any trailing
    // name data), so reading the struct's own fields back out is in-bounds.
    let info = unsafe { &*buffer.as_ptr().cast::<ObjectTypeInformation>() };
    Ok(u32::from(info.type_index))
}

/// Snapshots `process`'s handle table, growing the buffer until it fits.
fn query_process_handles(process: HANDLE) -> Result<Vec<u64>> {
    let mut size = 64 * 1024;
    loop {
        let mut buffer = aligned_buffer(size);
        let mut used = 0u32;
        // SAFETY: `buffer` is `size` bytes long, matching the length passed in; `process` is a
        // live handle with `PROCESS_QUERY_INFORMATION` access.
        let status = unsafe {
            NtQueryInformationProcess(
                process,
                PROCESS_HANDLE_INFORMATION,
                buffer.as_mut_ptr().cast(),
                size as u32,
                &mut used,
            )
        };
        if status.0 >= 0 {
            return Ok(buffer);
        }
        if status.0 == STATUS_INFO_LENGTH_MISMATCH && size < MAX_HANDLE_SNAPSHOT_BYTES {
            size = (size * 2).min(MAX_HANDLE_SNAPSHOT_BYTES);
            continue;
        }
        return Err(nt_error(
            status,
            "NtQueryInformationProcess(ProcessHandleInformation)",
        ));
    }
}

/// Reads the handle-table entries out of a buffer filled by [`query_process_handles`].
///
/// # Safety
///
/// `snapshot` must have been filled by a successful `NtQueryInformationProcess(
/// ProcessHandleInformation)` call.
unsafe fn handle_entries(snapshot: &[u64]) -> &[ProcessHandleTableEntryInfo] {
    // `snapshot` is a `u64` buffer, so it is aligned for both structs (their alignment is 8).
    let bytes = std::mem::size_of_val(snapshot);
    let base = snapshot.as_ptr().cast::<u8>();
    let header_len = size_of::<ProcessHandleSnapshotInformation>();
    if bytes < header_len {
        return &[];
    }
    // SAFETY: checked above that `snapshot` holds at least a full header, and it is aligned.
    let header = unsafe { &*base.cast::<ProcessHandleSnapshotInformation>() };

    let entry_len = size_of::<ProcessHandleTableEntryInfo>();
    let available_entries = (bytes - header_len) / entry_len;
    // Defensive: trust the byte length we actually allocated over the kernel-reported count.
    let count = header.number_of_handles.min(available_entries);

    // SAFETY: `count` was just clamped to the number of whole entries that fit within
    // `snapshot`; the entries begin immediately after the 16-byte header per the NT struct
    // layout, which keeps them 8-byte aligned.
    unsafe {
        std::slice::from_raw_parts(
            base.add(header_len).cast::<ProcessHandleTableEntryInfo>(),
            count,
        )
    }
}

/// A zeroed buffer of at least `bytes` bytes that is aligned for the NT structs read out of it
/// (a `Vec<u8>` is only guaranteed to be 1-byte aligned).
fn aligned_buffer(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(size_of::<u64>())]
}

/// Duplicates `handle` (owned by `source_process`) into Breakbar's own process so it can be
/// queried locally, without ever touching the source handle beyond this read-only duplication.
fn duplicate_for_local_query(source_process: HANDLE, handle: HANDLE) -> Result<HANDLE> {
    // The documented, fixed value of `GetCurrentProcess()` — a pseudo-handle that never needs
    // closing and is valid in every process.
    const CURRENT_PROCESS: HANDLE = HANDLE(-1isize as *mut core::ffi::c_void);

    let mut duplicate = HANDLE::default();
    // SAFETY: `source_process` and `handle` are both valid for the duration of this call.
    unsafe {
        DuplicateHandle(
            source_process,
            handle,
            CURRENT_PROCESS,
            &mut duplicate,
            0,
            false,
            windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
        )
    }?;
    Ok(duplicate)
}

/// Reads the name of a locally-owned object handle, if it has one.
fn object_name_of(handle: HANDLE) -> Result<Option<String>> {
    let mut buffer = aligned_buffer(QUERY_OBJECT_BUFFER_BYTES);
    let mut used = 0u32;
    // SAFETY: `buffer` is `QUERY_OBJECT_BUFFER_BYTES` long, matching the length passed in;
    // `handle` is a handle this process owns (a local duplicate), so querying its name cannot
    // block on another process (see module docs on the pipe-hang pitfall).
    let status = unsafe {
        NtQueryObject(
            handle,
            OBJECT_NAME_INFORMATION_CLASS,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<u64>()) as u32,
            &mut used,
        )
    };
    if status.0 < 0 {
        return Err(nt_error(status, "NtQueryObject(ObjectNameInformation)"));
    }

    // SAFETY: the successful call above filled at least `size_of::<ObjectNameInformation>()`
    // bytes of `buffer`.
    let info = unsafe { &*buffer.as_ptr().cast::<ObjectNameInformation>() };
    Ok(unicode_string_to_string(&info.name))
}

/// Returns the part of an NT object path after its last `\`, or the whole string if it has none.
fn last_path_component(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// Converts an in-buffer `UNICODE_STRING` (as filled by `NtQueryObject`) into an owned `String`.
fn unicode_string_to_string(value: &UNICODE_STRING) -> Option<String> {
    if value.Buffer.is_null() || value.Length == 0 {
        return None;
    }
    let len_utf16 = (value.Length / 2) as usize;
    // SAFETY: `NtQueryObject` filled `value.Buffer` with `value.Length` valid bytes inside the
    // same buffer this `UNICODE_STRING` was read from, which is still alive at this call site.
    let wide = unsafe { std::slice::from_raw_parts(value.Buffer.0, len_utf16) };
    Some(String::from_utf16_lossy(wide))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn mutant_type_index_is_stable_across_calls() {
        // Not a fixed, documented constant (it's assigned by the kernel per boot), but it must
        // be internally consistent for our filtering to make sense.
        assert_eq!(mutant_type_index().unwrap(), mutant_type_index().unwrap());
    }

    /// End-to-end: a helper process creates a named mutex and blocks; this test closes that
    /// exact handle from the outside, the same way `close_named_mutex_in_process` closes GW2's.
    #[test]
    fn closes_a_named_mutex_in_another_process() {
        let name = format!("Breakbar-Test-Kill-{}", std::process::id());
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let powershell = std::path::Path::new(&windir)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");

        // The helper creates the mutex, signals readiness by *not* exiting, and holds it until
        // killed; we detect "created" by polling for the handle to appear.
        let mut child = Command::new(&powershell)
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$m = New-Object Threading.Mutex($false,'{name}'); Start-Sleep -Seconds 30"
                ),
            ])
            .spawn()
            .expect("powershell.exe should be available on every Windows installation");

        let found = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            crate::mutex::mutex_exists(&name).unwrap_or(false)
        });
        assert!(found, "helper process never created the mutex");

        let closed = close_named_mutex_in_process(child.id(), &name).unwrap();
        assert!(
            closed,
            "expected to find and close the helper's mutex handle"
        );
        assert!(!crate::mutex::mutex_exists(&name).unwrap());

        let _ = child.kill();
        let _ = child.wait();
    }
}
