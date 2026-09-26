//! The tray icon: left click and right-click menu.

use super::{
    AccountId, AccountState, App, ComponentHandle, LaunchQueue, MainWindow, Messages, Rc, RefCell,
    is_startable, launch_all, rows, toggle_account,
};

/// Left click / double-click on the tray icon: show and focus the main window.
pub(super) fn tray_activate(window: &MainWindow) -> impl FnMut() + 'static {
    let weak = window.as_weak();
    move || {
        if let Some(window) = weak.upgrade() {
            let _ = window.show();
        }
    }
}

/// Builds the tray's right-click menu fresh on every open, so it reflects current account state
/// (design hand-off screen 05): "Launch all" with a counter, one entry per account (locked ones
/// disabled), then "Open window" and "Quit".
pub(super) fn tray_menu(
    app: &Rc<RefCell<App>>,
    queue: &Rc<LaunchQueue>,
    window: &MainWindow,
) -> impl FnMut() -> Vec<bb_win::tray::MenuItem> + 'static {
    let app = Rc::clone(app);
    let queue = Rc::clone(queue);
    let weak = window.as_weak();
    move || {
        let Some(window) = weak.upgrade() else {
            return Vec::new();
        };
        let messages = window.global::<Messages>();
        let all_rows = rows(&window);
        let startable = all_rows
            .iter()
            .filter(|row| is_startable(row.state))
            .count();

        let mut items = vec![bb_win::tray::MenuItem::entry(
            messages.invoke_tray_launch_all(startable as i32),
            startable > 0,
            {
                let app = Rc::clone(&app);
                let queue = Rc::clone(&queue);
                let weak = weak.clone();
                move || {
                    if let Some(window) = weak.upgrade() {
                        launch_all(&window, &app, &queue);
                    }
                }
            },
        )];

        if !all_rows.is_empty() {
            items.push(bb_win::tray::MenuItem::separator());
            for row in all_rows {
                let id = AccountId(row.id as u32);
                let label = messages.invoke_tray_account(row.name.clone(), row.state);
                let locked = row.state == AccountState::Locked;
                items.push(bb_win::tray::MenuItem::entry(label, !locked, {
                    let app = Rc::clone(&app);
                    let queue = Rc::clone(&queue);
                    let weak = weak.clone();
                    move || {
                        if let Some(window) = weak.upgrade() {
                            toggle_account(&window, &app, &queue, id);
                        }
                    }
                }));
            }
        }

        items.push(bb_win::tray::MenuItem::separator());
        items.push(bb_win::tray::MenuItem::entry(
            messages.invoke_tray_open_window(),
            true,
            {
                let weak = weak.clone();
                move || {
                    if let Some(window) = weak.upgrade() {
                        let _ = window.show();
                    }
                }
            },
        ));
        items.push(bb_win::tray::MenuItem::entry(
            messages.invoke_tray_quit(),
            true,
            || {
                let _ = slint::quit_event_loop();
            },
        ));
        items
    }
}
