//! The instance-switcher overlay: a small always-on-top bar with one numbered chip per account
//! (up to [`MAX_CHIPS`]), for jumping between running clients (design hand-off section 6). First
//! pass: global hotkeys, the other sizes, orientation, opacity and the dedicated settings section
//! come with a later refinement.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use bb_core::AccountId;
use bb_store::OverlayPosition;
use bb_win::menu::MenuItem;
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, Timer, TimerMode, VecModel};

use crate::gui::ui::{AccountState, MainWindow, Messages, OverlaySwitcher, SwitcherEntry};
use crate::gui::{App, LaunchQueue, is_active, native_handle, rows, start_account, stop_account};
use crate::launcher::{GAME_WINDOW_CLASSES, LaunchMode};

/// How often the overlay re-reads the account rows and the foreground window: cheap for the
/// handful of rows involved, and simpler than threading an explicit "something changed" signal
/// through every place `gui.rs` can change a row's state.
/// The bar shows at most this many accounts (the first ones in the launcher's order).
const MAX_CHIPS: usize = 4;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Owns the overlay window and keeps it alive. Dropping it destroys the window (and stops the
/// timer that drives it).
pub(crate) struct Overlay {
    _window: OverlaySwitcher,
    _poll: Timer,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Overlay")
    }
}

impl Overlay {
    /// A handle for applying settings (the theme) to the overlay window later.
    pub(crate) fn window(&self) -> slint::Weak<OverlaySwitcher> {
        self._window.as_weak()
    }

    /// Creates the overlay, restores its last saved position, and starts polling `main_window`'s
    /// account rows to keep it in sync. Does not show it yet — it only appears once an account is
    /// active (see the poll below).
    pub(crate) fn new(
        main_window: &MainWindow,
        app: &Rc<RefCell<App>>,
        queue: &Rc<LaunchQueue>,
    ) -> Result<Self, slint::PlatformError> {
        let window = OverlaySwitcher::new()?;

        if let Some(position) = app.borrow().config.overlay_position {
            window
                .window()
                .set_position(PhysicalPosition::new(position.x, position.y));
        }

        window.on_chip_clicked({
            let overlay = window.as_weak();
            let main = main_window.as_weak();
            let app = Rc::clone(app);
            let queue = Rc::clone(queue);
            move |id| {
                if let (Some(overlay), Some(main)) = (overlay.upgrade(), main.upgrade()) {
                    chip_clicked(&overlay, &main, &app, &queue, AccountId(id as u32), false);
                }
            }
        });

        window.on_context_menu({
            let overlay = window.as_weak();
            let main = main_window.as_weak();
            let app = Rc::clone(app);
            let queue = Rc::clone(queue);
            move |id| {
                if let (Some(overlay), Some(main)) = (overlay.upgrade(), main.upgrade()) {
                    chip_clicked(&overlay, &main, &app, &queue, AccountId(id as u32), true);
                }
            }
        });

        let last_position = Cell::new(app.borrow().config.overlay_position);
        let last_active = Cell::new(None);
        let poll_timer = Timer::default();
        poll_timer.start(TimerMode::Repeated, POLL_INTERVAL, {
            let app = Rc::clone(app);
            let overlay_weak = window.as_weak();
            let main_weak = main_window.as_weak();
            move || {
                let (Some(overlay), Some(main_window)) =
                    (overlay_weak.upgrade(), main_weak.upgrade())
                else {
                    return;
                };
                poll(&overlay, &main_window, &last_active);
                save_position_if_moved(&overlay, &app, &last_position);
            }
        });

        Ok(Self {
            _window: window,
            _poll: poll_timer,
        })
    }
}

/// Brings `id`'s game window to the foreground, if it has one.
fn activate(main_window: &MainWindow, id: AccountId) {
    let Some(row) = rows(main_window)
        .into_iter()
        .find(|row| row.id == id.0 as i32)
    else {
        return;
    };
    if row.handle == 0 {
        return;
    }
    let pid = bb_win::window::pid_of(row.handle as isize);
    let _ = bb_win::window::activate_window(pid, GAME_WINDOW_CLASSES);
}

/// Shows `items` as a native popup menu owned by the overlay window.
fn show_native_menu(overlay: &OverlaySwitcher, items: Vec<MenuItem>) {
    if let Some(hwnd) = native_handle(overlay.window()) {
        let _ = bb_win::menu::show(hwnd, items);
    }
}

