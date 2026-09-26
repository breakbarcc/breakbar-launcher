//! The account list: rows, selection, toasts and the native frame.

use super::{
    Account, AccountId, AccountRow, AccountState, Cell, CompanionApp, ComponentHandle, Config,
    DARK_FRAME, Duration, HasWindowHandle, LIGHT_FRAME, MAX_TOASTS, MainWindow, Model, Provider,
    RawWindowHandle, Rc, SharedString, TOAST_LIFETIME, ToastData, ToastKind, VecModel, idle_state,
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

/// Runs `f` on the model behind the list. The model is created once and then changed in place, so
/// Slint only updates the rows that changed (a replaced model would rebuild every row, and reset
/// what a row keeps to itself, such as hover). A window that doesn't have such a model yet gets
/// one holding its current rows.
fn with_model<R>(window: &MainWindow, f: impl FnOnce(&VecModel<AccountRow>) -> R) -> R {
    let current = window.get_accounts();
    if let Some(model) = current.as_any().downcast_ref::<VecModel<AccountRow>>() {
        return f(model);
    }
    let model = Rc::new(VecModel::from(rows(window)));
    window.set_accounts(Rc::clone(&model).into());
    f(&model)
}

/// Replaces all rows at once (the start, and the previews).
pub(super) fn set_rows(window: &MainWindow, rows: Vec<AccountRow>) {
    with_model(window, |model| model.set_vec(rows));
    crate::overlay::wake();
}

/// Adds `row` at `index` (the end if that is past it).
pub(super) fn insert_row(window: &MainWindow, index: usize, row: AccountRow) {
    with_model(window, |model| {
        model.insert(index.min(model.row_count()), row);
    });
    crate::overlay::wake();
}

/// Removes the row of account `id`.
pub(super) fn remove_row(window: &MainWindow, id: AccountId) {
    with_model(window, |model| {
        if let Some(index) = position_of(model, id) {
            model.remove(index);
        }
    });
    crate::overlay::wake();
}

/// Moves the row of account `id` to position `to`.
pub(super) fn move_row(window: &MainWindow, id: AccountId, to: usize) {
    with_model(window, |model| move_in(model, id, to));
    crate::overlay::wake();
}

/// Changes the row of account `id` with `f`. Nothing is touched if `f` leaves the row as it was.
pub(super) fn update_row(window: &MainWindow, id: AccountId, f: impl FnOnce(&mut AccountRow)) {
    if with_model(window, |model| update_in(model, id, f)) {
        refresh(window);
        crate::overlay::wake();
    }
}

pub(super) fn clear_selection(window: &MainWindow) {
    with_model(window, |model| {
        for (index, row) in model.iter().enumerate() {
            if row.selected {
                model.set_row_data(
                    index,
                    AccountRow {
                        selected: false,
                        ..row
                    },
                );
            }
        }
    });
    refresh(window);
}

/// Derives what depends on all rows together: the Steam lock and the header counters.
pub(super) fn refresh(window: &MainWindow) {
    let counts = with_model(window, refresh_in);
    window.set_running_count(counts.running);
    window.set_starting_count(counts.starting);
    window.set_selected_count(counts.selected);
}

fn position_of(model: &VecModel<AccountRow>, id: AccountId) -> Option<usize> {
    model.iter().position(|row| row.id == id.0 as i32)
}

/// Returns whether the row was found and `f` changed it.
fn update_in(model: &VecModel<AccountRow>, id: AccountId, f: impl FnOnce(&mut AccountRow)) -> bool {
    let Some(index) = position_of(model, id) else {
        return false;
    };
    let Some(before) = model.row_data(index) else {
        return false;
    };
    let mut row = before.clone();
    f(&mut row);
    if row == before {
        return false;
    }
    model.set_row_data(index, row);
    true
}

/// Puts the row of `id` at position `to` (clamped to the list) by shifting the rows in between:
/// the rows in the list stay where they are, only their data moves.
fn move_in(model: &VecModel<AccountRow>, id: AccountId, to: usize) {
    let Some(from) = position_of(model, id) else {
        return;
    };
    let to = to.min(model.row_count() - 1);
    if from == to {
        return;
    }
    let mut rows: Vec<AccountRow> = model.iter().collect();
    let row = rows.remove(from);
    rows.insert(to, row);
    let changed = from.min(to)..=from.max(to);
    for (index, row) in rows.into_iter().enumerate() {
        if changed.contains(&index) {
            model.set_row_data(index, row);
        }
    }
}

/// The numbers the header shows.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    running: i32,
    starting: i32,
    selected: i32,
}

