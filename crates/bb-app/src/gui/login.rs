//! Logins: the state of an account's `Local.dat`, the setup offer and "Set up one after another".

use super::{
    AccountId, AccountState, App, ComponentHandle, LaunchMode, LaunchQueue, MainWindow, Messages,
    Rc, RefCell, SharedString, ToastKind, game, is_active, is_startable, push_toast, refresh, row,
    rows, start_account, update_row,
};

/// Size and modification time of the account's `Local.dat`, `None` if it has none.
pub(super) fn login_file_stamp(id: AccountId) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(bb_store::local_dat_path(id).ok()?).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Tells the user how a login setup ended. The client saves the login into `Local.dat` when it
/// is closed normally, so a file that didn't change means nothing was saved. That can't show
/// whether the password was stored, only that the client wrote its file.
pub(super) fn report_login_setup(
    window: &MainWindow,
    id: AccountId,
    name: &str,
    steam: bool,
    before: Option<(u64, std::time::SystemTime)>,
) {
    let messages = window.global::<Messages>();
    let after = login_file_stamp(id);
    if after.is_some() && after != before {
        push_toast(
            window,
            ToastKind::Success,
            messages.invoke_login_setup_done_title(),
            messages.invoke_login_setup_done(name.into()),
        );
    } else {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_login_setup_unchanged_title(),
            messages.invoke_login_setup_unchanged(name.into(), steam),
        );
    }
}

/// Offers to set up the login of a just created account.
pub(super) fn offer_login_setup(window: &MainWindow, id: AccountId, name: &str, steam: bool) {
    window.set_login_offer_name(name.into());
    window.set_login_offer_steam(steam);
    window.set_login_offer_id(id.0 as i32);
}

/// The accounts (in list order) that don't run and whose login is older than the game.
pub(super) fn outdated_accounts(window: &MainWindow, app: &RefCell<App>) -> Vec<AccountId> {
    let Some(gw2_path) = app.borrow().config.gw2_path.clone() else {
        return Vec::new();
    };
    rows(window)
        .into_iter()
        .filter(|row| !is_active(row.state))
        .map(|row| AccountId(row.id as u32))
        .filter(|id| game::login_outdated(&gw2_path, *id))
        .collect()
}

/// Puts every account that isn't running in the state its login calls for (no or an outdated
/// login: "login needed"; otherwise ready) and shows which accounts have to refresh their login
/// after a game update. A game update makes every `Local.dat` older than `Gw2.dat`, see
/// [`game::login_outdated`].
pub(super) fn sync_login_states(window: &MainWindow, app: &RefCell<App>) {
    let gw2_path = app.borrow().config.gw2_path.clone();
    let mut outdated_names = Vec::new();
    for row in rows(window) {
        let id = AccountId(row.id as u32);
        let outdated = gw2_path
            .as_deref()
            .is_some_and(|path| game::login_outdated(path, id));
        if matches!(row.state, AccountState::Idle | AccountState::NeedsLogin) {
            let wanted = if outdated || (!row.steam && !bb_store::is_set_up(id)) {
                AccountState::NeedsLogin
            } else {
                AccountState::Idle
            };
            if wanted != row.state {
                update_row(window, id, |row| row.state = wanted);
            }
        }
        if outdated && !is_active(row.state) {
            outdated_names.push(row.name.to_string());
        }
    }
    let names: SharedString = outdated_names.join(", ").into();
    if window.get_refresh_names() != names {
        window.set_refresh_names(names);
    }
    refresh(window);
}

/// Goes on with "Set up one after another": takes the next account of the queue that still needs
/// its login refreshed. The first one is started at once (the user just asked for it); each
/// further one is offered in the login dialog after the previous one finished.
pub(super) fn continue_refresh(
    window: &MainWindow,
    app: &RefCell<App>,
    queue: &LaunchQueue,
    start_now: bool,
) {
    loop {
        let Some(id) = app.borrow_mut().refresh_queue.pop_front() else {
            return;
        };
        let outdated = app
            .borrow()
            .config
            .gw2_path
            .as_deref()
            .is_some_and(|path| game::login_outdated(path, id));
        let Some(row) = row(window, id) else {
            continue;
        };
        if !outdated || !is_startable(row.state) {
            continue;
        }
        if start_now {
            start_account(window, app, queue, id, LaunchMode::SetUpLogin);
        } else {
            offer_login_setup(window, id, row.name.as_str(), row.steam);
        }
        return;
    }
}

/// State of an account that isn't running: `ArenaNet` accounts without their own `Local.dat` have
/// to set up their login first. Steam accounts need no login; their first start just creates it.
pub(super) fn idle_state(id: AccountId, steam: bool) -> AccountState {
    if steam || bb_store::is_set_up(id) {
        AccountState::Idle
    } else {
        AccountState::NeedsLogin
    }
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>, queue: &Rc<LaunchQueue>) {
    window.on_refresh_logins({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let ids = outdated_accounts(&window, &app);
            app.borrow_mut().refresh_queue = ids.into();
            continue_refresh(&window, &app, &queue, true);
        }
    });

    window.on_client_exited({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move |setup_finished| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            sync_login_states(&window, &app);
            if setup_finished {
                continue_refresh(&window, &app, &queue, false);
            }
        }
    });

    window.on_login_offer_accept({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move |id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            window.set_login_offer_id(0);
            let id = AccountId(id as u32);
            if row(&window, id).is_some_and(|row| is_startable(row.state)) {
                start_account(&window, &app, &queue, id, LaunchMode::SetUpLogin);
            }
        }
    });

    window.on_login_offer_dismiss({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                window.set_login_offer_id(0);
            }
        }
    });
}
