//! Reading the version resource of executables.

use std::ffi::c_void;
use std::path::Path;

use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::core::HSTRING;

/// Language/code page used when an executable declares no translation table (US English, Unicode).
const FALLBACK_TRANSLATION: &str = "040904b0";

/// Returns the `ProductName` from an executable's version resource.
#[must_use]
pub fn product_name(path: &Path) -> Option<String> {
    let data = version_info(path)?;
    let translation = translation(&data).unwrap_or_else(|| FALLBACK_TRANSLATION.to_owned());
    string_value(&data, &translation, "ProductName")
}

fn version_info(path: &Path) -> Option<Vec<u8>> {
    let path = HSTRING::from(path);
    // SAFETY: `path` is a valid wide string.
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is exactly `size` bytes large.
    unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()) }.ok()?;
    Some(data)
}

/// Returns the first language/code page pair as the hex string used in `StringFileInfo` paths.
fn translation(data: &[u8]) -> Option<String> {
    let (ptr, len) = query(data, &HSTRING::from(r"\VarFileInfo\Translation"))?;
    if len < 4 {
        return None;
    }
    // SAFETY: VerQueryValueW returned a pointer to at least `len` (>= 4) bytes inside `data`.
    let (lang, code_page) = unsafe {
        let words = ptr.cast::<u16>();
        (words.read_unaligned(), words.add(1).read_unaligned())
    };
    Some(format!("{lang:04x}{code_page:04x}"))
}

fn string_value(data: &[u8], translation: &str, name: &str) -> Option<String> {
    let key = HSTRING::from(format!("\\StringFileInfo\\{translation}\\{name}"));
    let (ptr, len) = query(data, &key)?;
    // SAFETY: for string values VerQueryValueW returns a pointer to `len` UTF-16 units inside `data`.
    let units: Vec<u16> = (0..len as usize)
        .map(|i| unsafe { ptr.cast::<u16>().add(i).read_unaligned() })
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16(&units).ok()
}

fn query(data: &[u8], key: &HSTRING) -> Option<(*const c_void, u32)> {
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: `data` is a version info block returned by GetFileVersionInfoW; the returned
    // pointer points into `data` and is only used while `data` is alive.
    let found = unsafe { VerQueryValueW(data.as_ptr().cast(), key, &raw mut ptr, &raw mut len) };
    (found.as_bool() && !ptr.is_null()).then_some((ptr.cast_const(), len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_product_name_of_system_binary() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let notepad = Path::new(&windir).join("System32").join("notepad.exe");
        let name = product_name(&notepad).expect("notepad.exe has a version resource");
        assert!(name.contains("Windows"), "unexpected product name {name:?}");
    }

    #[test]
    fn missing_file_has_no_product_name() {
        assert_eq!(product_name(Path::new(r"C:\does\not\exist.exe")), None);
    }
}
