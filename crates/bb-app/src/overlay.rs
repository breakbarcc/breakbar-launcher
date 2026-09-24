//! The instance-switcher overlay: a small always-on-top bar for jumping between running clients
//! (design hand-off section 6). First pass: the bar itself at a fixed size, click-to-switch, and
//! the "+" menu to start accounts from it. Global hotkeys, the other two sizes, orientation,
//! opacity and the dedicated settings section come with a later refinement.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use bb_core::AccountId;
use bb_store::OverlayPosition;
use slint::{ComponentHandle, ModelRc, PhysicalPosition, Timer, TimerMode, VecModel};

use crate::gui::ui::{AccountState, MainWindow, OverlaySwitcher, StartableEntry, SwitcherEntry};
use crate::gui::{
    App, LaunchQueue, is_active, is_startable, launch_all, rows, start_account, stop_account,
};
use crate::launcher::{GAME_WINDOW_CLASSES, LaunchMode};

/// How often the overlay re-reads the account rows and the foreground window: cheap for the
/// handful of rows involved, and simpler than threading an explicit "something changed" signal
/// through every place `gui.rs` can change a row's state.
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

        window.on_activate({
            let weak = main_window.as_weak();
            move |id| {
                if let Some(main_window) = weak.upgrade() {
                    activate(&main_window, AccountId(id as u32));
                }
            }
        });

        window.on_stop({
            let weak = main_window.as_weak();
            move |id| {
                if let Some(main_window) = weak.upgrade() {
                    stop_account(&main_window, AccountId(id as u32));
                }
            }
        });

        window.on_show_in_launcher({
            let weak = main_window.as_weak();
            move |_id| {
                if let Some(main_window) = weak.upgrade() {
                    let _ = main_window.show();
                }
            }
        });

        window.on_launch({
            let app = Rc::clone(app);
            let queue = Rc::clone(queue);
            let weak = main_window.as_weak();
            move |id| {
                if let Some(main_window) = weak.upgrade() {
                    start_account(
                        &main_window,
                        &app,
                        &queue,
                        AccountId(id as u32),
                        LaunchMode::Play,
                    );
                }
            }
        });

        window.on_launch_all({
            let app = Rc::clone(app);
            let queue = Rc::clone(queue);
            let weak = main_window.as_weak();
            move || {
                if let Some(main_window) = weak.upgrade() {
                    launch_all(&main_window, &app, &queue);
                }
            }
        });

        window.on_open_launcher({
            let weak = main_window.as_weak();
            move || {
                if let Some(main_window) = weak.upgrade() {
                    let _ = main_window.show();
                }
            }
        });

        let last_position = Cell::new(app.borrow().config.overlay_position);
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
                poll(&overlay, &main_window);
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

/// Rebuilds the overlay's entries and startable list from `main_window`'s current rows, and
/// shows or hides it depending on whether anything is active.
fn poll(overlay: &OverlaySwitcher, main_window: &MainWindow) {
    let all_rows = rows(main_window);
    let foreground = bb_win::window::foreground_pid();

    let entries: Vec<SwitcherEntry> = all_rows
        .iter()
        .filter(|row| is_active(row.state))
        .map(|row| SwitcherEntry {
            id: row.id,
            name: row.name.clone(),
            active: row.handle != 0 && bb_win::window::pid_of(row.handle as isize) == foreground,
            starting: row.state == AccountState::Starting,
        })
        .collect();

    if entries.is_empty() {
        let _ = overlay.hide();
        return;
    }

    overlay.set_entries(ModelRc::new(VecModel::from(entries)));
    let startable: Vec<StartableEntry> = all_rows
        .iter()
        .filter(|row| !is_active(row.state))
        .map(|row| StartableEntry {
            id: row.id,
            name: row.name.clone(),
            enabled: row.state != AccountState::Locked,
        })
        .collect();
    overlay.set_startable(ModelRc::new(VecModel::from(startable)));
    overlay.set_launch_all_count(
        all_rows
            .iter()
            .filter(|row| is_startable(row.state))
            .count() as i32,
    );
    let _ = overlay.show();
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
