//! The main window and its state.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use bb_core::{Account, AccountId, BLISH_HUD, CompanionApp, CompanionId, Provider, Scope, Trigger};
use bb_store::{Config, ConfigWriter};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, SharedString, VecModel};
use ui::{
    AccountRow, AccountState, AfterStart, CompanionToggle, EditorData, FpsLimit, LanguageChoice,
    LaunchFailure, LoginState, MainWindow, Messages, PathProblem, SteamSetupStep, Theme,
    ThemeChoice, ToastData, ToastKind,
};

use crate::companions::{self, SharedInstances};
use crate::launcher::{LaunchError, LaunchMode, LaunchWarning};
use crate::{game, launcher, steam_setup};

mod accounts;
mod convert;
mod launch;
mod list;
mod login;
#[cfg(test)]
mod preview;
mod settings;
mod steam_dialog;
#[cfg(test)]
mod tests;
mod texts;
mod tray;

use convert::{
    apply_language, from_ui_after_start, from_ui_fps_limit, from_ui_language, from_ui_theme,
    to_ui_after_start, to_ui_fps_limit, to_ui_language, to_ui_theme,
};
use launch::{launch_all, launch_failure, toggle_account};
use list::{
    account_row, account_rows, apply_account, apply_frame, clear_selection, insert_row,
    is_startable, move_row, push_toast, refresh, remove_row, row, set_rows, update_row,
};
use login::{
    idle_state, login_file_stamp, offer_login_setup, report_login_setup, sync_login_states,
};
use settings::{
    apply_after_start, display_path, path_problem, show_blish_path, show_gw2_path,
    show_overlay_settings,
};
use steam_dialog::steam_ready;
use tray::{tray_activate, tray_menu};

pub(crate) use launch::{LaunchQueue, start_account, stop_account};
pub(crate) use list::{is_active, native_handle, rows};
pub use texts::Texts;

/// Code generated from `ui/app.slint`.
///
/// Slint's generated component types don't implement `Debug`, and generated code can't be
/// edited, so our workspace-wide `missing_debug_implementations` lint is disabled here only.
pub(crate) mod ui {
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
/// How often the logins are checked against the game's build (see `sync_login_states`).
const LOGIN_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// Toasts disappear after this long, unless the pointer is on them.
const TOAST_LIFETIME: Duration = Duration::from_secs(6);

/// Shown in the tray icon's tooltip and used as the `Run` key entry name for starting Breakbar
/// with Windows.
const APP_NAME: &str = "Breakbar Launcher";
/// Named mutex marking a Breakbar GUI as already running (see [`run`]).
const INSTANCE_MUTEX_NAME: &str = "Breakbar-Instance";

/// Breakbar's website, opened from the About section.
const WEBSITE_URL: &str = "https://www.breakbar.cc/";
/// Slint's website, opened from the "Made with Slint" badge.
const SLINT_URL: &str = "https://slint.dev/";
/// The license text in the repository (`repository` of the workspace manifest).
const LICENSE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/blob/main/LICENSE");

/// Application state shared between UI callbacks.
///
/// Only ever touched on the UI thread: Slint callbacks run there, and so does the
/// `invoke_from_event_loop` closure that background launch-monitoring threads hand back.
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) config: Config,
    /// `None` if the config could not be loaded; saving is then disabled so a broken
    /// config file is never overwritten with defaults.
    config_path: Option<PathBuf>,
    /// There was no config file yet: the first-start setup runs.
    first_start: bool,
    /// The account being edited on the editor page.
    draft: Option<Draft>,
    /// The Steam setup dialog is open for this link.
    steam_setup: Option<PendingSteamSetup>,
    /// Accounts still to go through "Set up one after another" after a game update.
    refresh_queue: VecDeque<AccountId>,
    /// Writes the config in the background; `None` until the window exists (see
    /// [`App::start_writer`]), when it is written right away instead.
    writer: Option<ConfigWriter>,
}

/// State behind the Steam setup dialog (see [`steam_setup`]).
#[derive(Debug)]
struct PendingSteamSetup {
    link: steam_setup::Link,
    /// The Steam account whose start opened the dialog.
    account: String,
}

/// Editor state kept on the Rust side (the text fields live in the UI until saved).
#[derive(Debug)]
struct Draft {
    /// `None` while adding a new account.
    id: Option<AccountId>,
    companions: Vec<CompanionId>,
}

