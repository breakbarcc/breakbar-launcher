//! The main window and its state.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::rc::Rc;
use std::thread;

use bb_core::{Account, AccountId, LaunchOptions, Provider};
use bb_store::Config;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, SharedString};
use ui::{AccountRow, MainWindow};

use crate::{game, launcher};

/// Code generated from `ui/app.slint`.
///
/// Slint's generated component types don't implement `Debug`, and generated code can't be
/// edited, so our workspace-wide `missing_debug_implementations` lint is disabled here only.
mod ui {
    #![allow(missing_debug_implementations)]
    slint::include_modules!();
}

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
    fn load() -> (Self, Option<String>) {
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
                Some(format!(
                    "Settings could not be loaded, changes won't be saved: {error}"
                )),
            ),
        }
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.config_path else {
            return Err("Settings could not be loaded, changes won't be saved.".to_owned());
        };
        self.config
            .save(path)
            .map_err(|error| format!("Settings could not be saved: {error}"))
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let (mut app, mut notice) = App::load();

    match &app.config.gw2_path {
        Some(path) => {
            if let Err(error) = game::validate(path) {
                notice = Some(format!("Guild Wars 2 client not usable: {error}"));
            }
        }
        None => {
            if let Some(path) = game::detect() {
                app.config.gw2_path = Some(path);
                if let Err(error) = app.save() {
                    notice.get_or_insert(error);
                }
            }
        }
    }

    let window = MainWindow::new()?;
    set_rows(&window, account_rows(&app.config));
    window.set_gw2_path(display_path(app.config.gw2_path.as_deref()).into());
    window.set_notice(notice.unwrap_or_default().into());

    let app = Rc::new(RefCell::new(app));

    window.on_launch_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                toggle_account(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_launch_all({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                let idle_ids: Vec<AccountId> = account_rows_snapshot(&window)
                    .iter()
                    .filter(|row| !row.running)
                    .map(|row| AccountId(row.id as u32))
                    .collect();
                for id in idle_ids {
                    start_account(&window, &app, id);
                }
            }
        }
    });

    window.on_add_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |name| {
            if let Some(window) = weak.upgrade() {
                add_account(&window, &app, name.trim());
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

    window.run()
}

/// Starts `id` if idle, or requests that its running client stop.
///
/// Stopping only asks the client to terminate; the status returns to "Idle" once the
/// background monitor thread (started in [`start_account`]) observes the actual exit.
fn toggle_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let row = account_rows_snapshot(window)
        .into_iter()
        .find(|row| row.id == id.0 as i32);

    match row {
        Some(row) if row.running => {
            update_row(window, id, |row| row.status = "Stopping…".into());
            if let Err(error) = bb_win::process::terminate(row.handle as isize) {
                window.set_notice(format!("The client could not be stopped: {error}").into());
            }
        }
        _ => start_account(window, app, id),
    }
}

/// Spawns `id`'s client and starts a background thread that reports its exit back to the UI.
fn start_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let (gw2_path, account) = {
        let app = app.borrow();
        (
            app.config.gw2_path.clone(),
            app.config.accounts.iter().find(|a| a.id == id).cloned(),
        )
    };
    let Some(gw2_path) = gw2_path else {
        window.set_notice("Set the Guild Wars 2 path first.".into());
        return;
    };
    let Some(account) = account else {
        return;
    };

    update_row(window, id, |row| row.status = "Starting…".into());

    let running = match launcher::spawn(&gw2_path, &account, LaunchOptions::default()) {
        Ok(running) => running,
        Err(error) => {
            update_row(window, id, |row| row.status = "Idle".into());
            window.set_notice(error.to_string().into());
            return;
        }
    };

    let pid = running.pid();
    // Slint's `int` is 32-bit; real process handle values comfortably fit in practice (they are
    // small table indices even in a 64-bit process), so this narrowing is safe here.
    let handle = running.raw_handle() as i32;
    update_row(window, id, |row| {
        row.running = true;
        row.handle = handle;
        row.status = format!("Running (PID {pid})").into();
    });

    let weak = window.as_weak();
    thread::spawn(move || {
        let result = running.wait_for_exit();
        // Marshal back to the UI thread: Slint's model and window may only be touched there.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                update_row(&window, id, |row| {
                    row.running = false;
                    row.handle = 0;
                    row.status = exit_status_text(&result);
                });
            }
        });
    });
}

fn exit_status_text(result: &io::Result<ExitStatus>) -> SharedString {
    match result {
        Ok(status) if status.success() => "Idle".into(),
        Ok(status) => {
            let code = status
                .code()
                .map_or_else(|| "unknown".to_owned(), |c| c.to_string());
            format!("Exited (code {code})").into()
        }
        Err(error) => format!("Exited: {error}").into(),
    }
}

fn add_account(window: &MainWindow, app: &RefCell<App>, name: &str) {
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
    app_ref.config.accounts.push(Account::new(next_id, name));
    let save_result = app_ref.save();
    drop(app_ref);

    if let Err(error) = save_result {
        window.set_notice(error.into());
    }

    let mut rows = account_rows_snapshot(window);
    rows.push(AccountRow {
        id: next_id.0 as i32,
        name: name.into(),
        provider: Provider::default().display_name().into(),
        status: "Idle".into(),
        running: false,
        handle: 0,
    });
    set_rows(window, rows);
    window.set_new_account_name("".into());
}

fn choose_gw2_path(window: &MainWindow, app: &RefCell<App>) {
    let initial_dir = app
        .borrow()
        .config
        .gw2_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);

    let picked = bb_win::dialog::open_file(
        native_handle(window),
        "Select the Guild Wars 2 client",
        &[
            ("Guild Wars 2 client", game::GW2_EXE),
            ("Programs", "*.exe"),
        ],
        initial_dir.as_deref(),
    );
    let path = match picked {
        Ok(Some(path)) => path,
        Ok(None) => return,
        Err(error) => {
            window.set_notice(format!("The file dialog could not be opened: {error}").into());
            return;
        }
    };

    if let Err(error) = game::validate(&path) {
        window.set_notice(error.to_string().into());
        return;
    }

    let mut app = app.borrow_mut();
    window.set_gw2_path(display_path(Some(&path)).into());
    app.config.gw2_path = Some(path);
    window.set_notice(app.save().err().unwrap_or_default().into());
}

fn account_rows(config: &Config) -> Vec<AccountRow> {
    config
        .accounts
        .iter()
        .map(|account| AccountRow {
            id: account.id.0 as i32,
            name: account.name.as_str().into(),
            provider: account.provider.display_name().into(),
            status: "Idle".into(),
            running: false,
            handle: 0,
        })
        .collect()
}

/// Reads all rows out of the window's current model.
fn account_rows_snapshot(window: &MainWindow) -> Vec<AccountRow> {
    let model = window.get_accounts();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect()
}

fn set_rows(window: &MainWindow, rows: Vec<AccountRow>) {
    window.set_accounts(Rc::new(slint::VecModel::from(rows)).into());
}

/// Replaces the whole model with one row updated. Rebuilding is simplest and cheap at the
/// list sizes Breakbar deals with (a handful of accounts), and keeps every mutation path
/// (including the one crossing back from a background thread) free of shared, non-`Send`
/// model handles.
fn update_row(window: &MainWindow, id: AccountId, f: impl FnOnce(&mut AccountRow)) {
    let mut rows = account_rows_snapshot(window);
    if let Some(row) = rows.iter_mut().find(|row| row.id == id.0 as i32) {
        f(row);
        set_rows(window, rows);
    }
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
