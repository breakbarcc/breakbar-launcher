//! Native file dialogs.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW};
use windows::core::{HRESULT, HSTRING, PCWSTR, Result};

use crate::com::ComApartment;

/// A file type filter: display name and pattern(s), e.g. `("Programs", "*.exe")`.
pub type Filter<'a> = (&'a str, &'a str);

/// Shows the system "open file" dialog, modal to `owner`.
///
/// Returns `Ok(None)` if the user cancelled.
pub fn open_file(
    owner: Option<isize>,
    title: &str,
    filters: &[Filter<'_>],
    initial_dir: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let _com = ComApartment::enter()?;

    // Keep the wide strings alive while the dialog references them.
    let filter_strings: Vec<(HSTRING, HSTRING)> = filters
        .iter()
        .map(|(name, spec)| (HSTRING::from(*name), HSTRING::from(*spec)))
        .collect();
    let specs: Vec<COMDLG_FILTERSPEC> = filter_strings
        .iter()
        .map(|(name, spec)| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(spec.as_ptr()),
        })
        .collect();

    // SAFETY: COM is initialized for this thread by `_com`; all pointers passed to the dialog
    // (title, filter strings, shell item) stay alive until `Show` returns.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
        }
        dialog.SetOptions(dialog.GetOptions()? | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)?;
        if let Some(dir) = initial_dir.filter(|dir| dir.is_dir()) {
            let folder: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dir), None)?;
            dialog.SetFolder(&folder)?;
        }

        let owner = owner.map(|hwnd| HWND(hwnd as *mut _));
        match dialog.Show(owner) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }

        let raw = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast_const().cast()));
        Ok(Some(PathBuf::from(path?)))
    }
}

/// Shows a blocking error message box. Only for when Breakbar has no window of its own to show
/// the error in (e.g. starting an account from a desktop shortcut).
pub fn error_box(title: &str, text: &str) {
    // SAFETY: both strings outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}