impl App {
    fn load() -> (Self, Option<bb_store::StoreError>) {
        let loaded = bb_store::default_config_path().and_then(|path| {
            let first_start = !path.exists();
            Config::load(&path).map(|config| (config, path, first_start))
        });
        match loaded {
            Ok((config, path, first_start)) => (
                Self {
                    config,
                    config_path: Some(path),
                    first_start,
                    draft: None,
                    steam_setup: None,
                    refresh_queue: VecDeque::new(),
                    writer: None,
                },
                None,
            ),
            Err(error) => (
                Self {
                    config: Config::default(),
                    config_path: None,
                    first_start: false,
                    draft: None,
                    steam_setup: None,
                    refresh_queue: VecDeque::new(),
                    writer: None,
                },
                Some(error),
            ),
        }
    }

    fn account(&self, id: AccountId) -> Option<&Account> {
        self.config.accounts.iter().find(|account| account.id == id)
    }

    fn account_index(&self, id: AccountId) -> Option<usize> {
        self.config
            .accounts
            .iter()
            .position(|account| account.id == id)
    }

    fn next_account_id(&self) -> AccountId {
        AccountId(
            self.config
                .accounts
                .iter()
                .map(|account| account.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        )
    }

    /// The Blish HUD preset, if its path has been set.
    fn blish_hud(&self) -> Option<&CompanionApp> {
        self.config
            .companions
            .iter()
            .find(|app| app.name == BLISH_HUD)
    }

    /// Lets the config be written on a background thread from now on, which keeps the window
    /// from stuttering while a slow disk flushes the file. A write that fails is reported with a
    /// toast, like a save that fails at once.
    fn start_writer(&mut self, window: &MainWindow) {
        let weak = window.as_weak();
        self.writer = Some(ConfigWriter::start(move |error| {
            crate::log::write(format!("could not write the settings: {error}"));
            let _ = weak.upgrade_in_event_loop(move |window| {
                let messages = window.global::<Messages>();
                push_toast(
                    &window,
                    ToastKind::Error,
                    messages.invoke_settings_title(),
                    messages.invoke_settings_save_failed(error.to_string().into()),
                );
            });
        }));
    }

    /// Hands the config over for saving to `path`: to the writer thread if there is one.
    fn store(&self, path: &Path) -> Result<(), bb_store::StoreError> {
        match &self.writer {
            Some(writer) => writer.submit(&self.config, path),
            None => self.config.save(path),
        }
    }

    /// Persists the overlay's dragged-to position, best-effort: this runs on every drag, so a
    /// failure isn't worth a toast over something this minor.
    pub(crate) fn save_overlay_position(&mut self, position: bb_store::OverlayPosition) {
        self.config.overlay_position = Some(position);
        if let Some(path) = &self.config_path {
            let _ = self.store(path);
        }
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
    pub(crate) fn save(&self, window: &MainWindow) {
        let messages = window.global::<Messages>();
        let detail = match &self.config_path {
            None => messages.invoke_settings_load_failed("".into()),
            Some(path) => match self.store(path) {
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
    // A second Breakbar process (autostart plus a manual start, a desktop shortcut while it's
    // already running in the tray, ...) asks the first one to show itself instead of opening a
    // second window managing the same accounts.
    if bb_win::mutex::mutex_exists(INSTANCE_MUTEX_NAME).unwrap_or(false) {
        let _ = bb_win::tray::request_show();
        return Ok(());
    }
    // Held for the rest of the process so later starts see the mutex above as existing; a failure
    // here (never observed, but not worth failing over) just means this check stops working.
    let _instance_lock = bb_win::mutex::OwnedMutex::acquire(INSTANCE_MUTEX_NAME).ok();

    let (mut app, load_error) = App::load();
    let window = MainWindow::new()?;
    app.start_writer(&window);
    apply_language(app.config.language);
    check_startup(&window, &mut app, load_error);
    show_initial_state(&window, &app);

    let app = Rc::new(RefCell::new(app));
    sync_login_states(&window, &app);
    let _login_check = start_login_check(&window, &app);
    let queue = Rc::new(LaunchQueue::start(window.as_weak()));
    let overlay = start_overlay(&window, &app, &queue);

    launch::wire(&window, &app, &queue);
    login::wire(&window, &app, &queue);
    steam_dialog::wire(&window, &app);
    list::wire(&window);
    accounts::wire(&window, &app);
    settings::wire(&window, &app, overlay.as_ref());

    // Closing the window never quits Breakbar (it would stop monitoring running clients); it
    // always minimizes to the tray instead. Quitting is only reachable from the tray menu.
    window
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    let _tray = start_tray(&window, &app, &queue);

    window.show()?;
    // The native frame exists once the window is shown.
    apply_frame(&window, window.global::<Theme>().get_dark());
    // `run_event_loop()` exits once the last *Slint* window is hidden - it doesn't know about the
    // tray icon (a plain Win32 window of our own), so with that variant, closing to the tray would
    // quit Breakbar right along with it. This variant only exits on an explicit
    // `quit_event_loop()`, which is exactly what the tray menu's "Quit" calls.
    let result = slint::run_event_loop_until_quit();
    // Ends the writer thread once it has written what is still queued. Closures kept by the tray
    // and the windows hold on to `app`, so it can't be counted on to be dropped at the end.
    app.borrow_mut().writer = None;
    result?;
    window.hide()
}

/// Tells the user what is wrong with the config or the game path, and picks the first page: the
/// setup on the first start, the list otherwise. Without a path, one is detected.
fn check_startup(window: &MainWindow, app: &mut App, load_error: Option<bb_store::StoreError>) {
    let messages = window.global::<Messages>();
    if let Some(error) = load_error {
        push_toast(
            window,
            ToastKind::Error,
            messages.invoke_settings_title(),
            messages.invoke_settings_load_failed(error.to_string().into()),
        );
    }

    match &app.config.gw2_path {
        // First start: the setup asks where Guild Wars 2 is, suggesting what detection finds.
        None if app.first_start => {
            window.set_setup_detected_path(display_path(game::detect().as_deref()).into());
            window.set_page(ui::Page::SetupPath);
        }
        Some(path) => {
            if let Err(error) = game::validate(path) {
                let (kind, detail) = path_problem(&error);
                push_toast(
                    window,
                    ToastKind::Warning,
                    messages.invoke_path_unusable_title(),
                    messages.invoke_path_problem(kind, detail),
                );
            }
        }
        None => {
            if let Some(path) = game::detect() {
                app.config.gw2_path = Some(path);
                app.save(window);
            }
        }
    }
}

/// Shows the config in the window: the accounts and every setting.
fn show_initial_state(window: &MainWindow, app: &App) {
    set_rows(window, account_rows(&app.config));
    show_gw2_path(window, app.config.gw2_path.as_deref());
    show_blish_path(window, app.blish_hud().map(|app| app.exe.as_path()));
    window.set_autostart(bb_win::autostart::is_enabled(APP_NAME));
    window.set_after_start(to_ui_after_start(app.config.after_start));
    window.set_fps_limit(to_ui_fps_limit(app.config.fps_limit));
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    window.set_language(to_ui_language(app.config.language));
    show_overlay_settings(window, app.config.overlay);
    window
        .global::<Theme>()
        .set_choice(to_ui_theme(app.config.theme));
    refresh(window);
}

/// A game update can happen while Breakbar runs (through the game's own launcher or Steam), so
/// the logins are looked at again now and then: a few file times, nothing else. The check runs
/// as long as the returned timer lives.
fn start_login_check(window: &MainWindow, app: &Rc<RefCell<App>>) -> slint::Timer {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, LOGIN_CHECK_INTERVAL, {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                sync_login_states(&window, &app);
            }
        }
    });
    timer
}

/// Opens the instance switcher overlay in the theme of the main window. `None` (with a toast) if
/// it could not be created.
fn start_overlay(
    window: &MainWindow,
    app: &Rc<RefCell<App>>,
    queue: &Rc<LaunchQueue>,
) -> Option<crate::overlay::Overlay> {
    let overlay = match crate::overlay::Overlay::new(window, app, queue) {
        Ok(overlay) => overlay,
        Err(error) => {
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_overlay_failed_title(),
                error.to_string().into(),
            );
            return None;
        }
    };
    if let Some(overlay_window) = overlay.window().upgrade() {
        let theme = to_ui_theme(app.borrow().config.theme);
        overlay_window.global::<Theme>().set_choice(theme);
    }
    Some(overlay)
}

/// Adds the tray icon. `None` (with a toast) if it could not be created; the icon stays as long as
/// the returned value lives.
fn start_tray(
    window: &MainWindow,
    app: &Rc<RefCell<App>>,
    queue: &Rc<LaunchQueue>,
) -> Option<bb_win::tray::Tray> {
    match bb_win::tray::Tray::new(
        APP_NAME,
        tray_activate(window),
        tray_menu(app, queue, window),
    ) {
        Ok(tray) => Some(tray),
        Err(error) => {
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_tray_failed_title(),
                error.to_string().into(),
            );
            None
        }
    }
}
