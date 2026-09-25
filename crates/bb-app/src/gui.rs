//! The main window and its state.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use bb_core::{Account, AccountId, BLISH_HUD, CompanionApp, CompanionId, Provider, Scope, Trigger};
use bb_store::Config;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, SharedString};
use ui::{
    AccountRow, AccountState, AfterStart, CompanionToggle, EditorData, FpsLimit, LaunchFailure,
    LoginState, MainWindow, Messages, PathProblem, SteamSetupStep, Theme, ThemeChoice, ToastData,
    ToastKind,
};

use crate::companions::{self, SharedInstances};
use crate::launcher::{LaunchError, LaunchMode, LaunchWarning};
use crate::{game, launcher, steam_setup};

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
/// Toasts disappear after this long, unless the pointer is on them.
const TOAST_LIFETIME: Duration = Duration::from_secs(6);

/// Shown in the tray icon's tooltip and used as the `Run` key entry name for starting Breakbar
/// with Windows.
const APP_NAME: &str = "Breakbar Launcher";
/// Named mutex marking a Breakbar GUI as already running (see [`run`]).
const INSTANCE_MUTEX_NAME: &str = "Breakbar-Instance";

/// Breakbar's website, opened from the About section.
const WEBSITE_URL: &str = "https://www.breakbar.cc/";
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

    /// Persists the overlay's dragged-to position, best-effort: this runs on every drag, so a
    /// failure isn't worth a toast over something this minor.
    pub(crate) fn save_overlay_position(&mut self, position: bb_store::OverlayPosition) {
        self.config.overlay_position = Some(position);
        if let Some(path) = &self.config_path {
            let _ = self.config.save(path);
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
        // First start: the setup asks where Guild Wars 2 is, suggesting what detection finds.
        None if app.first_start => {
            window.set_setup_detected_path(display_path(game::detect().as_deref()).into());
            window.set_page(ui::Page::SetupPath);
        }
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
    show_blish_path(&window, app.blish_hud().map(|app| app.exe.as_path()));
    window.set_autostart(bb_win::autostart::is_enabled(APP_NAME));
    window.set_after_start(to_ui_after_start(app.config.after_start));
    window.set_fps_limit(to_ui_fps_limit(app.config.fps_limit));
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    window
        .global::<Theme>()
        .set_choice(to_ui_theme(app.config.theme));
    refresh(&window);

    let app = Rc::new(RefCell::new(app));
    let queue = Rc::new(LaunchQueue::start(window.as_weak()));

    let _overlay = match crate::overlay::Overlay::new(&window, &app, &queue) {
        Ok(overlay) => Some(overlay),
        Err(error) => {
            push_toast(
                &window,
                ToastKind::Error,
                messages.invoke_overlay_failed_title(),
                error.to_string().into(),
            );
            None
        }
    };
    if let Some(overlay) = _overlay
        .as_ref()
        .and_then(|overlay| overlay.window().upgrade())
    {
        let theme = to_ui_theme(app.borrow().config.theme);
        overlay.global::<Theme>().set_choice(theme);
    }

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

    window.on_steam_setup_create_link({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                steam_setup_create_link(&window, &app);
            }
        }
    });

    window.on_steam_setup_done({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                steam_setup_done(&window, &app);
            }
        }
    });

    window.on_steam_setup_cancel({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                close_steam_setup(&window, &app);
            }
        }
    });

    window.on_login_offer_accept({
        let app = Rc::clone(&app);
        let queue = Rc::clone(&queue);
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

    window.on_edit_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                let id = (id != 0).then_some(AccountId(id as u32));
                open_editor(&window, &app, id);
            }
        }
    });

    window.on_editor_toggle_companion({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |companion| {
            if let Some(window) = weak.upgrade() {
                editor_toggle_companion(&window, &app, CompanionId(companion as u32));
            }
        }
    });

    window.on_editor_save({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |name, steam, args| {
            if let Some(window) = weak.upgrade() {
                editor_save(&window, &app, name.trim(), steam, args.trim());
            }
        }
    });

    window.on_duplicate_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                duplicate_account(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_move_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id, index| {
            if let Some(window) = weak.upgrade() {
                move_account(&window, &app, AccountId(id as u32), index);
            }
        }
    });

    window.on_delete_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                delete_account(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_create_shortcut({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                create_shortcut(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_open_profile_folder({
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                open_profile_folder(&window, AccountId(id as u32));
            }
        }
    });

    window.on_setup_browse({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                setup_browse(&window);
            }
        }
    });

    window.on_setup_path_chosen({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |path| {
            if let Some(window) = weak.upgrade() {
                let path = PathBuf::from(path.as_str());
                show_gw2_path(&window, Some(&path));
                let mut app = app.borrow_mut();
                app.config.gw2_path = Some(path);
                app.save(&window);
                window.set_page(ui::Page::SetupAccount);
            }
        }
    });

    window.on_setup_create_account({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |name, steam| {
            if let Some(window) = weak.upgrade() {
                let mut account = Account::new(app.borrow().next_account_id(), name.trim());
                if steam {
                    account.provider = Provider::Steam;
                }
                let (id, name) = (account.id, account.name.clone());
                insert_account(&window, &app, account, None);
                window.set_page(ui::Page::Accounts);
                offer_login_setup(&window, id, &name, steam);
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

    window.on_toggle_autostart({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                toggle_autostart(&window);
            }
        }
    });

    window.on_set_after_start({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.set_after_start(value);
                let mut app = app.borrow_mut();
                app.config.after_start = from_ui_after_start(value);
                app.save(&window);
            }
        }
    });

    window.on_open_website(|| open_url(WEBSITE_URL));
    window.on_open_license(|| open_url(LICENSE_URL));

    window.on_set_theme({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        let overlay = _overlay.as_ref().map(crate::overlay::Overlay::window);
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.global::<Theme>().set_choice(value);
                if let Some(overlay) = overlay.as_ref().and_then(slint::Weak::upgrade) {
                    overlay.global::<Theme>().set_choice(value);
                }
                let mut app = app.borrow_mut();
                app.config.theme = from_ui_theme(value);
                app.save(&window);
            }
        }
    });

    window.on_set_fps_limit({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.set_fps_limit(value);
                let mut app = app.borrow_mut();
                app.config.fps_limit = from_ui_fps_limit(value);
                app.save(&window);
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

    // Closing the window never quits Breakbar (it would stop monitoring running clients); it
    // always minimizes to the tray instead. Quitting is only reachable from the tray menu.
    window
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    let _tray = match bb_win::tray::Tray::new(
        APP_NAME,
        tray_activate(&window),
        tray_menu(&app, &queue, &window),
    ) {
        Ok(tray) => Some(tray),
        Err(error) => {
            push_toast(
                &window,
                ToastKind::Error,
                messages.invoke_tray_failed_title(),
                error.to_string().into(),
            );
            None
        }
    };

    window.show()?;
    // The native frame exists once the window is shown.
    apply_frame(&window, window.global::<Theme>().get_dark());
    // `run_event_loop()` exits once the last *Slint* window is hidden - it doesn't know about the
    // tray icon (a plain Win32 window of our own), so with that variant, closing to the tray would
    // quit Breakbar right along with it. This variant only exits on an explicit
    // `quit_event_loop()`, which is exactly what the tray menu's "Quit" calls.
    slint::run_event_loop_until_quit()?;
    window.hide()
}

/// Left click / double-click on the tray icon: show and focus the main window.
fn tray_activate(window: &MainWindow) -> impl FnMut() + 'static {
    let weak = window.as_weak();
    move || {
        if let Some(window) = weak.upgrade() {
            let _ = window.show();
        }
    }
}

