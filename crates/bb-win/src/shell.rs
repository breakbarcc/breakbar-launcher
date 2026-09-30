//! Opening URLs in the user's default browser.

use std::io;

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// Opens `url` in the user's default browser, the same as double-clicking a link.
///
/// # Errors
///
/// Returns an error if the shell has no handler for `url` (for example because it is not a
/// well-formed URL).
pub fn open_url(url: &str) -> io::Result<()> {
    let operation = HSTRING::from("open");
    let file = HSTRING::from(url);
    // SAFETY: `operation` and `file` are valid HSTRINGs kept alive for the call; no window
    // handle, extra parameters or working directory are needed to open a URL.
    let result = unsafe {
        ShellExecuteW(
            None,
            &operation,
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a pseudo-HINSTANCE; MSDN: a value greater than 32 means success,
    // anything else is one of a small set of SE_ERR_* codes, not a real system error.
    if (result.0 as usize) > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecuteW failed with code {}",
            result.0 as usize
        )))
    }
}
