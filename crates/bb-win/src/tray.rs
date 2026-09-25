//! A classic notification-area ("system tray") icon.
//!
//! It lives on its own hidden window rather than the main one, so it never has to share a
//! `WNDPROC` with the (winit-owned) main window. Both still get their messages delivered: on
//! Windows, `DispatchMessageW` looks up a message's window procedure by the target `HWND`'s own
//! window class, not by which library's loop happens to be pumping the thread's message queue —
//! so this works without running a message loop of its own, riding on whatever loop (here,
//! winit's) already pumps this thread.

use std::cell::RefCell;
use std::io;
use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, HICON, IDI_APPLICATION,
    IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, PostMessageW, RegisterClassW, WM_APP, WM_CONTEXTMENU,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_EX_LEFT, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR};

pub use crate::menu::MenuItem;

/// Not a message-only window (no `HWND_MESSAGE` parent): it needs to stay findable by
/// [`request_show`] from a second Breakbar process, which `FindWindowW` cannot see into the
/// message-only window class.
const CLASS_NAME: &str = "BreakbarTrayIcon";
/// Shell_NotifyIcon's callback message: carries the triggering mouse message in `lParam` (the
/// legacy, un-versioned packing — this never calls `NIM_SETVERSION`).
const CALLBACK_MESSAGE: u32 = WM_APP + 1;
/// Sent by [`request_show`] (a second Breakbar process asking the running one to show itself);
/// handled the same as a left click.
const SHOW_REQUEST_MESSAGE: u32 = WM_APP + 2;
/// Resource id of the icon embedded via `assets/breakbar.rc`.
const APP_ICON_ID: u16 = 1;

type MenuBuilder = Box<dyn FnMut() -> Vec<MenuItem>>;

thread_local! {
    static ON_ACTIVATE: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    static ON_MENU: RefCell<Option<MenuBuilder>> = const { RefCell::new(None) };
}

/// The tray icon. Dropping it removes the icon and destroys its hidden window.
#[derive(Debug)]
pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    /// Creates the icon, using the running executable's own embedded icon, with `tooltip`.
    ///
    /// `on_activate` fires for a left click or double-click (conventionally: show the window).
    /// `on_menu` is asked to build a fresh menu every time the icon is right-clicked or the
    /// context-menu key is pressed on it, so it can reflect current state; the chosen entry's
    /// `action` runs right after the menu closes.
    pub fn new(
        tooltip: &str,
        on_activate: impl FnMut() + 'static,
        on_menu: impl FnMut() -> Vec<MenuItem> + 'static,
    ) -> io::Result<Self> {
        ON_ACTIVATE.with(|cell| *cell.borrow_mut() = Some(Box::new(on_activate)));
        ON_MENU.with(|cell| *cell.borrow_mut() = Some(Box::new(on_menu)));

        let hwnd = create_hidden_window()?;
        if let Err(error) = add_icon(hwnd, tooltip) {
            // SAFETY: `hwnd` was just created above and hasn't been shared with anything else.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
        Ok(Self { hwnd })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let _ = remove_icon(self.hwnd);
        // SAFETY: `self.hwnd` was created by this module in `new` and is only used here and in
        // the window procedure, which stops receiving messages once the window is destroyed.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn create_hidden_window() -> io::Result<HWND> {
    static REGISTER: Once = Once::new();
    let class_name = HSTRING::from(CLASS_NAME);

    REGISTER.call_once(|| {
        // SAFETY: `GetModuleHandleW(None)` returns the current module; no pointers are stored.
        let instance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        // SAFETY: `class` is fully initialized and only needs to live for this call.
        unsafe {
            RegisterClassW(&class);
        }
    });

    // SAFETY: `GetModuleHandleW(None)` returns the current module; no pointers are stored.
    let instance = unsafe { GetModuleHandleW(None) }.map_err(io::Error::other)?;
    // SAFETY: all pointer arguments (`class_name`) outlive this call; the window is never shown.
    unsafe {
        CreateWindowExW(
            WS_EX_LEFT,
            &class_name,
            &class_name,
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(io::Error::other)
}

/// Asks an already-running Breakbar instance (found by its tray window's class) to show its main
/// window, for a second process launched while the first is already running (autostart, another
/// desktop shortcut, `breakbar-launcher.exe` started by hand again). Returns an error if no instance is
/// found running.
pub fn request_show() -> io::Result<()> {
    let class_name = HSTRING::from(CLASS_NAME);
    // SAFETY: `class_name` outlives this call; no window name filter is applied.
    let hwnd = unsafe { FindWindowW(&class_name, PCWSTR::null()) }.map_err(io::Error::other)?;
    // SAFETY: `hwnd` was just found above and the message carries no pointers.
    unsafe { PostMessageW(Some(hwnd), SHOW_REQUEST_MESSAGE, WPARAM(0), LPARAM(0)) }
        .map_err(io::Error::other)
}

fn app_icon() -> HICON {
    // SAFETY: `GetModuleHandleW(None)` returns the current module; no pointers are stored.
    let Ok(instance) = (unsafe { GetModuleHandleW(None) }) else {
        return HICON::default();
    };
    // SAFETY: `APP_ICON_ID` names a real `ICON` resource embedded via `assets/breakbar.rc`.
    let icon = unsafe {
        LoadImageW(
            Some(instance.into()),
            PCWSTR(APP_ICON_ID as usize as *const u16),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTCOLOR,
        )
    };
    match icon {
        Ok(icon) => HICON(icon.0),
        Err(_) => {
            // SAFETY: `IDI_APPLICATION` is a well-known system resource id.
            unsafe { windows::Win32::UI::WindowsAndMessaging::LoadIconW(None, IDI_APPLICATION) }
                .unwrap_or_default()
        }
    }
}

fn notify_icon_data(hwnd: HWND, tooltip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: CALLBACK_MESSAGE,
        hIcon: app_icon(),
        ..Default::default()
    };
    let tooltip: Vec<u16> = tooltip.encode_utf16().take(data.szTip.len() - 1).collect();
    data.szTip[..tooltip.len()].copy_from_slice(&tooltip);
    data
}

fn add_icon(hwnd: HWND, tooltip: &str) -> io::Result<()> {
    let data = notify_icon_data(hwnd, tooltip);
    // SAFETY: `data` is fully initialized and only needs to live for this call.
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        Ok(())
    } else {
        Err(io::Error::other("Shell_NotifyIcon(NIM_ADD) failed"))
    }
}

fn remove_icon(hwnd: HWND) -> io::Result<()> {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    // SAFETY: `data` is fully initialized and only needs to live for this call.
    if unsafe { Shell_NotifyIconW(NIM_DELETE, &data) }.as_bool() {
        Ok(())
    } else {
        Err(io::Error::other("Shell_NotifyIcon(NIM_DELETE) failed"))
    }
}

fn show_context_menu(hwnd: HWND) {
    let Some(items) = ON_MENU.with(|cell| cell.borrow_mut().as_mut().map(|build| build())) else {
        return;
    };
    let _ = crate::menu::show(hwnd.0 as isize, items);
}

fn activate() {
    ON_ACTIVATE.with(|cell| {
        if let Some(callback) = cell.borrow_mut().as_mut() {
            callback();
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == SHOW_REQUEST_MESSAGE {
        activate();
        return LRESULT(0);
    }
    if msg == CALLBACK_MESSAGE {
        match lparam.0 as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => activate(),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_context_menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    // SAFETY: forwarding any message this window procedure doesn't itself handle.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
