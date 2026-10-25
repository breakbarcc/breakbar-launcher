//! The Nexus section of the settings page and its update banner: installing, updating and
//! removing the addon loader, and the hourly check for a new release.

use super::{
    App, ComponentHandle, MainWindow, Messages, NexusProblem, NexusStatus, NexusTask, Rc, RefCell,
    SharedString, ToastKind, push_toast, thread,
};
use crate::nexus::{self, Error, Release, Status};

fn to_ui_status(status: Status) -> NexusStatus {
    match status {
        Status::NotInstalled => NexusStatus::NotInstalled,
        Status::Foreign => NexusStatus::Foreign,
        Status::Installed {
            update_available: false,
        } => NexusStatus::Current,
        Status::Installed {
            update_available: true,
        } => NexusStatus::UpdateAvailable,
    }
}

fn problem(error: &Error) -> (NexusProblem, SharedString) {
    match error {
        Error::ClientRunning => (NexusProblem::ClientRunning, SharedString::new()),
        Error::ForeignDll => (NexusProblem::Foreign, SharedString::new()),
        Error::ChainloadExists => (NexusProblem::ChainloadExists, SharedString::new()),
        Error::NotNexus => (NexusProblem::NotNexus, SharedString::new()),
        Error::AddonsMissing => (NexusProblem::AddonsMissing, SharedString::new()),
        Error::Mismatch => (NexusProblem::Mismatch, SharedString::new()),
        Error::Io(error) => (NexusProblem::Io, error.to_string().into()),
    }
}

fn toast_problem(window: &MainWindow, kind: NexusProblem, detail: SharedString) {
    let messages = window.global::<Messages>();
    push_toast(
        window,
        ToastKind::Error,
        messages.invoke_nexus_failed_title(),
        messages.invoke_nexus_problem(kind, detail),
    );
}

/// Re-reads the game folder and, if the setting allows, asks GitHub for the latest release. Does
/// nothing while Nexus management is off. Runs on a thread of its own: it reads the DLL, hashes it
/// and uses the network.
pub(super) fn refresh(window: &MainWindow, app: &Rc<RefCell<App>>) {
    let (enabled, check, gw2_path) = {
        let app = app.borrow();
        (
            app.config.nexus.enabled,
            app.config.nexus.check_for_updates,
            app.config.gw2_path.clone(),
        )
    };
    let Some(gw2_path) = enabled.then_some(gw2_path).flatten() else {
        return;
    };

    let weak = window.as_weak();
    let _ = thread::Builder::new()
        .name("nexus-check".to_owned())
        .spawn(move || {
            let Some(dir) = nexus::game_dir(&gw2_path) else {
                return;
            };
            let latest: Option<Release> = if check { nexus::fetch_latest() } else { None };
            let status = nexus::status(dir, latest.as_ref());
            let reffect = nexus::reffect_installed(dir);
            let _ = weak.upgrade_in_event_loop(move |window| {
                window.set_nexus_status(to_ui_status(status));
                window.set_reffect_installed(reffect);
                if let Some(latest) = latest {
                    window.set_nexus_version(latest.tag.as_str().into());
                }
            });
        });
}

/// Installs or updates Nexus. `rename_foreign` is the user's confirmation to move another
/// `d3d11.dll` aside; without it, finding one opens the confirmation dialog instead.
fn install(window: &MainWindow, app: &Rc<RefCell<App>>, rename_foreign: bool) {
    let Some(gw2_path) = app.borrow().config.gw2_path.clone() else {
        return;
    };
    if window.get_nexus_task() != NexusTask::Idle {
        return;
    }
    window.set_nexus_task(NexusTask::Installing);

    let weak = window.as_weak();
    let _ = thread::Builder::new()
        .name("nexus-install".to_owned())
        .spawn(move || {
            let outcome = match nexus::fetch_latest() {
                None => Err(None),
                Some(release) => nexus::install(&gw2_path, &release, rename_foreign)
                    .map(|()| release)
                    .map_err(Some),
            };
            let status = nexus::game_dir(&gw2_path).map(|dir| nexus::status(dir, None));
            let _ = weak.upgrade_in_event_loop(move |window| {
                window.set_nexus_task(NexusTask::Idle);
                if let Some(status) = status {
                    window.set_nexus_status(to_ui_status(status));
                }
                match outcome {
                    Ok(release) => {
                        window.set_nexus_version(release.tag.as_str().into());
                        let messages = window.global::<Messages>();
                        push_toast(
                            &window,
                            ToastKind::Success,
                            messages.invoke_nexus_done_title(),
                            messages.invoke_nexus_installed(release.tag.as_str().into()),
                        );
                    }
                    Err(Some(Error::ForeignDll)) => window.set_nexus_confirm_open(true),
                    Err(Some(error)) => {
                        let (kind, detail) = problem(&error);
                        toast_problem(&window, kind, detail);
                    }
                    Err(None) => toast_problem(&window, NexusProblem::Offline, SharedString::new()),
                }
            });
        });
}

