//! Registry reads.

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::HSTRING;

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