/// Applies the Steam lock (only one Steam account can run, so the others can't start) to the rows
/// that need it and counts the rows by state.
fn refresh_in(model: &VecModel<AccountRow>) -> Counts {
    let rows: Vec<AccountRow> = model.iter().collect();
    let active_steam = rows
        .iter()
        .find(|row| row.steam && is_active(row.state))
        .map(|row| row.id);

    for (index, row) in rows.iter().enumerate().filter(|(_, row)| row.steam) {
        let lock = active_steam.is_some_and(|active| active != row.id);
        let state = match row.state {
            AccountState::Idle | AccountState::NeedsLogin if lock => AccountState::Locked,
            AccountState::Locked if !lock => idle_state(AccountId(row.id as u32), true),
            state => state,
        };
        if state != row.state {
            model.set_row_data(
                index,
                AccountRow {
                    state,
                    ..row.clone()
                },
            );
        }
    }

    let count = |state| rows.iter().filter(|row| row.state == state).count() as i32;
    Counts {
        running: count(AccountState::Running) + count(AccountState::Stopping),
        starting: count(AccountState::Starting),
        selected: rows.iter().filter(|row| row.selected).count() as i32,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i32, steam: bool, state: AccountState) -> AccountRow {
        AccountRow {
            id,
            name: format!("Account {id}").into(),
            steam,
            state,
            ..AccountRow::default()
        }
    }

    fn model(rows: Vec<AccountRow>) -> Rc<VecModel<AccountRow>> {
        Rc::new(VecModel::from(rows))
    }

    fn ids(model: &VecModel<AccountRow>) -> Vec<i32> {
        model.iter().map(|row| row.id).collect()
    }

    /// `with_model` only avoids replacing the model if the one the window hands out can be
    /// recognized as the `VecModel` it was given.
    #[test]
    fn the_model_of_the_window_is_recognized_again() {
        let list = model(vec![row(1, false, AccountState::Idle)]);
        let handed_out: slint::ModelRc<AccountRow> = Rc::clone(&list).into();

        let same = handed_out
            .as_any()
            .downcast_ref::<VecModel<AccountRow>>()
            .expect("a VecModel behind a ModelRc can be found again");
        same.push(row(2, false, AccountState::Idle));

        assert_eq!(list.row_count(), 2);
    }

    #[test]
    fn updating_a_row_changes_only_that_row() {
        let list = model(vec![
            row(1, false, AccountState::Idle),
            row(2, false, AccountState::Idle),
        ]);

        let changed = update_in(&list, AccountId(2), |row| row.state = AccountState::Running);

        assert!(changed);
        assert_eq!(list.row_data(0).unwrap().state, AccountState::Idle);
        assert_eq!(list.row_data(1).unwrap().state, AccountState::Running);
    }

    #[test]
    fn an_update_that_changes_nothing_is_reported_as_such() {
        let list = model(vec![row(1, false, AccountState::Idle)]);

        assert!(!update_in(&list, AccountId(1), |row| row.state = AccountState::Idle));
        assert!(!update_in(&list, AccountId(9), |row| row.selected = true));
    }

    #[test]
    fn moving_a_row_shifts_the_ones_in_between() {
        let list = model(
            (1..=4)
                .map(|id| row(id, false, AccountState::Idle))
                .collect(),
        );

        move_in(&list, AccountId(1), 2);
        assert_eq!(ids(&list), [2, 3, 1, 4]);
        move_in(&list, AccountId(4), 0);
        assert_eq!(ids(&list), [4, 2, 3, 1]);
        // Past the end counts as the end; staying where it is changes nothing.
        move_in(&list, AccountId(4), 99);
        assert_eq!(ids(&list), [2, 3, 1, 4]);
        move_in(&list, AccountId(3), 1);
        assert_eq!(ids(&list), [2, 3, 1, 4]);
    }

    #[test]
    fn a_running_steam_account_locks_the_other_steam_accounts() {
        let list = model(vec![
            row(1, true, AccountState::Running),
            row(2, true, AccountState::Idle),
            row(3, false, AccountState::Idle),
            AccountRow {
                selected: true,
                ..row(4, true, AccountState::NeedsLogin)
            },
        ]);

        let counts = refresh_in(&list);

        assert_eq!(list.row_data(1).unwrap().state, AccountState::Locked);
        assert_eq!(list.row_data(2).unwrap().state, AccountState::Idle);
        assert_eq!(list.row_data(3).unwrap().state, AccountState::Locked);
        assert_eq!(
            counts,
            Counts {
                running: 1,
                starting: 0,
                selected: 1
            }
        );
    }
}