/// Installs Reffect into Nexus's addons folder.
fn install_reffect(window: &MainWindow, app: &Rc<RefCell<App>>) {
    let Some(gw2_path) = app.borrow().config.gw2_path.clone() else {
        return;
    };
    if window.get_nexus_task() != NexusTask::Idle {
        return;
    }
    window.set_nexus_task(NexusTask::InstallingReffect);

    let weak = window.as_weak();
    let _ = thread::Builder::new()
        .name("reffect-install".to_owned())
        .spawn(move || {
            let outcome = match nexus::fetch_latest_reffect() {
                None => Err(None),
                Some(release) => nexus::install_reffect(&gw2_path, &release)
                    .map(|()| release)
                    .map_err(Some),
            };
            let installed = nexus::game_dir(&gw2_path).is_some_and(nexus::reffect_installed);
            let _ = weak.upgrade_in_event_loop(move |window| {
                window.set_nexus_task(NexusTask::Idle);
                window.set_reffect_installed(installed);
                match outcome {
                    Ok(release) => {
                        let messages = window.global::<Messages>();
                        push_toast(
                            &window,
                            ToastKind::Success,
                            messages.invoke_reffect_title(),
                            messages.invoke_reffect_installed(release.tag.as_str().into()),
                        );
                    }
                    Err(Some(error)) => {
                        let (kind, detail) = problem(&error);
                        toast_problem(&window, kind, detail);
                    }
                    Err(None) => toast_problem(&window, NexusProblem::Offline, SharedString::new()),
                }
            });
        });
}

fn uninstall(window: &MainWindow, app: &Rc<RefCell<App>>) {
    let Some(gw2_path) = app.borrow().config.gw2_path.clone() else {
        return;
    };
    if window.get_nexus_task() != NexusTask::Idle {
        return;
    }
    window.set_nexus_task(NexusTask::Removing);

    let weak = window.as_weak();
    let _ = thread::Builder::new()
        .name("nexus-uninstall".to_owned())
        .spawn(move || {
            let result = nexus::uninstall(&gw2_path);
            let status = nexus::game_dir(&gw2_path).map(|dir| nexus::status(dir, None));
            let _ = weak.upgrade_in_event_loop(move |window| {
                window.set_nexus_task(NexusTask::Idle);
                if let Some(status) = status {
                    window.set_nexus_status(to_ui_status(status));
                }
                match result {
                    Ok(()) => {
                        let messages = window.global::<Messages>();
                        push_toast(
                            &window,
                            ToastKind::Success,
                            messages.invoke_nexus_done_title(),
                            messages.invoke_nexus_removed(),
                        );
                    }
                    Err(error) => {
                        let (kind, detail) = problem(&error);
                        toast_problem(&window, kind, detail);
                    }
                }
            });
        });
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_set_nexus_enabled({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.set_nexus_enabled(value);
                {
                    let mut app = app.borrow_mut();
                    app.config.nexus.enabled = value;
                    app.save(&window);
                }
                refresh(&window, &app);
            }
        }
    });

    window.on_set_nexus_check_for_updates({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.set_nexus_check_for_updates(value);
                {
                    let mut app = app.borrow_mut();
                    app.config.nexus.check_for_updates = value;
                    app.save(&window);
                }
                if value {
                    refresh(&window, &app);
                } else if window.get_nexus_status() == NexusStatus::UpdateAvailable {
                    // Nothing to announce while the check is off.
                    window.set_nexus_status(NexusStatus::Current);
                }
            }
        }
    });

    window.on_nexus_install({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |rename_foreign| {
            if let Some(window) = weak.upgrade() {
                install(&window, &app, rename_foreign);
            }
        }
    });

    window.on_install_reffect({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                install_reffect(&window, &app);
            }
        }
    });

    window.on_nexus_uninstall({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                uninstall(&window, &app);
            }
        }
    });
}
