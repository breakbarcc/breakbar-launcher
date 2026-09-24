//! The main window and its state.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bb_store::Config;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use ui::{AccountRow, MainWindow};

use crate::game;

/// Code generated from `ui/app.slint`.
///
/// Slint's generated component types don't implement `Debug`, and generated code can't be
/// edited, so our workspace-wide `missing_debug_implementations` lint is disabled here only.
mod ui {
    #![allow(missing_debug_implementations)]
    slint::include_modules!();
}

/// Application state shared between UI callbacks.
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
    window.set_accounts(Rc::new(slint::VecModel::from(account_rows(&app.config))).into());
    window.set_gw2_path(display_path(app.config.gw2_path.as_deref()).into());
    window.set_notice(notice.unwrap_or_default().into());

    let app = Rc::new(RefCell::new(app));

    // Placeholders until the launch pipeline is implemented (phase 3.2+).
    window.on_launch_account(|id| eprintln!("launch account {id}: not implemented yet"));
    window.on_launch_all(|| eprintln!("launch all: not implemented yet"));

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
        })
        .collect()
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
