//! Finding and closing the top-level windows of a process.

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmSetWindowAttribute,
};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, PostMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE, SetForegroundWindow, ShowWindow, WM_CLOSE,
};
use windows::core::{BOOL, Result};

/// A top-level window and what is needed to tell windows apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub class_name: String,
    pub visible: bool,
}

/// Lists the top-level windows owned by process `pid`, in z-order.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn process_windows(pid: u32) -> Result<Vec<WindowInfo>> {
    let mut all: Vec<HWND> = Vec::new();
    // SAFETY: the callback only runs during this call, and `lparam` points to `all`, which
    // outlives it and is not otherwise accessed meanwhile.
    unsafe { EnumWindows(Some(collect), LPARAM(&raw mut all as isize)) }?;

    Ok(all
        .into_iter()
        .filter(|&hwnd| window_pid(hwnd) == pid)
        .map(|hwnd| WindowInfo {
            hwnd: hwnd.0 as isize,
            class_name: class_name(hwnd),
            // SAFETY: querying a possibly already destroyed window is harmless; it reports false.
            visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        })
        .collect())
}

/// Whether process `pid` shows a top-level window of one of `classes`.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn has_visible_window(pid: u32, classes: &[&str]) -> Result<bool> {
    Ok(process_windows(pid)?
        .iter()
        .any(|window| window.visible && classes.contains(&window.class_name.as_str())))
}

/// Asks every top-level window of process `pid` to close (`WM_CLOSE`), the same as clicking its
/// close button, so the program can save its state. Returns how many windows were asked.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn request_close(pid: u32) -> Result<usize> {
    let windows = process_windows(pid)?;
    for window in &windows {
        // SAFETY: posting to a window that has meanwhile been destroyed just fails.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(window.hwnd as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
    Ok(windows.len())
}

/// Restores (if minimized) and activates the first visible window of `pid` whose class is one of
/// `classes`. Returns whether such a window was found.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn activate_window(pid: u32, classes: &[&str]) -> Result<bool> {
    let Some(window) = process_windows(pid)?
        .into_iter()
        .find(|window| window.visible && classes.contains(&window.class_name.as_str()))
    else {
        return Ok(false);
    };
    let hwnd = HWND(window.hwnd as *mut _);
    // SAFETY: `hwnd` was just found by `process_windows` and is used only for these two calls.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(true)
}

/// PID owning the current foreground window, or 0 if there is none.
#[must_use]
pub fn foreground_pid() -> u32 {
    // SAFETY: takes no arguments; a missing foreground window yields a null handle, and
    // `window_pid` reports that as PID 0.
    window_pid(unsafe { GetForegroundWindow() })
}

/// PID of the process behind an open handle to it (such as a `Child`'s raw handle).
#[must_use]
pub fn pid_of(handle: isize) -> u32 {
    // SAFETY: `handle` is a still-open handle owned by the caller; this call only reads it.
    unsafe { GetProcessId(HANDLE(handle as *mut _)) }
}

/// Colors of a window's frame, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameColors {
    pub caption: u32,
    pub text: u32,
    pub border: u32,
}

/// The virtual screen, the bounding box of all monitors, as `(left, top, width, height)` in physical
/// pixels. Left and top are negative for a monitor left of or above the primary one.
#[must_use]
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    // SAFETY: `GetSystemMetrics` has no preconditions.
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Colors the native title bar and border of window `hwnd` to match the UI, in dark or light
/// mode. Custom colors need Windows 11; Windows 10 only follows the dark/light mode. Best-effort:
/// attributes the system doesn't know are ignored.
pub fn set_frame(hwnd: isize, dark: bool, colors: FrameColors) {
    let hwnd = HWND(hwnd as *mut _);
    let dark = BOOL::from(dark);
    // SAFETY: each call passes a pointer to a value that lives for the call, with its size.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            size_of::<BOOL>() as u32,
        );
        for (attribute, rgb) in [
            (DWMWA_CAPTION_COLOR, colors.caption),
            (DWMWA_TEXT_COLOR, colors.text),
            (DWMWA_BORDER_COLOR, colors.border),
        ] {
            let colorref = colorref(rgb);
            let _ = DwmSetWindowAttribute(
                hwnd,
                attribute,
                (&raw const colorref).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
}

/// `0xRRGGBB` → `COLORREF` (`0x00BBGGRR`).
fn colorref(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
    (b << 16) | (g << 8) | r
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` is the `&mut Vec<HWND>` passed by `process_windows`.
    let all = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    all.push(hwnd);
    true.into()
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: `pid` is valid for the call; an invalid window yields 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    pid
}

fn class_name(hwnd: HWND) -> String {
    // Window class names are limited to 256 characters.
    let mut buffer = [0u16; 257];
    // SAFETY: the buffer is valid for the call and its length is passed along.
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_without_windows_has_none() {
        let windows = process_windows(std::process::id()).unwrap();
        assert!(windows.iter().all(|window| !window.visible));
        assert!(!has_visible_window(std::process::id(), &["ArenaNet_Gr_Window_Class"]).unwrap());
    }

    #[test]
    fn colorref_swaps_red_and_blue() {
        assert_eq!(colorref(0x0d0f12), 0x120f0d);
    }

    #[test]
    fn an_unused_pid_has_no_windows() {
        // PIDs are multiples of 4, so this one never exists.
        assert!(process_windows(3).unwrap().is_empty());
    }
}
