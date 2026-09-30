//! "Delete all data": undoes everything Breakbar ever put on disk or in the registry, since there
//! is no installer and therefore no uninstaller either.

use super::{
    App, ComponentHandle, MainWindow, Messages, Rc, RefCell, ToastKind, is_active, push_toast, rows,
};
use crate::{profile_link, steam_setup};

/// Connects the window's callback for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_request_delete_all_data({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                request(&window);
            }
        }
    });

    window.on_confirm_delete_all({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                delete_all_data(&window, &app);
            }
        }
    });
}

/// Opens the confirmation dialog, unless an account is still active: deleting profile folders out
/// from under a running client would leave it in an undefined state.
fn request(window: &MainWindow) {
    if rows(window).iter().any(|row| is_active(row.state)) {
        let messages = window.global::<Messages>();
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_reset_failed_title(),
            messages.invoke_reset_running(),
        );
        return;
    }
    window.set_confirm_delete_all_open(true);
}

/// Removes every trace of Breakbar: the desktop shortcuts and the start-with-Windows entry (both
/// best-effort, neither being critical enough to stop the rest over), then the Steam link and the
/// real `%APPDATA%\Guild Wars 2` link (restoring its data first), then the profiles and finally the
/// config, in that order, since a later step would otherwise delete what an earlier one still needs
/// to find. Quits Breakbar on full success, since none of its in-memory state is valid anymore;
/// reports what went wrong and leaves Breakbar running otherwise, so the rest can be retried or
/// cleaned up by hand.
fn delete_all_data(window: &MainWindow, app: &RefCell<App>) {
    let (gw2_path, accounts) = {
        let app = app.borrow();
        (
            app.config.gw2_path.clone(),
            app.config
                .accounts
                .iter()
                .map(|account| account.name.clone())
                .collect::<Vec<_>>(),
        )
    };

    if let Ok(desktop) = bb_win::shortcut::desktop_dir() {
        for name in &accounts {
            let file = format!("{} (Breakbar).lnk", super::accounts::file_name_safe(name));
            let _ = std::fs::remove_file(desktop.join(file));
        }
    }
    let _ = bb_win::autostart::disable(super::APP_NAME);

    let mut errors = Vec::new();
    if let Some(gw2_path) = &gw2_path
        && let Err(error) = steam_setup::remove_link(gw2_path)
    {
        errors.push(error.to_string());
    }
    if let Err(error) = profile_link::unlink() {
        errors.push(error.to_string());
    }
    if let Err(error) = bb_store::delete_all_profiles() {
        errors.push(error.to_string());
    }
    if let Err(error) = bb_store::delete_all_config() {
        errors.push(error.to_string());
    }

    if errors.is_empty() {
        let _ = slint::quit_event_loop();
        return;
    }
    let messages = window.global::<Messages>();
    push_toast(
        window,
        ToastKind::Error,
        messages.invoke_reset_failed_title(),
        errors.join("\n\n").into(),
    );
}
