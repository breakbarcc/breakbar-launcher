//! The account list: rows, selection, toasts and the native frame.

use super::{
    Account, AccountId, AccountRow, AccountState, Cell, CompanionApp, ComponentHandle, Config,
    DARK_FRAME, Duration, HasWindowHandle, LIGHT_FRAME, MAX_TOASTS, MainWindow, Model, Provider,
    RawWindowHandle, Rc, SharedString, TOAST_LIFETIME, ToastData, ToastKind, idle_state,
};

/// Whether an account in `state` can be started.
pub(crate) fn is_startable(state: AccountState) -> bool {
    matches!(
        state,
        AccountState::Idle | AccountState::NeedsLogin | AccountState::Error
    )
}

/// Whether the account's client is starting, running or being stopped.
pub(crate) fn is_active(state: AccountState) -> bool {
    matches!(
        state,
        AccountState::Starting | AccountState::Running | AccountState::Stopping
    )
}

pub(super) fn account_rows(config: &Config) -> Vec<AccountRow> {
    config
        .accounts
        .iter()
        .map(|account| account_row(account, &config.companions))
        .collect()
}

/// The row of an account that isn't running.
pub(super) fn account_row(account: &Account, companions: &[CompanionApp]) -> AccountRow {
    let mut row = AccountRow {
        id: account.id.0 as i32,
        ..AccountRow::default()
    };
    apply_account(&mut row, account, companions);
    row.state = idle_state(account.id, row.steam);
    row
}

/// Copies what the row shows of the account's settings into `row`.
pub(super) fn apply_account(row: &mut AccountRow, account: &Account, companions: &[CompanionApp]) {
    row.name = account.name.as_str().into();
    row.provider = account.provider.display_name().into();
    row.steam = account.provider == Provider::Steam;
    row.has_companions = companions
        .iter()
        .any(|app| account.companions.contains(&app.id));
}

/// Reads all rows out of the window's current model.
pub(crate) fn rows(window: &MainWindow) -> Vec<AccountRow> {
    let model = window.get_accounts();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect()
}

pub(super) fn row(window: &MainWindow, id: AccountId) -> Option<AccountRow> {
    rows(window).into_iter().find(|row| row.id == id.0 as i32)
}

pub(super) fn set_rows(window: &MainWindow, rows: Vec<AccountRow>) {
    window.set_accounts(Rc::new(slint::VecModel::from(rows)).into());
}

/// Replaces the whole model with one row updated. Rebuilding is simplest and cheap at the
/// list sizes Breakbar deals with (a handful of accounts), and keeps every mutation path
/// (including the one crossing back from a background thread) free of shared, non-`Send`
/// model handles.
pub(super) fn update_row(window: &MainWindow, id: AccountId, f: impl FnOnce(&mut AccountRow)) {
    let mut rows = rows(window);
    if let Some(row) = rows.iter_mut().find(|row| row.id == id.0 as i32) {
        f(row);
        set_rows(window, rows);
        refresh(window);
    }
}

pub(super) fn clear_selection(window: &MainWindow) {
    let mut rows = rows(window);
    if rows.iter().any(|row| row.selected) {
        for row in &mut rows {
            row.selected = false;
        }
        set_rows(window, rows);
        refresh(window);
    }
}

/// Derives what depends on all rows together: the Steam lock and the header counters.
pub(super) fn refresh(window: &MainWindow) {
    let mut rows = rows(window);
    let active_steam = rows
        .iter()
        .find(|row| row.steam && is_active(row.state))
        .map(|row| row.id);

    let mut changed = false;
    for row in rows.iter_mut().filter(|row| row.steam) {
        let lock = active_steam.is_some_and(|active| active != row.id);
        let state = match row.state {
            AccountState::Idle | AccountState::NeedsLogin if lock => AccountState::Locked,
            AccountState::Locked if !lock => idle_state(AccountId(row.id as u32), true),
            state => state,
        };
        if state != row.state {
            row.state = state;
            changed = true;
        }
    }

    let count = |state| rows.iter().filter(|row| row.state == state).count() as i32;
    window.set_running_count(count(AccountState::Running) + count(AccountState::Stopping));
    window.set_starting_count(count(AccountState::Starting));
    window.set_selected_count(rows.iter().filter(|row| row.selected).count() as i32);
    if changed {
        set_rows(window, rows);
    }
}

thread_local! {
    static NEXT_TOAST_ID: Cell<i32> = const { Cell::new(1) };
}

/// Shows a toast at the bottom of the window.
pub(super) fn push_toast(
    window: &MainWindow,
    kind: ToastKind,
    title: SharedString,
    message: SharedString,
) {
    let id = NEXT_TOAST_ID.with(|next| next.replace(next.get() + 1));
    let mut toasts: Vec<ToastData> = window.get_toasts().iter().collect();
    // Same text again (e.g. "Launch all" hitting the same problem twice): show it once.
    toasts.retain(|toast| toast.title != title || toast.message != message);
    toasts.push(ToastData {
        id,
        kind,
        title,
        message,
    });
    let excess = toasts.len().saturating_sub(MAX_TOASTS);
    toasts.drain(..excess);
    window.set_toasts(Rc::new(slint::VecModel::from(toasts)).into());

    expire_toast(window, id, TOAST_LIFETIME);
}

/// Removes toast `id` after `delay`, or later if the pointer is on it then.
pub(super) fn expire_toast(window: &MainWindow, id: i32, delay: Duration) {
    let weak = window.as_weak();
    slint::Timer::single_shot(delay, move || {
        if let Some(window) = weak.upgrade() {
            if window.get_hovered_toast() == id {
                expire_toast(&window, id, Duration::from_secs(2));
            } else {
                dismiss_toast(&window, id);
            }
        }
    });
}

pub(super) fn dismiss_toast(window: &MainWindow, id: i32) {
    let toasts: Vec<ToastData> = window
        .get_toasts()
        .iter()
        .filter(|toast| toast.id != id)
        .collect();
    window.set_toasts(Rc::new(slint::VecModel::from(toasts)).into());
}

/// Colors the native title bar like the current theme.
pub(super) fn apply_frame(window: &MainWindow, dark: bool) {
    if let Some(hwnd) = native_handle(window.window()) {
        let colors = if dark { DARK_FRAME } else { LIGHT_FRAME };
        bb_win::window::set_frame(hwnd, dark, colors);
    }
}

/// Returns the window's HWND so native dialogs can be made modal to it.
pub(crate) fn native_handle(window: &slint::Window) -> Option<isize> {
    let slint_window = window.window_handle();
    let handle = slint_window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow) {
    window.on_toggle_selected({
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                update_row(&window, AccountId(id as u32), |row| {
                    row.selected = !row.selected;
                });
            }
        }
    });

    window.on_clear_selection({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                clear_selection(&window);
            }
        }
    });

    window.on_dismiss_toast({
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                dismiss_toast(&window, id);
            }
        }
    });

    window.on_theme_changed({
        let weak = window.as_weak();
        move |dark| {
            if let Some(window) = weak.upgrade() {
                apply_frame(&window, dark);
            }
        }
    });
}
