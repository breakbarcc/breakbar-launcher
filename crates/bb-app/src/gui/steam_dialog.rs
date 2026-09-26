//! The dialog that links the `ArenaNet` installation into Steam.

use super::{
    App, ComponentHandle, MainWindow, Messages, Path, PendingSteamSetup, Rc, RefCell,
    SteamSetupStep, ToastKind, display_path, game, push_toast, steam_setup,
};

/// Whether a Steam account can be started now. If Steam has no copy of the game yet, opens the
/// setup dialog that links the `ArenaNet` installation into Steam and returns `false`. Cases the
/// dialog can't help with (no Steam at all) pass, so the launch reports them itself.
pub(super) fn steam_ready(
    window: &MainWindow,
    app: &RefCell<App>,
    gw2_path: &Path,
    account_name: &str,
) -> bool {
    let (step, link) = match steam_setup::plan(gw2_path) {
        None | Some(steam_setup::Plan::NoSteam) => return true,
        Some(steam_setup::Plan::Blocked(path)) => {
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_steam_setup_failed_title(),
                messages.invoke_steam_setup_blocked(display_path(Some(&path)).into()),
            );
            return false;
        }
        Some(steam_setup::Plan::CreateLink(link)) => (SteamSetupStep::Link, link),
        Some(steam_setup::Plan::AwaitInstall(link)) => (SteamSetupStep::Install, link),
    };
    window.set_steam_setup_account(account_name.into());
    window.set_steam_setup_link(display_path(Some(&link.link)).into());
    window.set_steam_setup_target(display_path(Some(&link.target)).into());
    window.set_steam_setup_retry(false);
    window.set_steam_setup(step);
    app.borrow_mut().steam_setup = Some(PendingSteamSetup {
        link,
        account: account_name.to_owned(),
    });
    // A start from the tray or the overlay may find the window hidden.
    let _ = window.show();
    false
}

/// The user agreed to create the link: creates it and asks them to install in Steam.
pub(super) fn steam_setup_create_link(window: &MainWindow, app: &RefCell<App>) {
    let Some(link) = app
        .borrow()
        .steam_setup
        .as_ref()
        .map(|pending| pending.link.clone())
    else {
        return;
    };
    match steam_setup::create_link(&link) {
        Ok(()) => window.set_steam_setup(SteamSetupStep::Install),
        Err(error) => {
            close_steam_setup(window, app);
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_steam_setup_failed_title(),
                error.to_string().into(),
            );
        }
    }
}

/// The user says Steam has finished installing: checks it and reports.
pub(super) fn steam_setup_done(window: &MainWindow, app: &RefCell<App>) {
    let ready = {
        let app = app.borrow();
        app.config
            .gw2_path
            .as_deref()
            .is_some_and(|path| game::steam_client(path).is_some())
    };
    if !ready {
        window.set_steam_setup_retry(true);
        return;
    }
    let account = app
        .borrow()
        .steam_setup
        .as_ref()
        .map(|pending| pending.account.clone())
        .unwrap_or_default();
    close_steam_setup(window, app);
    let messages = window.global::<Messages>();
    push_toast(
        window,
        ToastKind::Success,
        messages.invoke_steam_setup_ready_title(),
        messages.invoke_steam_setup_ready(account.into()),
    );
}

pub(super) fn close_steam_setup(window: &MainWindow, app: &RefCell<App>) {
    app.borrow_mut().steam_setup = None;
    window.set_steam_setup(SteamSetupStep::Hidden);
    window.set_steam_setup_retry(false);
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_steam_setup_create_link({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                steam_setup_create_link(&window, &app);
            }
        }
    });

    window.on_steam_setup_done({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                steam_setup_done(&window, &app);
            }
        }
    });

    window.on_steam_setup_cancel({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                close_steam_setup(&window, &app);
            }
        }
    });
}
