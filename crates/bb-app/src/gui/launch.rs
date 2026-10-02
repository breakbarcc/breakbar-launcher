//! Starting and stopping clients: the launch queue and what happens when a client starts and exits.

use super::{
    Account, AccountId, AccountState, App, Arc, CompanionApp, ComponentHandle, LaunchError,
    LaunchFailure, LaunchMode, LaunchWarning, MainWindow, Messages, PathBuf, Provider, Rc, RefCell,
    SharedInstances, SharedString, ToastKind, apply_after_start, clear_selection, companions,
    discord, game, idle_state, is_active, is_startable, launcher, login_file_stamp, mpsc,
    push_toast, report_login_setup, row, rows, steam_ready, thread, update_row,
};
use crate::profile_link;

/// Starts `id` if idle, or requests that its running client stop.
///
/// Stopping only asks the client to terminate; the status returns to idle once the background
/// monitor thread observes the actual exit. A client that is still starting (queued or waiting
/// for its `Local.dat`) can't be stopped or started again until its launch finishes.
pub(super) fn toggle_account(
    window: &MainWindow,
    app: &RefCell<App>,
    queue: &LaunchQueue,
    id: AccountId,
) {
    let Some(row) = row(window, id) else {
        return;
    };
    match row.state {
        AccountState::Running if row.handle != 0 => stop_account(window, id),
        state if is_startable(state) => start_account(window, app, queue, id, LaunchMode::Play),
        _ => {}
    }
}

/// Terminates `id`'s running client, if it has one. A no-op for any other state (not running yet,
/// already stopping, ...).
pub(crate) fn stop_account(window: &MainWindow, id: AccountId) {
    let Some(row) = row(window, id) else {
        return;
    };
    if row.state != AccountState::Running || row.handle == 0 {
        return;
    }
    update_row(window, id, |row| row.state = AccountState::Stopping);
    if let Err(error) = bb_win::process::terminate(row.handle as isize) {
        update_row(window, id, |row| row.state = AccountState::Running);
        push_toast(
            window,
            ToastKind::Error,
            window.global::<Messages>().invoke_stop_failed_title(),
            error.to_string().into(),
        );
    }
}

/// Starts the selected accounts, or all startable ones if nothing is selected.
pub(crate) fn launch_all(window: &MainWindow, app: &RefCell<App>, queue: &LaunchQueue) {
    let rows = rows(window);
    let any_selected = rows.iter().any(|row| row.selected);
    let ids: Vec<AccountId> = rows
        .iter()
        .filter(|row| is_startable(row.state) && (row.selected || !any_selected))
        .map(|row| AccountId(row.id as u32))
        .collect();
    clear_selection(window);
    for id in ids {
        start_account(window, app, queue, id, LaunchMode::Play);
    }
}

/// Queues `id`'s launch. The row counts as starting from now on, so it can't be queued twice.
pub(crate) fn start_account(
    window: &MainWindow,
    app: &RefCell<App>,
    queue: &LaunchQueue,
    id: AccountId,
    mode: LaunchMode,
) {
    let messages = window.global::<Messages>();
    let (gw2_path, account, companions, fps_limit) = {
        let app = app.borrow();
        let account = app.config.accounts.iter().find(|a| a.id == id).cloned();
        let companions = account
            .as_ref()
            .map(|account| app.companions_of(account))
            .unwrap_or_default();
        (
            app.config.gw2_path.clone(),
            account,
            companions,
            app.config.fps_limit.frames_per_second(),
        )
    };
    let Some(gw2_path) = gw2_path else {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_start_failed(),
            messages.invoke_set_path_first(),
        );
        return;
    };
    let Some(account) = account else {
        return;
    };

    // Steam signs the game in with whichever Steam user is currently signed in, so a second
    // Steam account can't run next to the first one.
    if account.provider == Provider::Steam
        && let Some(running) = rows(window)
            .into_iter()
            .find(|row| row.steam && is_active(row.state) && row.id != id.0 as i32)
    {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_steam_busy_title(),
            messages.invoke_steam_busy(account.name.as_str().into(), running.name),
        );
        return;
    }

    if account.provider == Provider::Steam && !steam_ready(window, app, &gw2_path, &account.name) {
        return;
    }

    maybe_start_discord(window, app);

    update_row(window, id, |row| {
        row.state = AccountState::Starting;
        row.handle = 0;
        row.detail = SharedString::new();
    });
    queue.push(LaunchJob {
        gw2_path,
        account,
        mode,
        fps_limit,
        companions,
    });

    if mode == LaunchMode::Play {
        apply_after_start(window);
    }
}