/// Builds the tray's right-click menu fresh on every open, so it reflects current account state
/// (design hand-off screen 05): "Launch all" with a counter, one entry per account (locked ones
/// disabled), then "Open window" and "Quit".
fn tray_menu(
    app: &Rc<RefCell<App>>,
    queue: &Rc<LaunchQueue>,
    window: &MainWindow,
) -> impl FnMut() -> Vec<bb_win::tray::MenuItem> + 'static {
    let app = Rc::clone(app);
    let queue = Rc::clone(queue);
    let weak = window.as_weak();
    move || {
        let Some(window) = weak.upgrade() else {
            return Vec::new();
        };
        let messages = window.global::<Messages>();
        let all_rows = rows(&window);
        let startable = all_rows
            .iter()
            .filter(|row| is_startable(row.state))
            .count();

        let mut items = vec![bb_win::tray::MenuItem::entry(
            messages.invoke_tray_launch_all(startable as i32),
            startable > 0,
            {
                let app = Rc::clone(&app);
                let queue = Rc::clone(&queue);
                let weak = weak.clone();
                move || {
                    if let Some(window) = weak.upgrade() {
                        launch_all(&window, &app, &queue);
                    }
                }
            },
        )];

        if !all_rows.is_empty() {
            items.push(bb_win::tray::MenuItem::separator());
            for row in all_rows {
                let id = AccountId(row.id as u32);
                let label = messages.invoke_tray_account(row.name.clone(), row.state);
                let locked = row.state == AccountState::Locked;
                items.push(bb_win::tray::MenuItem::entry(label, !locked, {
                    let app = Rc::clone(&app);
                    let queue = Rc::clone(&queue);
                    let weak = weak.clone();
                    move || {
                        if let Some(window) = weak.upgrade() {
                            toggle_account(&window, &app, &queue, id);
                        }
                    }
                }));
            }
        }

        items.push(bb_win::tray::MenuItem::separator());
        items.push(bb_win::tray::MenuItem::entry(
            messages.invoke_tray_open_window(),
            true,
            {
                let weak = weak.clone();
                move || {
                    if let Some(window) = weak.upgrade() {
                        let _ = window.show();
                    }
                }
            },
        ));
        items.push(bb_win::tray::MenuItem::entry(
            messages.invoke_tray_quit(),
            true,
            || {
                let _ = slint::quit_event_loop();
            },
        ));
        items
    }
}

