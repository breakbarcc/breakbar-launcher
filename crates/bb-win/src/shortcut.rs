//! Windows shortcuts (`.lnk` files).

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{HSTRING, Interface, Result};

use crate::com::ComApartment;

/// What a shortcut starts.
#[derive(Debug, Clone)]
pub struct Shortcut<'a> {
    pub target: &'a Path,
    pub arguments: &'a str,
    pub working_dir: &'a Path,
    /// Shown as the tooltip.
    pub description: &'a str,
    /// File whose first icon the shortcut shows.
    pub icon: &'a Path,
}

/// Writes `shortcut` to `link` (a `.lnk` path), replacing an existing file.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn create(link: &Path, shortcut: &Shortcut<'_>) -> Result<()> {
    let _com = ComApartment::enter()?;
    // SAFETY: COM is initialized for this thread by `_com`; every string passed lives until the
    // call it is passed to returns.
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shell_link.SetPath(&HSTRING::from(shortcut.target))?;
        shell_link.SetArguments(&HSTRING::from(shortcut.arguments))?;
        shell_link.SetWorkingDirectory(&HSTRING::from(shortcut.working_dir))?;
        shell_link.SetDescription(&HSTRING::from(shortcut.description))?;
        shell_link.SetIconLocation(&HSTRING::from(shortcut.icon), 0)?;
        shell_link
            .cast::<IPersistFile>()?
            .Save(&HSTRING::from(link), true)
    }
}

/// The current user's desktop folder.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn desktop_dir() -> Result<PathBuf> {
    // SAFETY: the returned buffer is freed with CoTaskMemFree after copying it.
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast_const().cast()));
        Ok(PathBuf::from(path?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_shortcut_file() {
        let dir = std::env::temp_dir().join(format!("breakbar-lnk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("Test.lnk");
        let target = std::env::current_exe().unwrap();

        create(
            &link,
            &Shortcut {
                target: &target,
                arguments: "--launch-id 1",
                working_dir: &dir,
                description: "Test",
                icon: &target,
            },
        )
        .unwrap();

        assert!(link.is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn desktop_exists() {
        assert!(desktop_dir().unwrap().is_dir());
    }
}