/// Starts Discord alongside the very first account of a session, if the user turned that on in
/// Settings and it isn't already running. Fires once a launch is actually going ahead (all the
/// checks above `start_account`'s call site have passed), not on every click. Best-effort and
/// silent: a failure here must never block the account launch it's piggybacking on.
fn maybe_start_discord(window: &MainWindow, app: &RefCell<App>) {
    if rows(window).iter().any(|row| is_active(row.state)) {
        return;
    }
    let path = {
        let app = app.borrow();
        app.config
            .discord_autostart
            .then(|| app.config.discord_path.clone())
            .flatten()
    };
    let Some(path) = path else {
        return;
    };
    if let Err(error) = discord::start_if_not_running(&path) {
        crate::log::write(format!("could not start Discord: {error}"));
    }
}

#[derive(Debug)]
pub(super) struct LaunchJob {
    gw2_path: PathBuf,
    account: Account,
    mode: LaunchMode,
    /// Frame rate limit for the client (`-fps:N`), `None` for unlimited.
    fps_limit: Option<u32>,
    /// Started together with the client, closed after it exits.
    companions: Vec<CompanionApp>,
}

/// Runs launches one after another on a single background thread.
///
/// Each launch blocks for several seconds while `%APPDATA%\Guild Wars 2` points at that account
/// (see [`launcher::launch`]), so launches must never overlap and must never run on the UI
/// thread. A channel keeps them in click order.
#[derive(Debug)]
pub(crate) struct LaunchQueue {
    jobs: mpsc::Sender<LaunchJob>,
}

impl LaunchQueue {
    pub(super) fn start(window: slint::Weak<MainWindow>) -> Self {
        let (jobs, receiver) = mpsc::channel::<LaunchJob>();
        let shared = Arc::new(SharedInstances::default());
        thread::spawn(move || {
            for job in receiver {
                run_launch(&window, &shared, job);
            }
        });
        Self { jobs }
    }

    pub(super) fn push(&self, job: LaunchJob) {
        // The worker only stops when the sender is dropped, i.e. when the window is gone.
        let _ = self.jobs.send(job);
    }
}

/// Launches one client (on the queue's worker thread) and hands the result to the UI thread.
pub(super) fn run_launch(
    window: &slint::Weak<MainWindow>,
    shared: &Arc<SharedInstances>,
    job: LaunchJob,
) {
    let id = job.account.id;
    let name = job.account.name.clone();
    let login_before = login_file_stamp(id);

    let launched = match launcher::launch(&job.gw2_path, &job.account, job.mode, job.fps_limit) {
        Ok(launched) => launched,
        Err(error) => {
            crate::log::write(format!("starting {name} failed: {error}"));
            let _ = window.upgrade_in_event_loop(move |window| {
                show_start_failed(&window, id, &name, &error);
            });
            return;
        }
    };
    if let Some(warning) = &launched.warning {
        crate::log::write(format!("{name} started with a warning: {warning:?}"));
    }

    let pid = launched.client.pid();
    let (hour, minute) = bb_win::time::local_hour_minute();
    let configure = job.mode == LaunchMode::Configure;
    let started = Started {
        id,
        name: name.clone(),
        // Slint's `int` is 32-bit; real process handle values comfortably fit in practice (they
        // are small table indices even in a 64-bit process), so this narrowing is safe here.
        handle: launched.client.raw_handle() as i32,
        since: format!("{hour:02}:{minute:02}"),
        setup: launched.setup,
        steam: job.account.provider == Provider::Steam,
        warning: launched.warning,
        configure,
    };
    let (setup, steam) = (started.setup, started.steam);
    let _ = window.upgrade_in_event_loop(move |window| show_started(&window, started));

    let mut session =
        companions::Session::new(Arc::clone(shared), job.companions, &job.account, pid);
    let gw2_path = job.gw2_path;
    let mut client = launched.client;
    let window = window.clone();
    thread::spawn(move || {
        let errors = companions::start_for(&mut session, &mut client);
        let addon_active = session.has_started();
        let _ = window.upgrade_in_event_loop(move |window| {
            show_companions(&window, id, addon_active, errors);
        });

        let result = client.wait_for_exit();
        session.stop();
        if configure {
            // The junction was deliberately left on this account for its whole session (see
            // `LaunchMode::Configure`); now that it has exited, hand the profile back — seeded
            // with what was just configured, so a client reading it late (see
            // `seed_shared_with_account_settings`) still sees the right values.
            launcher::end_configure(id);
            if let Err(error) = profile_link::seed_shared_with_account_settings(id) {
                crate::log::write(format!(
                    "could not seed the shared profile with {name}'s configured settings: {error}"
                ));
            }
            if let Err(error) = profile_link::point_to_shared() {
                crate::log::write(format!(
                    "could not point Guild Wars 2 back at the shared profile after configuring \
                     {name}: {error}"
                ));
            }
        }
        // Marshal back to the UI thread: Slint's model and window may only be touched there.
        let exited = Exited {
            id,
            name,
            setup,
            steam,
            gw2_path,
            login_before,
            result,
            configure,
        };
        let _ = window.upgrade_in_event_loop(move |window| show_exited(&window, exited));
    });
}

