//! The main window and its state.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use bb_core::{Account, AccountId, BLISH_HUD, CompanionApp, CompanionId, Provider, Trigger};
use bb_store::Config;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, SharedString};
use ui::{
    AccountRow, AccountState, LaunchFailure, MainWindow, Messages, PathProblem, Theme, ToastData,
    ToastKind,
};

use crate::companions::{self, SharedInstances};
use crate::launcher::{LaunchError, LaunchMode, LaunchWarning};
use crate::{game, launcher};

/// Code generated from `ui/app.slint`.
///
/// Slint's generated component types don't implement `Debug`, and generated code can't be
/// edited, so our workspace-wide `missing_debug_implementations` lint is disabled here only.
mod ui {
    #![allow(missing_debug_implementations)]
    slint::include_modules!();
}

/// Native frame colors per theme: caption = `title-bar`, text = `text-muted`, border = `line-2`.
const DARK_FRAME: bb_win::window::FrameColors = bb_win::window::FrameColors {
    caption: 0x0d0f12,
    text: 0x98a0ab,
    border: 0x2a3038,
};
const LIGHT_FRAME: bb_win::window::FrameColors = bb_win::window::FrameColors {
    caption: 0xece2cc,
    text: 0x554c3c,
    border: 0xcbbb9a,
};

/// At most this many toasts are shown at once; older ones make room.
const MAX_TOASTS: usize = 3;
/// Toasts disappear after this long, unless the pointer is on them.
const TOAST_LIFETIME: Duration = Duration::from_secs(6);

/// Application state shared between UI callbacks.
///
/// Only ever touched on the UI thread: Slint callbacks run there, and so does the
/// `invoke_from_event_loop` closure that background launch-monitoring threads hand back.
#[derive(Debug)]
struct App {
    config: Config,
    /// `None` if the config could not be loaded; saving is then disabled so a broken
    /// config file is never overwritten with defaults.
    config_path: Option<PathBuf>,
}

impl App {
    fn load() -> (Self, Option<bb_store::StoreError>) {
        let loaded = bb_store::default_config_path()
            .and_then(|path| Config::load(&path).map(|config| (config, path)));
        match loaded {
            Ok((config, path)) => (
                Self {
                    config,
                    config_path: Some(path),
                },
                None,
            ),
            Err(error) => (
                Self {
                    config: Config::default(),
                    config_path: None,
                },
                Some(error),
            ),
        }
    }

    /// The Blish HUD preset, if its path has been set.
    fn blish_hud(&self) -> Option<&CompanionApp> {
        self.config
            .companions
            .iter()
            .find(|app| app.name == BLISH_HUD)
    }

    /// The companion apps to start with `account`.
    fn companions_of(&self, account: &Account) -> Vec<CompanionApp> {
        self.config
            .companions
            .iter()
            .filter(|app| account.companions.contains(&app.id))
            .cloned()
            .collect()
    }

    /// Saves the config; on failure tells the user in a toast.
    fn save(&self, window: &MainWindow) {
        let messages = window.global::<Messages>();
        let detail = match &self.config_path {
            None => messages.invoke_settings_load_failed("".into()),
            Some(path) => match self.config.save(path) {
                Ok(()) => return,
                Err(error) => messages.invoke_settings_save_failed(error.to_string().into()),
            },
        };
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_settings_title(),
            detail,
        );
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let (mut app, load_error) = App::load();
    let window = MainWindow::new()?;
    let messages = window.global::<Messages>();

    if let Some(error) = load_error {
        push_toast(
            &window,
            ToastKind::Error,
            messages.invoke_settings_title(),
            messages.invoke_settings_load_failed(error.to_string().into()),
        );
    }

    match &app.config.gw2_path {
        Some(path) => {
            if let Err(error) = game::validate(path) {
                let (kind, detail) = path_problem(&error);
                push_toast(
                    &window,
                    ToastKind::Warning,
                    messages.invoke_path_unusable_title(),
                    messages.invoke_path_problem(kind, detail),
                );
            }
        }
        None => {
            if let Some(path) = game::detect() {
                app.config.gw2_path = Some(path);
                app.save(&window);
            }
        }
    }

    set_rows(&window, account_rows(&app.config));
    show_gw2_path(&window, app.config.gw2_path.as_deref());
    window.set_blish_path(display_path(app.blish_hud().map(|app| app.exe.as_path())).into());
    refresh(&window);