/// Left or right click (`right_click`) on a chip. A running client is switched to on a
/// left click; a right click on it offers "Stop". An idle account offers "Start" either way.
/// Every menu starts with the account's name, since the chips themselves only show numbers.
fn chip_clicked(
    overlay: &OverlaySwitcher,
    main_window: &MainWindow,
    app: &Rc<RefCell<App>>,
    queue: &Rc<LaunchQueue>,
    id: AccountId,
    right_click: bool,
) {
    let Some(row) = rows(main_window)
        .into_iter()
        .find(|row| row.id == id.0 as i32)
    else {
        return;
    };
    let messages = main_window.global::<Messages>();
    let mut items = vec![
        MenuItem::entry(row.name.to_string(), false, || {}),
        MenuItem::separator(),
    ];
    match row.state {
        AccountState::Running if !right_click => {
            activate(main_window, id);
            return;
        }
        AccountState::Running => {
            let weak = main_window.as_weak();
            items.push(MenuItem::entry(
                messages.invoke_overlay_stop(),
                true,
                move || {
                    if let Some(main_window) = weak.upgrade() {
                        stop_account(&main_window, id);
                    }
                },
            ));
        }
        AccountState::Starting | AccountState::Stopping => return,
        state => {
            let weak = main_window.as_weak();
            let app = Rc::clone(app);
            let queue = Rc::clone(queue);
            items.push(MenuItem::entry(
                messages.invoke_overlay_start(),
                state != AccountState::Locked,
                move || {
                    if let Some(main_window) = weak.upgrade() {
                        start_account(&main_window, &app, &queue, id, LaunchMode::Play);
                    }
                },
            ));
        }
    }
    show_native_menu(overlay, items);
}

/// Rebuilds the overlay's chips from `main_window`'s current rows, and shows or hides it
/// depending on whether anything is active.
///
/// The highlighted chip is the last client that was in the foreground: it only moves when another
/// client takes focus, not when focus goes to something else (including this overlay), since the
/// user is still "in the game" then.
fn poll(overlay: &OverlaySwitcher, main_window: &MainWindow, last_active: &Cell<Option<i32>>) {
    let foreground = bb_win::window::foreground_pid();

    let all_rows = rows(main_window);
    let focused = all_rows.iter().find(|row| {
        row.handle != 0
            && is_active(row.state)
            && bb_win::window::pid_of(row.handle as isize) == foreground
    });
    if let Some(row) = focused {
        last_active.set(Some(row.id));
    } else if !all_rows
        .iter()
        .any(|row| Some(row.id) == last_active.get() && is_active(row.state))
    {
        last_active.set(None);
    }
    let entries: Vec<SwitcherEntry> = all_rows
        .iter()
        .take(MAX_CHIPS)
        .map(|row| SwitcherEntry {
            id: row.id,
            name: row.name.clone(),
            running: is_active(row.state),
            active: Some(row.id) == last_active.get() && is_active(row.state),
            starting: row.state == AccountState::Starting,
        })
        .collect();

    // Only shown while something runs: an all-idle bar would just be clutter over the desktop.
    if !all_rows.iter().any(|row| is_active(row.state)) {
        if overlay.window().is_visible() {
            let _ = overlay.hide();
        }
        return;
    }
    // Touch the window only when something actually changed: re-showing it or replacing the
    // chips on every tick would interrupt a drag in progress (the OS move loop keeps timers
    // running).
    let current: Vec<SwitcherEntry> = overlay.get_entries().iter().collect();
    if current != entries {
        overlay.set_entries(ModelRc::new(VecModel::from(entries)));
    }
    if !overlay.window().is_visible() {
        let _ = overlay.show();
    }
}

/// Persists the overlay's position once it settles somewhere new (dragged via its
/// `WindowMoveArea`). Best-effort, like the other background saves in this app: a failure here
/// isn't worth a toast over something this minor.
fn save_position_if_moved(
    overlay: &OverlaySwitcher,
    app: &RefCell<App>,
    last_position: &Cell<Option<OverlayPosition>>,
) {
    let current = overlay.window().position();
    let current = OverlayPosition {
        x: current.x,
        y: current.y,
    };
    if last_position.get() == Some(current) {
        return;
    }
    last_position.set(Some(current));
    app.borrow_mut().save_overlay_position(current);
}