fn to_ui_after_start(value: bb_store::AfterStart) -> AfterStart {
    match value {
        bb_store::AfterStart::KeepOpen => AfterStart::KeepOpen,
        bb_store::AfterStart::MinimizeToTray => AfterStart::MinimizeToTray,
        bb_store::AfterStart::Close => AfterStart::Close,
    }
}

fn to_ui_theme(value: bb_store::ThemeChoice) -> ThemeChoice {
    match value {
        bb_store::ThemeChoice::System => ThemeChoice::System,
        bb_store::ThemeChoice::Light => ThemeChoice::Light,
        bb_store::ThemeChoice::Dark => ThemeChoice::Dark,
    }
}

fn from_ui_theme(value: ThemeChoice) -> bb_store::ThemeChoice {
    match value {
        ThemeChoice::System => bb_store::ThemeChoice::System,
        ThemeChoice::Light => bb_store::ThemeChoice::Light,
        ThemeChoice::Dark => bb_store::ThemeChoice::Dark,
    }
}

fn to_ui_fps_limit(value: bb_store::FpsLimit) -> FpsLimit {
    match value {
        bb_store::FpsLimit::Fps60 => FpsLimit::Fps60,
        bb_store::FpsLimit::Fps30 => FpsLimit::Fps30,
        bb_store::FpsLimit::Unlimited => FpsLimit::Unlimited,
    }
}

fn from_ui_fps_limit(value: FpsLimit) -> bb_store::FpsLimit {
    match value {
        FpsLimit::Fps60 => bb_store::FpsLimit::Fps60,
        FpsLimit::Fps30 => bb_store::FpsLimit::Fps30,
        FpsLimit::Unlimited => bb_store::FpsLimit::Unlimited,
    }
}

fn from_ui_after_start(value: AfterStart) -> bb_store::AfterStart {
    match value {
        AfterStart::KeepOpen => bb_store::AfterStart::KeepOpen,
        AfterStart::MinimizeToTray => bb_store::AfterStart::MinimizeToTray,
        AfterStart::Close => bb_store::AfterStart::Close,
    }
}

/// Applies the "after starting an account" setting once a `Play` launch has been queued (not for
/// setup launches, and not for a manual stop).
fn apply_after_start(window: &MainWindow) {
    match window.get_after_start() {
        AfterStart::KeepOpen => {}
        AfterStart::MinimizeToTray => {
            let _ = window.hide();
        }
        AfterStart::Close => {
            let _ = slint::quit_event_loop();
        }
    }
}

/// Translated texts for starts without a window (desktop shortcuts, command line). Uses a window
/// instance that is never shown, because that is where Slint keeps the translated texts.
pub struct Texts(MainWindow);

impl std::fmt::Debug for Texts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Texts")
    }
}

impl Texts {
    pub fn new() -> Result<Self, slint::PlatformError> {
        MainWindow::new().map(Self)
    }

    pub fn start_failed(&self) -> String {
        self.0.global::<Messages>().invoke_start_failed().into()
    }

    pub fn launch_failure(&self, error: &LaunchError, name: &str) -> String {
        let (kind, detail) = launch_failure(error);
        self.0
            .global::<Messages>()
            .invoke_launch_failure(kind, name.into(), detail)
            .into()
    }

    pub fn companion_failed(&self, app: &str, error: &std::io::Error) -> String {
        let title = self
            .0
            .global::<Messages>()
            .invoke_companion_failed_title(app.into());
        format!("{title}: {error}")
    }

    pub fn unknown_account(&self, name: &str) -> String {
        self.0
            .global::<Messages>()
            .invoke_unknown_account(name.into())
            .into()
    }

    pub fn settings_load_failed(&self, detail: &str) -> String {
        self.0
            .global::<Messages>()
            .invoke_settings_load_failed(detail.into())
            .into()
    }
}

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