    let app = Rc::new(RefCell::new(app));
    let queue = Rc::new(LaunchQueue::start(window.as_weak()));

    window.on_launch_account({
        let app = Rc::clone(&app);
        let queue = Rc::clone(&queue);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                toggle_account(&window, &app, &queue, AccountId(id as u32));
            }
        }
    });

    window.on_launch_all({
        let app = Rc::clone(&app);
        let queue = Rc::clone(&queue);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                launch_all(&window, &app, &queue);
            }
        }
    });

    window.on_set_up_login({
        let app = Rc::clone(&app);
        let queue = Rc::clone(&queue);
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

    window.on_toggle_selected({
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                update_row(&window, AccountId(id as u32), |row| {
                    row.selected = !row.selected
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

    window.on_add_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |name, steam| {
            if let Some(window) = weak.upgrade() {
                add_account(&window, &app, name.trim(), steam);
            }
        }
    });

    window.on_choose_gw2_path({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                choose_gw2_path(&window, &app);
            }
        }
    });

    window.on_choose_blish_path({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                choose_blish_path(&window, &app);
            }
        }
    });

    window.on_set_blish({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id, enabled| {
            if let Some(window) = weak.upgrade() {
                set_blish(&window, &app, AccountId(id as u32), enabled);
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

    window.show()?;
    // The native frame exists once the window is shown.
    apply_frame(&window, window.global::<Theme>().get_dark());
    slint::run_event_loop()?;
    window.hide()
}

/// Whether an account in `state` can be started.
fn is_startable(state: AccountState) -> bool {
    matches!(
        state,
        AccountState::Idle | AccountState::NeedsLogin | AccountState::Error
    )
}

/// Whether the account's client is starting, running or being stopped.
fn is_active(state: AccountState) -> bool {
    matches!(
        state,
        AccountState::Starting | AccountState::Running | AccountState::Stopping
    )
}

/// Starts `id` if idle, or requests that its running client stop.
///
/// Stopping only asks the client to terminate; the status returns to idle once the background
/// monitor thread observes the actual exit. A client that is still starting (queued or waiting
/// for its `Local.dat`) can't be stopped or started again until its launch finishes.
fn toggle_account(window: &MainWindow, app: &RefCell<App>, queue: &LaunchQueue, id: AccountId) {
    let Some(row) = row(window, id) else {
        return;
    };
    match row.state {
        AccountState::Running if row.handle != 0 => {
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
        state if is_startable(state) => start_account(window, app, queue, id, LaunchMode::Play),
        _ => {}
    }
}

/// Starts the selected accounts, or all startable ones if nothing is selected.
fn launch_all(window: &MainWindow, app: &RefCell<App>, queue: &LaunchQueue) {
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
fn start_account(
    window: &MainWindow,
    app: &RefCell<App>,
    queue: &LaunchQueue,
    id: AccountId,
    mode: LaunchMode,
) {
    let messages = window.global::<Messages>();
    let (gw2_path, account, companions) = {
        let app = app.borrow();
        let account = app.config.accounts.iter().find(|a| a.id == id).cloned();
        let companions = account
            .as_ref()
            .map(|account| app.companions_of(account))
            .unwrap_or_default();
        (app.config.gw2_path.clone(), account, companions)
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

    update_row(window, id, |row| {
        row.state = AccountState::Starting;
        row.handle = 0;
        row.detail = SharedString::new();
    });
    queue.push(LaunchJob {
        gw2_path,
        account,
        mode,
        companions,
    });
}

#[derive(Debug)]
struct LaunchJob {
    gw2_path: PathBuf,
    account: Account,
    mode: LaunchMode,
    /// Started together with the client, closed after it exits.
    companions: Vec<CompanionApp>,
}

/// Runs launches one after another on a single background thread.
///
/// Each launch blocks for several seconds while `%APPDATA%\Guild Wars 2` points at that account
/// (see [`launcher::launch`]), so launches must never overlap and must never run on the UI
/// thread. A channel keeps them in click order.
#[derive(Debug)]
struct LaunchQueue {
    jobs: mpsc::Sender<LaunchJob>,
}

impl LaunchQueue {
    fn start(window: slint::Weak<MainWindow>) -> Self {
        let (jobs, receiver) = mpsc::channel::<LaunchJob>();
        let shared = Arc::new(SharedInstances::default());
        thread::spawn(move || {
            for job in receiver {
                run_launch(&window, &shared, job);
            }
        });
        Self { jobs }
    }

    fn push(&self, job: LaunchJob) {
        // The worker only stops when the sender is dropped, i.e. when the window is gone.
        let _ = self.jobs.send(job);
    }
}

/// Launches one client (on the queue's worker thread) and hands the result to the UI thread.
fn run_launch(window: &slint::Weak<MainWindow>, shared: &Arc<SharedInstances>, job: LaunchJob) {
    let id = job.account.id;
    let name = job.account.name.clone();

    let launched = match launcher::launch(&job.gw2_path, &job.account, job.mode) {
        Ok(launched) => launched,
        Err(error) => {
            let _ = window.upgrade_in_event_loop(move |window| {
                let messages = window.global::<Messages>();
                let (kind, detail) = launch_failure(&error);
                update_row(&window, id, |row| {
                    row.state = AccountState::Error;
                    row.detail = messages.invoke_start_failed_detail();
                });
                push_toast(
                    &window,
                    ToastKind::Error,
                    messages.invoke_start_failed(),
                    messages.invoke_launch_failure(kind, name.as_str().into(), detail),
                );
            });
            return;
        }
    };

    let pid = launched.client.pid();
    // Slint's `int` is 32-bit; real process handle values comfortably fit in practice (they are
    // small table indices even in a 64-bit process), so this narrowing is safe here.
    let handle = launched.client.raw_handle() as i32;
    let (hour, minute) = bb_win::time::local_hour_minute();
    let since = format!("{hour:02}:{minute:02}");
    let setup = launched.setup;
    let warning = launched.warning;
    let _ = window.upgrade_in_event_loop({
        let name = name.clone();
        move |window| {
            let messages = window.global::<Messages>();
            update_row(&window, id, |row| {
                row.state = AccountState::Running;
                row.handle = handle;
                row.since = since.into();
            });
            if setup {
                push_toast(
                    &window,
                    ToastKind::Info,
                    messages.invoke_setting_up_title(name.as_str().into()),
                    messages.invoke_setting_up(),
                );
            }
            match warning {
                Some(LaunchWarning::SlowStart) => push_toast(
                    &window,
                    ToastKind::Warning,
                    messages.invoke_slow_start_title(),
                    messages.invoke_slow_start(name.as_str().into()),
                ),
                Some(LaunchWarning::ProfileNotRestored(error)) => push_toast(
                    &window,
                    ToastKind::Warning,
                    messages.invoke_profile_restore_title(),
                    messages.invoke_profile_restore(error.to_string().into()),
                ),
                None => {}
            }
        }
    });

    let window = window.clone();
    let mut client = launched.client;
    let mut session =
        companions::Session::new(Arc::clone(shared), job.companions, &job.account, pid);
    thread::spawn(move || {
        let mut errors = session.start(Trigger::ProcessStarted);
        if session.waits_for(Trigger::WindowShown) && client.wait_for_game_window() {
            errors.extend(session.start(Trigger::WindowShown));
        }
        let addon_active = session.has_started();
        let _ = window.upgrade_in_event_loop(move |window| {
            if addon_active {
                update_row(&window, id, |row| row.addon_active = true);
            }
            let messages = window.global::<Messages>();
            for (app, error) in errors {
                push_toast(
                    &window,
                    ToastKind::Error,
                    messages.invoke_companion_failed_title(app.as_str().into()),
                    error.to_string().into(),
                );
            }
        });

        let result = client.wait_for_exit();
        session.stop();
        // Marshal back to the UI thread: Slint's model and window may only be touched there.
        let _ = window.upgrade_in_event_loop(move |window| {
            let messages = window.global::<Messages>();
            let stopped = row(&window, id).is_some_and(|row| row.state == AccountState::Stopping);
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
            update_row(&window, id, |row| {
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
            if let Some(code) = failure {
                push_toast(
                    &window,
                    ToastKind::Error,
                    messages.invoke_exited_title(),
                    messages.invoke_exited(name.as_str().into(), code.into()),
                );
            }
        });
    });
}

/// Maps a launch error to its message and the technical detail filled into it.
fn launch_failure(error: &LaunchError) -> (LaunchFailure, SharedString) {
    let (kind, detail) = match error {
        LaunchError::NoGamePath => (LaunchFailure::NoGamePath, String::new()),
        LaunchError::SteamNotRunning(_) => (LaunchFailure::SteamNotRunning, String::new()),
        LaunchError::SteamInstallMissing(_) => (LaunchFailure::SteamInstallMissing, String::new()),
        LaunchError::SetupNeedsExclusive(_) => (LaunchFailure::SetupNeedsExclusive, String::new()),
        LaunchError::SetupClientRunning => (LaunchFailure::SetupClientRunning, String::new()),
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

fn path_problem(error: &game::Gw2PathError) -> (PathProblem, SharedString) {
    use game::Gw2PathError as E;
    match error {
        E::NotFound => (PathProblem::NotFound, SharedString::new()),
        E::Unreadable(error) => (PathProblem::Unreadable, error.to_string().into()),
        E::NotExecutable => (PathProblem::NotExecutable, SharedString::new()),
        E::Not64Bit => (PathProblem::Not64Bit, SharedString::new()),
        E::NotGw2 => (PathProblem::NotGw2, SharedString::new()),
    }
}

/// State of an account that isn't running: ArenaNet accounts without their own `Local.dat` have
/// to set up their login first. Steam accounts need no login; their first start just creates it.
fn idle_state(id: AccountId, steam: bool) -> AccountState {
    if steam || bb_store::is_set_up(id) {
        AccountState::Idle
    } else {
        AccountState::NeedsLogin
    }
}

fn add_account(window: &MainWindow, app: &RefCell<App>, name: &str, steam: bool) {
    if name.is_empty() {
        return;
    }

    let mut app_ref = app.borrow_mut();
    let next_id = AccountId(
        app_ref
            .config
            .accounts
            .iter()
            .map(|a| a.id.0)
            .max()
            .unwrap_or(0)
            + 1,
    );
    let mut account = Account::new(next_id, name);
    if steam {
        account.provider = Provider::Steam;
    }
    let row = account_row(&account, None);
    app_ref.config.accounts.push(account);
    app_ref.save(window);
    drop(app_ref);

    let mut rows = rows(window);
    rows.push(row);
    set_rows(window, rows);
    refresh(window);
}

/// Shows the system file dialog for an `.exe`; `None` if cancelled or failed (then with a toast).
fn pick_exe(
    window: &MainWindow,
    title: &str,
    filter: (&str, &str),
    initial_dir: Option<&Path>,
) -> Option<PathBuf> {
    let picked = bb_win::dialog::open_file(
        native_handle(window),
        title,
        &[filter, ("Programs", "*.exe")],
        initial_dir,
    );
    match picked {
        Ok(path) => path,
        Err(error) => {
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_settings_title(),
                messages.invoke_dialog_failed(error.to_string().into()),
            );
            None
        }
    }
}

fn choose_gw2_path(window: &MainWindow, app: &RefCell<App>) {
    let initial_dir = app
        .borrow()
        .config
        .gw2_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let Some(path) = pick_exe(
        window,
        "Guild Wars 2",
        ("Guild Wars 2", game::GW2_EXE),
        initial_dir.as_deref(),
    ) else {
        return;
    };

    if let Err(error) = game::validate(&path) {
        let messages = window.global::<Messages>();
        let (kind, detail) = path_problem(&error);
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_path_invalid_title(),
            messages.invoke_path_problem(kind, detail),
        );
        return;
    }

    show_gw2_path(window, Some(&path));
    let mut app = app.borrow_mut();
    app.config.gw2_path = Some(path);
    app.save(window);
}

fn choose_blish_path(window: &MainWindow, app: &RefCell<App>) {
    let initial_dir = app
        .borrow()
        .blish_hud()
        .and_then(|blish| blish.exe.parent())
        .map(Path::to_path_buf);
    let Some(path) = pick_exe(
        window,
        BLISH_HUD,
        (BLISH_HUD, "Blish HUD.exe"),
        initial_dir.as_deref(),
    ) else {
        return;
    };

    let mut app = app.borrow_mut();
    let companions = &mut app.config.companions;
    match companions.iter_mut().find(|app| app.name == BLISH_HUD) {
        Some(blish) => blish.exe = path.clone(),
        None => {
            let id = CompanionId(companions.iter().map(|app| app.id.0).max().unwrap_or(0) + 1);
            companions.push(CompanionApp::blish_hud(id, path.clone()));
        }
    }
    window.set_blish_path(display_path(Some(&path)).into());
    app.save(window);
}

/// Adds Blish HUD to `id`'s companions or removes it. Takes effect from the account's next
/// launch; an instance already running with it is left alone.
fn set_blish(window: &MainWindow, app: &RefCell<App>, id: AccountId, enabled: bool) {
    let mut app_ref = app.borrow_mut();
    let Some(blish) = app_ref.blish_hud().map(|blish| blish.id) else {
        return;
    };
    let Some(account) = app_ref.config.accounts.iter_mut().find(|a| a.id == id) else {
        return;
    };
    account.companions.retain(|&companion| companion != blish);
    if enabled {
        account.companions.push(blish);
    }
    app_ref.save(window);
    drop(app_ref);

    update_row(window, id, |row| row.blish = enabled);
}

fn account_rows(config: &Config) -> Vec<AccountRow> {
    let blish = config
        .companions
        .iter()
        .find(|app| app.name == BLISH_HUD)
        .map(|app| app.id);
    config
        .accounts
        .iter()
        .map(|account| account_row(account, blish))
        .collect()
}

fn account_row(account: &Account, blish: Option<CompanionId>) -> AccountRow {
    let steam = account.provider == Provider::Steam;
    AccountRow {
        id: account.id.0 as i32,
        name: account.name.as_str().into(),
        provider: account.provider.display_name().into(),
        steam,
        state: idle_state(account.id, steam),
        blish: blish.is_some_and(|blish| account.companions.contains(&blish)),
        ..AccountRow::default()
    }
}

/// Rows in every state, for the UI previews.
#[cfg(test)]
fn demo_rows() -> Vec<AccountRow> {
    let row = |id: i32, name: &str, steam: bool, state: AccountState| AccountRow {
        id,
        name: name.into(),
        provider: if steam { "Steam" } else { "ArenaNet" }.into(),
        steam,
        state,
        ..AccountRow::default()
    };
    vec![
        AccountRow {
            blish: true,
            ..row(101, "Main", false, AccountState::Idle)
        },
        AccountRow {
            blish: true,
            ..row(102, "Raid Chrono", false, AccountState::Starting)
        },
        AccountRow {
            blish: true,
            addon_active: true,
            since: "14:02".into(),
            handle: 1,
            ..row(103, "Farm Alt", false, AccountState::Running)
        },
        row(104, "Steam", true, AccountState::NeedsLogin),
        AccountRow {
            selected: true,
            ..row(105, "WvW Guard", false, AccountState::Idle)
        },
        AccountRow {
            detail: "Start failed".into(),
            ..row(106, "Crafting", false, AccountState::Error)
        },
        row(107, "Steam Zweit", true, AccountState::Locked),
    ]
}

/// Reads all rows out of the window's current model.
fn rows(window: &MainWindow) -> Vec<AccountRow> {
    let model = window.get_accounts();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect()
}

fn row(window: &MainWindow, id: AccountId) -> Option<AccountRow> {
    rows(window).into_iter().find(|row| row.id == id.0 as i32)
}

fn set_rows(window: &MainWindow, rows: Vec<AccountRow>) {
    window.set_accounts(Rc::new(slint::VecModel::from(rows)).into());
}

/// Replaces the whole model with one row updated. Rebuilding is simplest and cheap at the
/// list sizes Breakbar deals with (a handful of accounts), and keeps every mutation path
/// (including the one crossing back from a background thread) free of shared, non-`Send`
/// model handles.
fn update_row(window: &MainWindow, id: AccountId, f: impl FnOnce(&mut AccountRow)) {
    let mut rows = rows(window);
    if let Some(row) = rows.iter_mut().find(|row| row.id == id.0 as i32) {
        f(row);
        set_rows(window, rows);
        refresh(window);
    }
}

fn clear_selection(window: &MainWindow) {
    let mut rows = rows(window);
    if rows.iter().any(|row| row.selected) {
        rows.iter_mut().for_each(|row| row.selected = false);
        set_rows(window, rows);
        refresh(window);
    }
}

/// Derives what depends on all rows together: the Steam lock and the header counters.
fn refresh(window: &MainWindow) {
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
fn push_toast(window: &MainWindow, kind: ToastKind, title: SharedString, message: SharedString) {
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
fn expire_toast(window: &MainWindow, id: i32, delay: Duration) {
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

fn dismiss_toast(window: &MainWindow, id: i32) {
    let toasts: Vec<ToastData> = window
        .get_toasts()
        .iter()
        .filter(|toast| toast.id != id)
        .collect();
    window.set_toasts(Rc::new(slint::VecModel::from(toasts)).into());
}

/// Colors the native title bar like the current theme.
fn apply_frame(window: &MainWindow, dark: bool) {
    if let Some(hwnd) = native_handle(window) {
        let colors = if dark { DARK_FRAME } else { LIGHT_FRAME };
        bb_win::window::set_frame(hwnd, dark, colors);
    }
}

fn show_gw2_path(window: &MainWindow, path: Option<&Path>) {
    window.set_gw2_path(display_path(path).into());
    window.set_gw2_path_ok(path.is_some_and(|path| game::validate(path).is_ok()));
}

fn display_path(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// Returns the window's HWND so native dialogs can be made modal to it.
fn native_handle(window: &MainWindow) -> Option<isize> {
    let slint_window = window.window().window_handle();
    let handle = slint_window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

/// Renders the UI into image files without opening a window, to check the design:
/// `cargo test -p breakbar -- --ignored render_ui_previews`, output in `%TEMP%\breakbar-ui`.
#[cfg(test)]
mod preview {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter};
    use slint::{PhysicalSize, Rgb8Pixel};

    struct Offscreen(Rc<MinimalSoftwareWindow>);

    /// One preview image.
    struct Variant {
        name: &'static str,
        dark: bool,
        demo_rows: bool,
        add_page: bool,
        size: (u32, u32),
    }

    impl Platform for Offscreen {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    /// Writes a 24-bit BMP (bottom-up rows, padded to 4 bytes).
    fn write_bmp(path: &Path, width: usize, height: usize, pixels: &[Rgb8Pixel]) {
        let row = (width * 3).div_ceil(4) * 4;
        let size = 54 + row * height;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for y in (0..height).rev() {
            for pixel in &pixels[y * width..(y + 1) * width] {
                out.extend_from_slice(&[pixel.b, pixel.g, pixel.r]);
            }
            out.resize(out.len() + row - width * 3, 0);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    #[ignore = "writes image files for looking at the design"]
    fn render_ui_previews() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
        let dir = std::env::temp_dir().join("breakbar-ui");
        std::fs::create_dir_all(&dir).unwrap();

        let variant = |name, dark, demo_rows, add_page, size| Variant {
            name,
            dark,
            demo_rows,
            add_page,
            size,
        };
        let variants = [
            variant("accounts-dark", true, true, false, (420, 520)),
            variant("accounts-light", false, true, false, (420, 520)),
            variant("empty-dark", true, false, false, (420, 520)),
            variant("empty-light", false, false, false, (420, 520)),
            variant("add-dark", true, false, true, (420, 520)),
            variant("add-light", false, false, true, (420, 520)),
            variant("narrow-dark", true, true, false, (320, 360)),
        ];
        for Variant {
            name,
            dark,
            demo_rows: demo,
            add_page: add,
            size: (width, height),
        } in variants
        {
            window.set_size(PhysicalSize::new(width, height));
            let ui = MainWindow::new().unwrap();
            ui.global::<Theme>().set_dark(dark);
            ui.set_gw2_path(r"C:\Program Files\Guild Wars 2\Gw2-64.exe".into());
            ui.set_gw2_path_ok(true);
            if demo {
                set_rows(&ui, demo_rows());
                let messages = ui.global::<Messages>();
                push_toast(
                    &ui,
                    ToastKind::Warning,
                    messages.invoke_steam_busy_title(),
                    messages.invoke_steam_busy("Steam Zweit".into(), "Steam".into()),
                );
            }
            refresh(&ui);
            if add {
                ui.set_page(ui::Page::AddAccount);
            }
            ui.show().unwrap();
            slint::platform::update_timers_and_animations();

            let (w, h) = (width as usize, height as usize);
            let mut pixels = vec![Rgb8Pixel::default(); w * h];
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, w);
            });
            write_bmp(&dir.join(format!("{name}.bmp")), w, h, &pixels);
            ui.hide().unwrap();
        }
    }
}
