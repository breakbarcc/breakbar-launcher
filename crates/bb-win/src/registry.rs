//! Registry reads.

use std::io;

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

/// Registry hive to read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    CurrentUser,
    LocalMachine,
}

impl Root {
    fn hkey(self) -> HKEY {
        match self {
            Root::CurrentUser => HKEY_CURRENT_USER,
            Root::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

/// Reads a string value (`REG_SZ`, or `REG_EXPAND_SZ` with variables expanded).
///
/// Returns `None` if the key or value does not exist or is not a string.
pub fn read_string(root: Root, subkey: &str, value: &str) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);

    let mut size = 0u32;
    // SAFETY: all string arguments are valid wide strings; only the size is queried.
    let status = unsafe {
        RegGetValueW(
            root.hkey(),
            &subkey,
            &value,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: `buffer` holds at least `size` bytes, which is passed as its capacity.
    let status = unsafe {
        RegGetValueW(
            root.hkey(),
            &subkey,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }

    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end]).ok()
}

/// Writes a `REG_SZ` value, creating `subkey` if it doesn't exist yet.
pub fn write_string(root: Root, subkey: &str, value: &str, data: &str) -> Result<(), io::Error> {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    let mut data: Vec<u16> = data.encode_utf16().chain([0]).collect();
    // SAFETY: reinterpreting the u16 buffer as bytes for the byte-oriented registry API; the
    // slice stays within `data`'s allocation and doesn't outlive it.
    let data =
        unsafe { std::slice::from_raw_parts(data.as_mut_ptr().cast::<u8>(), data.len() * 2) };

    let mut hkey = HKEY::default();
    // SAFETY: `hkey` receives the opened/created key; no other pointers are stored past this call.
    let status = unsafe {
        RegCreateKeyExW(
            root.hkey(),
            &subkey,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if status.is_err() {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }

    // SAFETY: `hkey` was just opened above and is closed below regardless of the outcome.
    let status = unsafe { RegSetValueExW(hkey, &value, None, REG_SZ, Some(data)) };
    // SAFETY: `hkey` is a valid, still-open key handle.
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    if status.is_err() {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    Ok(())
}

/// Deletes a value. Missing keys or values are not an error.
pub fn delete_value(root: Root, subkey: &str, value: &str) -> Result<(), io::Error> {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    // SAFETY: all arguments are valid wide strings.
    let status = unsafe { RegDeleteKeyValueW(root.hkey(), &subkey, &value) };
    if status.is_err() && status != ERROR_FILE_NOT_FOUND {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_existing_value() {
        let root = read_string(
            Root::LocalMachine,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "SystemRoot",
        )
        .expect("SystemRoot should exist on every Windows installation");
        assert!(root.to_ascii_lowercase().ends_with("windows"));
    }

    #[test]
    fn missing_value_is_none() {
        assert_eq!(
            read_string(Root::CurrentUser, r"Software\Breakbar-Test-Missing", "Nope"),
            None
        );
    }
}
