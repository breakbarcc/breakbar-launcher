//! NTFS directory junctions (mount-point reparse points).
//!
//! Guild Wars 2 resolves its `%APPDATA%` through the Windows "known folder" API, not through the
//! `APPDATA` environment variable, so a child process's environment can't send it to another
//! folder (tested: only its embedded browser respected a changed `TEMP`; `Local.dat` still landed
//! in the real folder). A directory junction placed directly on `%APPDATA%\Guild Wars 2` does work,
//! and is what gw2launcher uses for the same reason (its `Windows/Symlink.cs::CreateJunction`, which
//! this implementation was cross-checked against, together with the field layout in
//! `[MS-FSCC] 2.1.2.4 Mount Point Reparse Buffer`). Unlike a symbolic link, a junction needs no
//! special privilege: no admin rights, no Developer Mode, only write access to the parent directory.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
use windows::core::{HSTRING, Result};

/// `IO_REPARSE_TAG_MOUNT_POINT`.
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
/// NT-namespace prefix required for a mount point's substitute name.
const NT_PATH_PREFIX: &str = r"\??\";

/// Turns the already-existing, empty directory `link` into a junction that resolves to `target`.
///
/// `target` does not need to exist yet — NTFS only resolves it when something actually opens a
/// path through `link`.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn create(link: &Path, target: &Path) -> Result<()> {
    let buffer = mount_point_buffer(target);

    let link_wide = HSTRING::from(link);
    // SAFETY: `link_wide` is a valid wide string; the returned handle is closed by the guard.
    let handle = unsafe {
        CreateFileW(
            &link_wide,
            GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }?;
    let handle = OwnedHandle(handle);

    // SAFETY: `handle` was just opened with write access to `link`; `buffer` is valid for
    // `buffer.len()` bytes, matching the length passed in; no output buffer is requested.
    unsafe {
        DeviceIoControl(
            handle.0,
            FSCTL_SET_REPARSE_POINT,
            Some(buffer.as_ptr().cast()),
            buffer.len() as u32,
            None,
            0,
            None,
            None,
        )
    }
}

/// Builds a Mount Point Reparse Buffer (`[MS-FSCC] 2.1.2.4`) with `target` as the substitute
/// name and no print name. Layout: 16-byte fixed header, then the substitute name (UTF-16, no
/// manual escaping needed since it's length-prefixed, not NUL-terminated-and-scanned) followed
/// by two trailing NUL `u16`s (a terminator for the substitute name, and an empty print name).
fn mount_point_buffer(target: &Path) -> Vec<u8> {
    let substitute_name = format!("{NT_PATH_PREFIX}{}\\", target.display());
    let name: Vec<u16> = substitute_name.encode_utf16().collect();
    let name_bytes = name.len() * 2;

    let mut buffer = vec![0u8; 16 + name_bytes + 4];
    buffer[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer[4..6].copy_from_slice(&((name_bytes + 12) as u16).to_le_bytes()); // ReparseDataLength
    // buffer[6..8] Reserved = 0
    // buffer[8..10] SubstituteNameOffset = 0
    buffer[10..12].copy_from_slice(&(name_bytes as u16).to_le_bytes()); // SubstituteNameLength
    buffer[12..14].copy_from_slice(&((name_bytes + 2) as u16).to_le_bytes()); // PrintNameOffset
    // buffer[14..16] PrintNameLength = 0
    for (i, unit) in name.iter().enumerate() {
        buffer[16 + i * 2..16 + i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    // The trailing 4 bytes (2 UTF-16 NULs) stay zero: the substitute name's terminator and an
    // empty, zero-length print name.
    buffer
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a handle this guard owns exclusively.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "breakbar-junction-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn junction_resolves_to_its_target() {
        let target = temp_dir("target");
        let link = temp_dir("link");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&link).unwrap();
        fs::write(target.join("marker.txt"), b"hello").unwrap();

        create(&link, &target).unwrap();

        // If the junction resolves correctly, reading through `link` sees `target`'s content.
        let content = fs::read_to_string(link.join("marker.txt")).unwrap();
        assert_eq!(content, "hello");

        fs::remove_dir(&link).unwrap(); // removes the junction itself, not `target`'s content
        assert!(
            target.join("marker.txt").is_file(),
            "target must survive removing the link"
        );
        fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn retargeting_an_existing_junction_switches_to_the_new_target() {
        let target_a = temp_dir("retarget-a");
        let target_b = temp_dir("retarget-b");
        let link = temp_dir("retarget-link");
        fs::create_dir_all(&target_a).unwrap();
        fs::create_dir_all(&target_b).unwrap();
        fs::create_dir_all(&link).unwrap();
        fs::write(target_a.join("which.txt"), b"a").unwrap();
        fs::write(target_b.join("which.txt"), b"b").unwrap();

        create(&link, &target_a).unwrap();
        assert_eq!(fs::read_to_string(link.join("which.txt")).unwrap(), "a");

        create(&link, &target_b).unwrap();
        assert_eq!(fs::read_to_string(link.join("which.txt")).unwrap(), "b");

        fs::remove_dir(&link).unwrap();
        fs::remove_dir_all(&target_a).unwrap();
        fs::remove_dir_all(&target_b).unwrap();
    }
}
