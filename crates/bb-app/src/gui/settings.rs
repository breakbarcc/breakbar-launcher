//! The settings page and the first-start setup: paths, autostart, overlay, what happens after a start.

use super::{
    APP_NAME, AfterStart, App, BLISH_HUD, CompanionApp, CompanionId, ComponentHandle, LICENSE_URL,
    MainWindow, Messages, Path, PathBuf, PathProblem, Rc, RefCell, SLINT_URL, SharedString, Theme,
    ToastKind, WEBSITE_URL, apply_language, from_ui_after_start, from_ui_fps_limit,
    from_ui_language, from_ui_theme, game, native_handle, push_toast, ui,
};

/// Shows the overlay settings on the settings page. The overlay window itself picks them up from
/// the config on its next update, which this asks for.
pub(super) fn show_overlay_settings(window: &MainWindow, settings: bb_store::OverlaySettings) {
    window.set_overlay_enabled(settings.enabled);
    window.set_overlay_only_running(settings.only_when_running);
    window.set_overlay_locked(settings.lock_position);
    window.set_overlay_opacity(i32::from(settings.opacity_percent()));
    crate::overlay::wake();
}

/// A callback that changes one of the on/off overlay settings and saves the config.
pub(super) fn overlay_setting(
    app: &Rc<RefCell<App>>,
    window: &MainWindow,
    change: impl Fn(&mut bb_store::OverlaySettings, bool) + 'static,
) -> impl FnMut(bool) + 'static {
    let app = Rc::clone(app);
    let weak = window.as_weak();
    move |value| {
        if let Some(window) = weak.upgrade() {
            let mut app = app.borrow_mut();
            change(&mut app.config.overlay, value);
            show_overlay_settings(&window, app.config.overlay);
            app.save(&window);
        }
    }
}

/// Applies the "after starting an account" setting once a `Play` launch has been queued (not for
/// setup launches, and not for a manual stop).
pub(super) fn apply_after_start(window: &MainWindow) {
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

pub(super) fn path_problem(error: &game::Gw2PathError) -> (PathProblem, SharedString) {
    use game::Gw2PathError as E;
    match error {
        E::NotFound => (PathProblem::NotFound, SharedString::new()),
        E::Unreadable(error) => (PathProblem::Unreadable, error.to_string().into()),
        E::NotExecutable => (PathProblem::NotExecutable, SharedString::new()),
        E::Not64Bit => (PathProblem::Not64Bit, SharedString::new()),
        E::NotGw2 => (PathProblem::NotGw2, SharedString::new()),
    }
}

/// First-start setup: choose the client by hand and check it.
pub(super) fn setup_browse(window: &MainWindow) {
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
pub(super) fn pick_exe(
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

pub(super) fn choose_gw2_path(window: &MainWindow, app: &RefCell<App>) {
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

pub(super) fn choose_blish_path(window: &MainWindow, app: &RefCell<App>) {
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
    if let Some(blish) = companions.iter_mut().find(|app| app.name == BLISH_HUD) {
        blish.exe.clone_from(&path);
    } else {
        let id = CompanionId(companions.iter().map(|app| app.id.0).max().unwrap_or(0) + 1);
        companions.push(CompanionApp::blish_hud(id, path.clone()));
    }
    show_blish_path(window, Some(&path));
    app.save(window);
}

/// Flips the `Run` key entry that starts Breakbar with Windows. The registry is the source of
/// truth (like the path fields above), not the config file, so this stays correct even if the
/// install is moved without opening Breakbar in between.
pub(super) fn toggle_autostart(window: &MainWindow) {
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

pub(super) fn show_gw2_path(window: &MainWindow, path: Option<&Path>) {
    window.set_gw2_path(display_path(path).into());
    window.set_gw2_path_ok(path.is_some_and(|path| game::validate(path).is_ok()));
}

/// Blish HUD isn't validated the way the game client is (no `validate` for arbitrary programs);
/// the settings page just shows whether the configured file still exists.
pub(super) fn show_blish_path(window: &MainWindow, path: Option<&Path>) {
    window.set_blish_path(display_path(path).into());
    window.set_blish_path_ok(path.is_some_and(Path::is_file));
}

pub(super) fn display_path(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// Opens `url` in the default browser. Best-effort: a failure has nothing useful to tell the user.
pub(super) fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(
    window: &MainWindow,
    app: &Rc<RefCell<App>>,
    overlay: Option<&crate::overlay::Overlay>,
) {
    window.on_setup_browse({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                setup_browse(&window);
            }
        }
    });

    window.on_setup_path_chosen({
        let app = Rc::clone(app);
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

    window.on_choose_gw2_path({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                choose_gw2_path(&window, &app);
            }
        }
    });

    window.on_choose_blish_path({
        let app = Rc::clone(app);
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
        let app = Rc::clone(app);
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
    window.on_open_slint(|| open_url(SLINT_URL));

    wire_overlay(window, app);
    wire_appearance(window, app, overlay);
}

/// Connects the overlay settings.
fn wire_overlay(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_set_overlay_enabled(overlay_setting(app, window, |settings, value| {
        settings.enabled = value;
    }));
    window.on_set_overlay_only_running(overlay_setting(app, window, |settings, value| {
        settings.only_when_running = value;
    }));
    window.on_set_overlay_locked(overlay_setting(app, window, |settings, value| {
        settings.lock_position = value;
    }));
    window.on_set_overlay_opacity({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |percent| {
            if let Some(window) = weak.upgrade() {
                let mut app = app.borrow_mut();
                app.config.overlay.idle_opacity = percent.clamp(
                    i32::from(bb_store::OverlaySettings::MIN_OPACITY),
                    i32::from(bb_store::OverlaySettings::MAX_OPACITY),
                ) as u8;
                show_overlay_settings(&window, app.config.overlay);
                app.save(&window);
            }
        }
    });
}

/// Connects language, theme and the frame rate limit.
fn wire_appearance(
    window: &MainWindow,
    app: &Rc<RefCell<App>>,
    overlay: Option<&crate::overlay::Overlay>,
) {
    window.on_set_language({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |value| {
            if let Some(window) = weak.upgrade() {
                window.set_language(value);
                let choice = from_ui_language(value);
                apply_language(choice);
                let mut app = app.borrow_mut();
                app.config.language = choice;
                app.save(&window);
            }
        }
    });

    window.on_set_theme({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        let overlay = overlay.map(crate::overlay::Overlay::window);
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
        let app = Rc::clone(app);
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
}