/// A client that has just started.
struct Started {
    id: AccountId,
    name: String,
    handle: i32,
    /// Start time of day, `HH:MM`.
    since: String,
    setup: bool,
    steam: bool,
    warning: Option<LaunchWarning>,
    /// Started with `LaunchMode::Configure`.
    configure: bool,
}

/// A client that has exited.
struct Exited {
    id: AccountId,
    name: String,
    setup: bool,
    steam: bool,
    gw2_path: PathBuf,
    /// The account's `Local.dat` before the client started, to tell whether it saved anything.
    login_before: Option<(u64, std::time::SystemTime)>,
    result: std::io::Result<std::process::ExitStatus>,
    /// Started with `LaunchMode::Configure`.
    configure: bool,
}

/// Shows that starting `name` failed.
fn show_start_failed(window: &MainWindow, id: AccountId, name: &str, error: &LaunchError) {
    let messages = window.global::<Messages>();
    let (kind, detail) = launch_failure(error);
    update_row(window, id, |row| {
        row.state = AccountState::Error;
        row.detail = messages.invoke_start_failed_detail();
    });
    push_toast(
        window,
        ToastKind::Error,
        messages.invoke_start_failed(),
        messages.invoke_launch_failure(kind, name.into(), detail),
    );
}

/// Shows the running client, and what the launch had to say about it.
fn show_started(window: &MainWindow, started: Started) {
    let Started {
        id,
        name,
        handle,
        since,
        setup,
        steam,
        warning,
        configure,
    } = started;
    let messages = window.global::<Messages>();
    update_row(window, id, |row| {
        row.state = AccountState::Running;
        row.handle = handle;
        row.since = since.into();
    });
    if setup {
        window.set_login_setup_name(name.as_str().into());
        window.set_login_setup_steam(steam);
    }
    if configure {
        push_toast(
            window,
            ToastKind::Info,
            messages.invoke_configure_started_title(),
            messages.invoke_configure_started(name.as_str().into()),
        );
    }
    match warning {
        Some(LaunchWarning::SlowStart) => push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_slow_start_title(),
            messages.invoke_slow_start(name.as_str().into()),
        ),
        Some(LaunchWarning::ProfileNotRestored(error)) => push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_profile_restore_title(),
            messages.invoke_profile_restore(error.to_string().into()),
        ),
        None => {}
    }
}

/// Shows that companions have started, and the ones that could not.
fn show_companions(
    window: &MainWindow,
    id: AccountId,
    addon_active: bool,
    errors: Vec<(String, std::io::Error)>,
) {
    if addon_active {
        update_row(window, id, |row| row.addon_active = true);
    }
    let messages = window.global::<Messages>();
    for (app, error) in errors {
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_companion_failed_title(app.as_str().into()),
            error.to_string().into(),
        );
    }
}