/// Whether a Steam account can be started now. If Steam has no copy of the game yet, opens the
/// setup dialog that links the ArenaNet installation into Steam and returns `false`. Cases the
/// dialog can't help with (no Steam at all) pass, so the launch reports them itself.
fn steam_ready(
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
fn steam_setup_create_link(window: &MainWindow, app: &RefCell<App>) {
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
fn steam_setup_done(window: &MainWindow, app: &RefCell<App>) {
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

fn close_steam_setup(window: &MainWindow, app: &RefCell<App>) {
    app.borrow_mut().steam_setup = None;
    window.set_steam_setup(SteamSetupStep::Hidden);
    window.set_steam_setup_retry(false);
}

#[derive(Debug)]
struct LaunchJob {
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
    let login_before = login_file_stamp(id);

    let launched = match launcher::launch(&job.gw2_path, &job.account, job.mode, job.fps_limit) {
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
    let steam = job.account.provider == Provider::Steam;
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
                window.set_login_setup_name(name.as_str().into());
                window.set_login_setup_steam(steam);
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
        let errors = companions::start_for(&mut session, &mut client);
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
            if setup {
                window.set_login_setup_name(SharedString::new());
                if !stopped && failure.is_none() {
                    report_login_setup(&window, id, &name, steam, login_before);
                }
            }
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

/// Size and modification time of the account's `Local.dat`, `None` if it has none.
fn login_file_stamp(id: AccountId) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(bb_store::local_dat_path(id).ok()?).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Tells the user how a login setup ended. The client saves the login into `Local.dat` when it
/// is closed normally, so a file that didn't change means nothing was saved. That can't show
/// whether the password was stored, only that the client wrote its file.
fn report_login_setup(
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
fn offer_login_setup(window: &MainWindow, id: AccountId, name: &str, steam: bool) {
    window.set_login_offer_name(name.into());
    window.set_login_offer_steam(steam);
    window.set_login_offer_id(id.0 as i32);
}

/// Maps a launch error to its message and the technical detail filled into it.
fn launch_failure(error: &LaunchError) -> (LaunchFailure, SharedString) {
    let (kind, detail) = match error {
        LaunchError::NoGamePath => (LaunchFailure::NoGamePath, String::new()),
        LaunchError::SteamNotRunning(_) => (LaunchFailure::SteamNotRunning, String::new()),
        LaunchError::SteamInstallMissing(_) => (LaunchFailure::SteamInstallMissing, String::new()),
        LaunchError::SetupNeedsExclusive(_) => (LaunchFailure::SetupNeedsExclusive, String::new()),
        LaunchError::SetupClientRunning => (LaunchFailure::SetupClientRunning, String::new()),
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

/// Adds `account` to the config and the list, at `index` (the end if `None`).
fn insert_account(window: &MainWindow, app: &RefCell<App>, account: Account, index: Option<usize>) {
    let mut app_ref = app.borrow_mut();
    let row = account_row(&account, &app_ref.config.companions);
    let index = index
        .unwrap_or(app_ref.config.accounts.len())
        .min(app_ref.config.accounts.len());
    app_ref.config.accounts.insert(index, account);
    app_ref.save(window);
    drop(app_ref);

    let mut rows = rows(window);
    rows.insert(index.min(rows.len()), row);
    set_rows(window, rows);
    refresh(window);
}

/// Opens the editor for account `id`, or for a new account if `None`.
fn open_editor(window: &MainWindow, app: &RefCell<App>, id: Option<AccountId>) {
    let mut app_ref = app.borrow_mut();
    let account = match id {
        Some(id) => match app_ref.account(id) {
            Some(account) => Some(account.clone()),
            None => return,
        },
        None => None,
    };
    let draft = Draft {
        id,
        companions: account
            .as_ref()
            .map(|account| account.companions.clone())
            .unwrap_or_default(),
    };
    let active = id
        .and_then(|id| row(window, id))
        .is_some_and(|row| is_active(row.state));
    window.set_editor(editor_data(
        &app_ref.config,
        account.as_ref(),
        &draft,
        active,
    ));
    app_ref.draft = Some(draft);
    window.set_page(ui::Page::Editor);
}

fn editor_data(
    config: &Config,
    account: Option<&Account>,
    draft: &Draft,
    active: bool,
) -> EditorData {
    let steam = account.is_some_and(|account| account.provider == Provider::Steam);
    let login = match account {
        _ if steam => LoginState::Steam,
        Some(account) if bb_store::is_set_up(account.id) => LoginState::SetUp,
        _ => LoginState::Missing,
    };
    let companions: Vec<CompanionToggle> = config
        .companions
        .iter()
        .map(|app| CompanionToggle {
            id: app.id.0 as i32,
            name: app.name.as_str().into(),
            per_client: app.scope == Scope::PerClient,
            after_game_start: app.start_when == Trigger::WindowShown,
            enabled: draft.companions.contains(&app.id),
        })
        .collect();
    EditorData {
        id: draft.id.map_or(0, |id| id.0 as i32),
        name: account.map_or_else(SharedString::new, |account| account.name.as_str().into()),
        steam,
        args: account.map_or_else(SharedString::new, |account| {
            account.extra_args.as_str().into()
        }),
        login,
        active,
        companions: Rc::new(slint::VecModel::from(companions)).into(),
    }
}

fn editor_toggle_companion(window: &MainWindow, app: &RefCell<App>, companion: CompanionId) {
    let mut app_ref = app.borrow_mut();
    let Some(draft) = app_ref.draft.as_mut() else {
        return;
    };
    if let Some(position) = draft.companions.iter().position(|&id| id == companion) {
        draft.companions.remove(position);
    } else {
        draft.companions.push(companion);
    }
    let enabled = draft.companions.contains(&companion);

    let editor = window.get_editor();
    let toggles: Vec<CompanionToggle> = editor
        .companions
        .iter()
        .map(|mut toggle| {
            if toggle.id == companion.0 as i32 {
                toggle.enabled = enabled;
            }
            toggle
        })
        .collect();
    window.set_editor(EditorData {
        companions: Rc::new(slint::VecModel::from(toggles)).into(),
        ..editor
    });
}

/// Saves the editor: creates the new account or updates the edited one.
fn editor_save(window: &MainWindow, app: &RefCell<App>, name: &str, steam: bool, args: &str) {
    if name.is_empty() {
        return;
    }
    let Some(draft) = app.borrow_mut().draft.take() else {
        return;
    };
    let provider = if steam {
        Provider::Steam
    } else {
        Provider::ArenaNet
    };

    match draft.id {
        None => {
            let mut account = Account::new(app.borrow().next_account_id(), name);
            account.provider = provider;
            account.extra_args = args.to_owned();
            account.companions = draft.companions;
            let (id, name) = (account.id, account.name.clone());
            insert_account(window, app, account, None);
            offer_login_setup(window, id, &name, steam);
        }
        Some(id) => {
            let mut app_ref = app.borrow_mut();
            let Some(index) = app_ref.account_index(id) else {
                return;
            };
            let account = &mut app_ref.config.accounts[index];
            account.name = name.to_owned();
            account.provider = provider;
            account.extra_args = args.to_owned();
            account.companions = draft.companions;
            let account = account.clone();
            app_ref.save(window);
            let companions = app_ref.config.companions.clone();
            drop(app_ref);

            update_row(window, id, |row| {
                apply_account(row, &account, &companions);
                // The platform decides whether a missing login matters.
                if matches!(row.state, AccountState::Idle | AccountState::NeedsLogin) {
                    row.state = idle_state(id, row.steam);
                }
            });
        }
    }
    window.set_page(ui::Page::Accounts);
}

/// Copies an account's settings (not its login) into a new account right below it.
fn duplicate_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let (copy, index) = {
        let app_ref = app.borrow();
        let (Some(source), Some(index)) = (app_ref.account(id), app_ref.account_index(id)) else {
            return;
        };
        let name = window
            .global::<Messages>()
            .invoke_copy_name(source.name.as_str().into());
        let mut copy = Account::new(app_ref.next_account_id(), name.as_str());
        copy.provider = source.provider;
        copy.extra_args = source.extra_args.clone();
        copy.companions = source.companions.clone();
        (copy, index + 1)
    };
    insert_account(window, app, copy, Some(index));
}

/// Moves account `id` to list position `index` (clamped to the list).
fn move_account(window: &MainWindow, app: &RefCell<App>, id: AccountId, index: i32) {
    let mut app_ref = app.borrow_mut();
    let Some(from) = app_ref.account_index(id) else {
        return;
    };
    let last = app_ref.config.accounts.len() - 1;
    let to = (index.max(0) as usize).min(last);
    if from == to {
        return;
    }
    let account = app_ref.config.accounts.remove(from);
    app_ref.config.accounts.insert(to, account);
    app_ref.save(window);
    drop(app_ref);

    let mut rows = rows(window);
    if let Some(from) = rows.iter().position(|row| row.id == id.0 as i32) {
        let row = rows.remove(from);
        rows.insert(to.min(rows.len()), row);
        set_rows(window, rows);
    }
}

/// Deletes the account and, for good, its profile folder with the saved login. The UI has asked
/// the user first.
fn delete_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let messages = window.global::<Messages>();
    let Some(row) = row(window, id) else {
        return;
    };
    if is_active(row.state) {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_delete_running(row.name),
            SharedString::new(),
        );
        return;
    }

    let mut app_ref = app.borrow_mut();
    let Some(index) = app_ref.account_index(id) else {
        return;
    };
    app_ref.config.accounts.remove(index);
    app_ref.save(window);
    if app_ref
        .draft
        .as_ref()
        .is_some_and(|draft| draft.id == Some(id))
    {
        app_ref.draft = None;
        window.set_page(ui::Page::Accounts);
    }
    drop(app_ref);

    let rows: Vec<AccountRow> = rows(window)
        .into_iter()
        .filter(|row| row.id != id.0 as i32)
        .collect();
    set_rows(window, rows);
    refresh(window);

    match bb_store::delete_profile(id) {
        Ok(()) => push_toast(
            window,
            ToastKind::Success,
            messages.invoke_deleted_title(row.name),
            SharedString::new(),
        ),
        Err(error) => push_toast(
            window,
            ToastKind::Error,
            messages.invoke_delete_failed_title(),
            error.to_string().into(),
        ),
    }
}

/// Puts a shortcut on the desktop that starts the account without opening the launcher.
fn create_shortcut(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let messages = window.global::<Messages>();
    let Some(name) = app.borrow().account(id).map(|account| account.name.clone()) else {
        return;
    };
    let result = (|| -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let desktop = bb_win::shortcut::desktop_dir().map_err(|error| error.to_string())?;
        let file = format!("{} (Breakbar).lnk", file_name_safe(&name));
        let description = messages.invoke_shortcut_description(name.as_str().into());
        bb_win::shortcut::create(
            &desktop.join(&file),
            &bb_win::shortcut::Shortcut {
                target: &exe,
                arguments: &format!("--launch-id {}", id.0),
                working_dir: exe.parent().unwrap_or(&desktop),
                description: &description,
                icon: &exe,
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(file)
    })();
    match result {
        Ok(file) => push_toast(
            window,
            ToastKind::Success,
            messages.invoke_shortcut_created_title(),
            messages.invoke_shortcut_created(file.into()),
        ),
        Err(error) => push_toast(
            window,
            ToastKind::Error,
            messages.invoke_shortcut_failed_title(),
            error.into(),
        ),
    }
}

/// Replaces characters Windows doesn't allow in file names.
fn file_name_safe(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_owned()
}

fn open_profile_folder(window: &MainWindow, id: AccountId) {
    let result = bb_store::ensure_profile_dir(id)
        .map_err(|error| error.to_string())
        .and_then(|dir| {
            std::process::Command::new("explorer.exe")
                .arg(dir)
                .spawn()
                .map(drop)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        push_toast(
            window,
            ToastKind::Error,
            window.global::<Messages>().invoke_folder_failed_title(),
            error.into(),
        );
    }
}

/// First-start setup: choose the client by hand and check it.
fn setup_browse(window: &MainWindow) {
    let Some(path) = pick_exe(
        window,
        "Guild Wars 2",
        ("Guild Wars 2", game::GW2_EXE),
        None,
    ) else {
        return;
    };
    window.set_setup_other_path(display_path(Some(&path)).into());
    let error = match game::validate(&path) {
        Ok(()) => SharedString::new(),
        Err(error) => {
            let (kind, detail) = path_problem(&error);
            window
                .global::<Messages>()
                .invoke_path_problem(kind, detail)
        }
    };
    window.set_setup_other_error(error);
}

/// Shows the system file dialog for an `.exe`; `None` if cancelled or failed (then with a toast).
fn pick_exe(
    window: &MainWindow,
    title: &str,
    filter: (&str, &str),
    initial_dir: Option<&Path>,
) -> Option<PathBuf> {
    let picked = bb_win::dialog::open_file(
        native_handle(window.window()),
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
    show_blish_path(window, Some(&path));
    app.save(window);
}

/// Flips the `Run` key entry that starts Breakbar with Windows. The registry is the source of
/// truth (like the path fields above), not the config file, so this stays correct even if the
/// install is moved without opening Breakbar in between.
fn toggle_autostart(window: &MainWindow) {
    let enable = !window.get_autostart();
    let result = if enable {
        std::env::current_exe().and_then(|exe| bb_win::autostart::enable(APP_NAME, &exe))
    } else {
        bb_win::autostart::disable(APP_NAME)
    };
    match result {
        Ok(()) => window.set_autostart(enable),
        Err(error) => {
            window.set_autostart(bb_win::autostart::is_enabled(APP_NAME));
            let messages = window.global::<Messages>();
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_autostart_failed_title(),
                messages.invoke_autostart_failed(error.to_string().into()),
            );
        }
    }
}

fn account_rows(config: &Config) -> Vec<AccountRow> {
    config
        .accounts
        .iter()
        .map(|account| account_row(account, &config.companions))
        .collect()
}

/// The row of an account that isn't running.
fn account_row(account: &Account, companions: &[CompanionApp]) -> AccountRow {
    let mut row = AccountRow {
        id: account.id.0 as i32,
        ..AccountRow::default()
    };
    apply_account(&mut row, account, companions);
    row.state = idle_state(account.id, row.steam);
    row
}

/// Copies what the row shows of the account's settings into `row`.
fn apply_account(row: &mut AccountRow, account: &Account, companions: &[CompanionApp]) {
    row.name = account.name.as_str().into();
    row.provider = account.provider.display_name().into();
    row.steam = account.provider == Provider::Steam;
    row.has_companions = companions
        .iter()
        .any(|app| account.companions.contains(&app.id));
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
            has_companions: true,
            ..row(101, "Main", false, AccountState::Idle)
        },
        AccountRow {
            has_companions: true,
            ..row(102, "Raid Chrono", false, AccountState::Starting)
        },
        AccountRow {
            has_companions: true,
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
pub(crate) fn rows(window: &MainWindow) -> Vec<AccountRow> {
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
    if let Some(hwnd) = native_handle(window.window()) {
        let colors = if dark { DARK_FRAME } else { LIGHT_FRAME };
        bb_win::window::set_frame(hwnd, dark, colors);
    }
}

fn show_gw2_path(window: &MainWindow, path: Option<&Path>) {
    window.set_gw2_path(display_path(path).into());
    window.set_gw2_path_ok(path.is_some_and(|path| game::validate(path).is_ok()));
}

/// Blish HUD isn't validated the way the game client is (no `validate` for arbitrary programs);
/// the settings page just shows whether the configured file still exists.
fn show_blish_path(window: &MainWindow, path: Option<&Path>) {
    window.set_blish_path(display_path(path).into());
    window.set_blish_path_ok(path.is_some_and(Path::is_file));
}

fn display_path(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// Opens `url` in the default browser. Best-effort: a failure has nothing useful to tell the user.
fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
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

/// Renders the UI into image files without opening a window, to check the design:
/// `cargo test -p breakbar-launcher -- --ignored render_ui_previews`, output in `%TEMP%\breakbar-ui`.
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
        page: ui::Page,
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
        // For the README screenshots: `BREAKBAR_PREVIEW_LANG=en` renders the source language, and
        // `BREAKBAR_PREVIEW_SCALE=2` renders at twice the resolution, and
        // `BREAKBAR_PREVIEW_NO_TOAST=1` leaves out the demo toast.
        let language = std::env::var("BREAKBAR_PREVIEW_LANG").ok();
        let scale: f32 = std::env::var("BREAKBAR_PREVIEW_SCALE")
            .ok()
            .and_then(|scale| scale.parse().ok())
            .unwrap_or(1.0);
        let physical = |logical: u32| (logical as f32 * scale).round() as u32;
        window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });

        let variant = |name, dark, demo_rows, page, size| Variant {
            name,
            dark,
            demo_rows,
            page,
            size,
        };
        use ui::Page::{Accounts, Editor, Settings, SetupAccount, SetupPath};
        let variants = [
            variant("accounts-dark", true, true, Accounts, (420, 600)),
            variant("accounts-light", false, true, Accounts, (420, 600)),
            variant("empty-dark", true, false, Accounts, (420, 520)),
            variant("empty-light", false, false, Accounts, (420, 520)),
            variant("editor-dark", true, false, Editor, (420, 780)),
            variant("editor-light", false, false, Editor, (420, 780)),
            variant("setup-path-dark", true, false, SetupPath, (420, 520)),
            variant(
                "setup-account-light",
                false,
                false,
                SetupAccount,
                (420, 520),
            ),
            variant("narrow-dark", true, true, Accounts, (320, 360)),
            variant("settings-dark", true, false, Settings, (420, 1060)),
            variant("settings-light", false, false, Settings, (420, 1060)),
            variant("settings-top-light", false, false, Settings, (420, 600)),
            variant("editor-steam-dark", true, false, Editor, (420, 780)),
            variant("login-offer-dark", true, true, Accounts, (420, 520)),
            variant("login-offer-steam-light", false, true, Accounts, (420, 520)),
            variant("login-banner-dark", true, true, Accounts, (420, 520)),
            variant(
                "login-banner-steam-light",
                false,
                true,
                Accounts,
                (420, 520),
            ),
            variant("steam-link-dark", true, true, Accounts, (420, 520)),
            variant("steam-install-light", false, true, Accounts, (420, 520)),
        ];
        for Variant {
            name,
            dark,
            demo_rows: demo,
            page,
            size: (width, height),
        } in variants
        {
            window.set_size(PhysicalSize::new(physical(width), physical(height)));
            let ui = MainWindow::new().unwrap();
            if let Some(language) = &language {
                slint::select_bundled_translation(language).unwrap();
            }
            ui.global::<Theme>().set_choice(if dark {
                ThemeChoice::Dark
            } else {
                ThemeChoice::Light
            });
            ui.set_gw2_path(r"C:\Program Files\Guild Wars 2\Gw2-64.exe".into());
            ui.set_gw2_path_ok(true);
            ui.set_blish_path(r"D:\Tools\Blish HUD\Blish HUD.exe".into());
            ui.set_blish_path_ok(name != "settings-dark");
            ui.set_autostart(name == "settings-dark");
            ui.set_after_start(AfterStart::MinimizeToTray);
            ui.set_app_version(env!("CARGO_PKG_VERSION").into());
            ui.set_setup_detected_path(r"C:\Program Files\Guild Wars 2\Gw2-64.exe".into());
            if demo {
                set_rows(&ui, demo_rows());
                if std::env::var_os("BREAKBAR_PREVIEW_NO_TOAST").is_none() {
                    let messages = ui.global::<Messages>();
                    push_toast(
                        &ui,
                        ToastKind::Warning,
                        messages.invoke_steam_busy_title(),
                        messages.invoke_steam_busy("Steam Zweit".into(), "Steam".into()),
                    );
                }
            }
            refresh(&ui);
            ui.set_editor(EditorData {
                id: 1,
                name: "Main".into(),
                steam: name == "editor-steam-dark",
                args: "-windowed -mapLoadinfo".into(),
                login: if name == "editor-steam-dark" {
                    LoginState::Steam
                } else {
                    LoginState::SetUp
                },
                active: false,
                companions: Rc::new(slint::VecModel::from(vec![CompanionToggle {
                    id: 1,
                    name: "Blish HUD".into(),
                    per_client: true,
                    after_game_start: true,
                    enabled: true,
                }]))
                .into(),
            });
            ui.set_page(page);
            if name.starts_with("login-offer") {
                ui.set_login_offer_id(1);
                ui.set_login_offer_name("Main".into());
                ui.set_login_offer_steam(name.contains("steam"));
            }
            if name.starts_with("login-banner") {
                ui.set_login_setup_name("Main".into());
                ui.set_login_setup_steam(name.contains("steam"));
            }
            if name.starts_with("steam-") {
                ui.set_steam_setup_account("Steam Acc".into());
                ui.set_steam_setup_link(
                    r"C:\Program Files (x86)\Steam\steamapps\common\Guild Wars 2".into(),
                );
                ui.set_steam_setup_target(r"C:\Games\Guild Wars\Guild Wars 2".into());
                ui.set_steam_setup_retry(name == "steam-install-light");
                ui.set_steam_setup(if name == "steam-link-dark" {
                    SteamSetupStep::Link
                } else {
                    SteamSetupStep::Install
                });
            }
            ui.show().unwrap();
            slint::platform::update_timers_and_animations();

            let (w, h) = (physical(width) as usize, physical(height) as usize);
            let mut pixels = vec![Rgb8Pixel::default(); w * h];
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, w);
            });
            write_bmp(&dir.join(format!("{name}.bmp")), w, h, &pixels);
            ui.hide().unwrap();
        }

        // The overlay is a separate top-level component (its own window), so it's rendered the
        // same way but outside the `MainWindow` loop above.
        for (name, dark) in [("overlay-dark", true), ("overlay-light", false)] {
            let (width, height) = (260, 32);
            window.set_size(PhysicalSize::new(physical(width), physical(height)));
            let overlay = ui::OverlaySwitcher::new().unwrap();
            if let Some(language) = &language {
                slint::select_bundled_translation(language).unwrap();
            }
            overlay.global::<Theme>().set_choice(if dark {
                ThemeChoice::Dark
            } else {
                ThemeChoice::Light
            });
            overlay.set_entries(
                Rc::new(slint::VecModel::from(vec![
                    ui::SwitcherEntry {
                        id: 1,
                        name: "Main".into(),
                        running: true,
                        active: true,
                        starting: false,
                    },
                    ui::SwitcherEntry {
                        id: 2,
                        name: "Raid Chrono".into(),
                        running: true,
                        active: false,
                        starting: false,
                    },
                    ui::SwitcherEntry {
                        id: 3,
                        name: "Farm Alt".into(),
                        running: true,
                        active: false,
                        starting: true,
                    },
                    ui::SwitcherEntry {
                        id: 4,
                        name: "Steam".into(),
                        running: false,
                        active: false,
                        starting: false,
                    },
                ]))
                .into(),
            );
            overlay.show().unwrap();
            slint::platform::update_timers_and_animations();

            let (w, h) = (physical(width) as usize, physical(height) as usize);
            let mut pixels = vec![Rgb8Pixel::default(); w * h];
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, w);
            });
            write_bmp(&dir.join(format!("{name}.bmp")), w, h, &pixels);
            overlay.hide().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_file_names_drop_forbidden_characters() {
        assert_eq!(file_name_safe("Main"), "Main");
        assert_eq!(
            file_name_safe(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        // Windows drops trailing dots and spaces from file names.
        assert_eq!(file_name_safe("Alt. "), "Alt");
    }
}
