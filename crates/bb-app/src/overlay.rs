//! The instance-switcher overlay: a small always-on-top bar with one numbered chip per account
//! (up to [`MAX_CHIPS`]), for jumping between running clients (design hand-off section 6). First
//! pass: global hotkeys, the other sizes, orientation, opacity and the dedicated settings section
//! come with a later refinement.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use bb_core::AccountId;
use bb_store::{OverlayPosition, OverlaySettings};
use bb_win::menu::MenuItem;
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, Timer, TimerMode, VecModel};

use crate::gui::ui::{AccountState, MainWindow, Messages, OverlaySwitcher, SwitcherEntry};
use crate::gui::{App, LaunchQueue, is_active, native_handle, rows, start_account, stop_account};
use crate::launcher::{GAME_WINDOW_CLASSES, LaunchMode};

/// How often the overlay re-reads the account rows and the foreground window: cheap for the
/// handful of rows involved, and simpler than threading an explicit "something changed" signal
/// through every place `gui` can change a row's state.
/// The bar shows at most this many accounts (the first ones in the launcher's order).
const MAX_CHIPS: usize = 4;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Owns the overlay window and keeps it alive. Dropping it destroys the window (and stops the
/// timer that drives it).
pub(crate) struct Overlay {
    window: OverlaySwitcher,
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
        self.window.as_weak()
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

        // A position that is not on any monitor any more (a monitor was unplugged, or Windows
        // parked a hidden window at -32000) would leave the bar unreachable.
        if let Some(position) = app
            .borrow()
            .config
            .overlay_position
            .filter(|position| on_screen(position.x, position.y))
        {
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

        let mut position_saver = PositionSaver::new(app.borrow().config.overlay_position);
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
                let settings = app.borrow().config.overlay;
                poll(&overlay, &main_window, &last_active, settings);
                if let Some(position) = position_saver.observe(current_position(&overlay)) {
                    app.borrow_mut().save_overlay_position(position);
                }
            }
        });

        Ok(Self {
            window,
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
fn poll(
    overlay: &OverlaySwitcher,
    main_window: &MainWindow,
    last_active: &Cell<Option<i32>>,
    settings: OverlaySettings,
) {
    overlay.set_locked(settings.lock_position);
    overlay.set_idle_opacity(f32::from(settings.opacity_percent()) / 100.0);

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

    // Shown when enabled and there is something to switch between; by default only while a client
    // runs, since an all-idle bar would just be clutter over the desktop.
    let any_active = all_rows.iter().any(|row| is_active(row.state));
    let wanted =
        settings.enabled && !entries.is_empty() && (any_active || !settings.only_when_running);
    if !wanted {
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

/// Whether a window at `(x, y)` is at least partly on one of the monitors.
fn on_screen(x: i32, y: i32) -> bool {
    let (left, top, width, height) = bb_win::window::virtual_screen();
    x + 20 >= left && x <= left + width - 20 && y + 10 >= top && y <= top + height - 20
}

/// Where the overlay is now, `None` while it is hidden (a hidden window reports a parked position
/// that must never be saved) or not on any monitor.
fn current_position(overlay: &OverlaySwitcher) -> Option<OverlayPosition> {
    if !overlay.window().is_visible() {
        return None;
    }
    let position = overlay.window().position();
    on_screen(position.x, position.y).then_some(OverlayPosition {
        x: position.x,
        y: position.y,
    })
}

/// Decides when the dragged-to position is worth saving: once it has stayed the same for two updates
/// in a row (the drag is over) and differs from the saved one. Saving on every change would rewrite
/// the config twice a second for as long as the bar is being dragged: `WindowMoveArea` hands the
/// drag to Windows and Slint reports neither its start nor its end.
#[derive(Debug)]
struct PositionSaver {
    saved: Option<OverlayPosition>,
    pending: Option<OverlayPosition>,
}

impl PositionSaver {
    fn new(saved: Option<OverlayPosition>) -> Self {
        Self {
            saved,
            pending: None,
        }
    }

    /// Looks at where the window is (`None`: not to be saved) and returns the position to save now,
    /// if any. Best-effort like the other background saves in this app: a failure to write it isn't
    /// worth a toast over something this minor.
    fn observe(&mut self, current: Option<OverlayPosition>) -> Option<OverlayPosition> {
        let Some(current) = current else {
            self.pending = None;
            return None;
        };
        if self.saved == Some(current) {
            self.pending = None;
            return None;
        }
        if self.pending == Some(current) {
            self.pending = None;
            self.saved = Some(current);
            return Some(current);
        }
        self.pending = Some(current);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)] // the positions are `Option`s, this is one of them
    fn at(x: i32, y: i32) -> Option<OverlayPosition> {
        Some(OverlayPosition { x, y })
    }

    #[test]
    fn saves_only_once_the_position_has_settled() {
        let mut saver = PositionSaver::new(at(10, 10));
        // Unchanged: nothing to save.
        assert_eq!(saver.observe(at(10, 10)), None);
        // Being dragged: every update is somewhere else.
        assert_eq!(saver.observe(at(20, 10)), None);
        assert_eq!(saver.observe(at(30, 10)), None);
        assert_eq!(saver.observe(at(40, 10)), None);
        // Let go: the same place twice in a row.
        assert_eq!(saver.observe(at(40, 10)), at(40, 10));
        // And not again while it stays there.
        assert_eq!(saver.observe(at(40, 10)), None);
    }

    #[test]
    fn a_hidden_or_off_screen_window_resets_the_wait() {
        let mut saver = PositionSaver::new(None);
        assert_eq!(saver.observe(at(5, 5)), None);
        assert_eq!(saver.observe(None), None);
        // The first sighting after the gap only starts the wait again.
        assert_eq!(saver.observe(at(5, 5)), None);
        assert_eq!(saver.observe(at(5, 5)), at(5, 5));
    }

    #[test]
    fn moving_back_to_the_saved_position_saves_nothing() {
        let mut saver = PositionSaver::new(at(1, 1));
        assert_eq!(saver.observe(at(9, 9)), None);
        assert_eq!(saver.observe(at(1, 1)), None);
        assert_eq!(saver.observe(at(1, 1)), None);
    }
}
