//! A native (non-Slint) popup menu shown at the current cursor position.
//!
//! Used wherever a Slint `PopupWindow` can't work: outside any window (the tray icon, which has
//! none at all) or inside one deliberately too small to contain the popup, which on this backend
//! clips an embedded popup to its owning window's bounds instead of letting it float over the
//! rest of the screen (the switcher overlay).

use std::io;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_DISABLED, MF_SEPARATOR, MF_STRING,
    PostMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR};

/// One entry of a native popup menu.
#[allow(missing_debug_implementations)]
pub enum MenuItem {
    Entry {
        text: String,
        enabled: bool,
        action: Box<dyn FnMut()>,
    },
    Separator,
}

impl MenuItem {
    pub fn entry(text: impl Into<String>, enabled: bool, action: impl FnMut() + 'static) -> Self {
        MenuItem::Entry {
            text: text.into(),
            enabled,
            action: Box::new(action),
        }
    }

    #[must_use]
    pub fn separator() -> Self {
        MenuItem::Separator
    }
}

/// Shows `items` as a native popup menu at the current cursor position, owned by `hwnd` (any
/// window on the calling thread, as a raw handle), and runs the chosen entry's action. Blocks until the menu
/// closes (`TrackPopupMenuEx` pumps its own nested loop internally, same as any native modal
/// Win32 UI).
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn show(hwnd: isize, mut items: Vec<MenuItem>) -> io::Result<()> {
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: the returned handle is only used below and destroyed before returning.
    let menu = unsafe { CreatePopupMenu() }.map_err(io::Error::other)?;

    for (index, item) in items.iter().enumerate() {
        let id = index + 1;
        // SAFETY: `menu` was just created above and is valid for the rest of this function.
        let _ = unsafe {
            match item {
                MenuItem::Entry { text, enabled, .. } => AppendMenuW(
                    menu,
                    if *enabled {
                        MF_STRING
                    } else {
                        MF_STRING | MF_DISABLED
                    },
                    id,
                    &HSTRING::from(text.as_str()),
                ),
                MenuItem::Separator => AppendMenuW(menu, MF_SEPARATOR, id, PCWSTR::null()),
            }
        };
    }

    let mut point = POINT::default();
    // SAFETY: `point` is a valid, appropriately sized out-pointer.
    unsafe {
        let _ = GetCursorPos(&raw mut point);
    }
    // SAFETY: documented Win32 pattern for context menus, so the menu gets keyboard focus and
    // closes correctly when the user clicks elsewhere.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    let flags = TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0 | TPM_NONOTIFY.0;
    // SAFETY: `menu` and `hwnd` are both valid; this blocks until the menu is dismissed.
    let selected = unsafe { TrackPopupMenuEx(menu, flags, point.x, point.y, hwnd, None) };
    // SAFETY: documented follow-up to the pattern above (a required no-op message).
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    }
    // SAFETY: `menu` is no longer needed after `TrackPopupMenuEx` returns.
    unsafe {
        let _ = DestroyMenu(menu);
    }

    let id = selected.0;
    if id > 0
        && let Some(MenuItem::Entry { action, .. }) = items.get_mut((id - 1) as usize)
    {
        action();
    }
    Ok(())
}