/// Puts the row back to idle (or to an error if the client crashed) after its client exited, and
/// reports how a login setup ended.
fn show_exited(window: &MainWindow, exited: Exited) {
    let Exited {
        id,
        name,
        setup,
        steam,
        gw2_path,
        login_before,
        result,
        configure,
    } = exited;
    let messages = window.global::<Messages>();
    let stopped = row(window, id).is_some_and(|row| row.state == AccountState::Stopping);
    let failure = match &result {
        _ if stopped => None,
        Ok(status) if status.success() => None,
        Ok(status) => Some(
            status
                .code()
                .map_or_else(|| "?".to_owned(), |code| code.to_string()),
        ),
        Err(error) => Some(error.to_string()),
    };
    update_row(window, id, |row| {
        row.handle = 0;
        row.since = SharedString::new();
        row.addon_active = false;
        match &failure {
            None => {
                row.state = idle_state(id, row.steam);
                row.detail = SharedString::new();
            }
            Some(code) => {
                row.state = AccountState::Error;
                row.detail = messages.invoke_exited_detail(code.as_str().into());
            }
        }
    });
    let finished = setup && !stopped && failure.is_none();
    if setup {
        window.set_login_setup_name(SharedString::new());
    }
    if finished {
        // The client is through with this build: the login counts as current for it even if the
        // client had nothing to change in its file.
        if let Some(build) = game::game_build(&gw2_path) {
            let _ = bb_store::mark_build_verified(id, build);
        }
        report_login_setup(window, id, &name, steam, login_before);
    }
    if let Some(code) = failure {
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_exited_title(),
            messages.invoke_exited(name.as_str().into(), code.into()),
        );
    } else if configure && !stopped {
        push_toast(
            window,
            ToastKind::Success,
            messages.invoke_configure_saved_title(),
            messages.invoke_configure_saved(name.as_str().into()),
        );
    }
    window.invoke_client_exited(finished);
}

/// Maps a launch error to its message and the technical detail filled into it.
pub(super) fn launch_failure(error: &LaunchError) -> (LaunchFailure, SharedString) {
    let (kind, detail) = match error {
        LaunchError::NoGamePath => (LaunchFailure::NoGamePath, String::new()),
        LaunchError::SteamNotRunning(_) => (LaunchFailure::SteamNotRunning, String::new()),
        LaunchError::SteamInstallMissing(_) => (LaunchFailure::SteamInstallMissing, String::new()),
        LaunchError::SetupNeedsExclusive(_) => (LaunchFailure::SetupNeedsExclusive, String::new()),
        LaunchError::SetupClientRunning => (LaunchFailure::SetupClientRunning, String::new()),
        LaunchError::ConfigureNeedsExclusive(_) => {
            (LaunchFailure::ConfigureNeedsExclusive, String::new())
        }
        LaunchError::ConfigureSessionActive => {
            (LaunchFailure::ConfigureSessionActive, String::new())
        }
        LaunchError::PatchNeedsExclusive => (LaunchFailure::PatchNeedsExclusive, String::new()),
        LaunchError::AlreadyRunning(_) => (LaunchFailure::AlreadyRunning, String::new()),
        LaunchError::Profile(error) => (LaunchFailure::Profile, error.to_string()),
        LaunchError::ProfileLink(error) => (LaunchFailure::ProfileLink, error.to_string()),
        LaunchError::Spawn(error) => (LaunchFailure::Spawn, error.to_string()),
        LaunchError::ExitedDuringStartup(status) => (
            LaunchFailure::ExitedDuringStartup,
            status
                .code()
                .map_or_else(|| "?".to_owned(), |code| code.to_string()),
        ),
    };
    (kind, detail.into())
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>, queue: &Rc<LaunchQueue>) {
    window.on_launch_account({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                toggle_account(&window, &app, &queue, AccountId(id as u32));
            }
        }
    });

    window.on_launch_all({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                launch_all(&window, &app, &queue);
            }
        }
    });

    window.on_set_up_login({
        let app = Rc::clone(app);
        let queue = Rc::clone(queue);
        let weak = window.as_weak();
        move |id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let id = AccountId(id as u32);
            if row(&window, id).is_some_and(|row| is_startable(row.state)) {
                start_account(&window, &app, &queue, id, LaunchMode::SetUpLogin);
            }
        }
    });
}
