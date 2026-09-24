//! Finding and closing the top-level windows of a process.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
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
pub fn has_visible_window(pid: u32, classes: &[&str]) -> Result<bool> {
    Ok(process_windows(pid)?
        .iter()
        .any(|window| window.visible && classes.contains(&window.class_name.as_str())))
}

/// Asks every top-level window of process `pid` to close (`WM_CLOSE`), the same as clicking its
/// close button, so the program can save its state. Returns how many windows were asked.
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

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` is the `&mut Vec<HWND>` passed by `process_windows`.
    let all = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    all.push(hwnd);
    true.into()
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: `pid` is valid for the call; an invalid window yields 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
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
    fn an_unused_pid_has_no_windows() {
        // PIDs are multiples of 4, so this one never exists.
        assert!(process_windows(3).unwrap().is_empty());
    }
}
