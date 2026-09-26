//! Starting accounts without the window: desktop shortcuts and `breakbar-launcher --launch`.
//!
//! The process stays in the background until the clients it started have exited, so their
//! companion apps (Blish HUD) are started and closed with them, exactly as from the window.
//! Launches are serialized with the window's through the launch lock (see [`launcher::launch`]).
//! Errors are shown in a message box, since there is no window to show them in.

use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;

use bb_core::Account;
use bb_store::Config;

use crate::cli::LaunchTarget;
use crate::companions::{self, SharedInstances};
use crate::gui::Texts;
use crate::launcher::{self, LaunchMode};

const TITLE: &str = "Breakbar Launcher";

pub fn run(targets: &[LaunchTarget]) -> ExitCode {
    let texts = Texts::new().ok();
    let text = |f: &dyn Fn(&Texts) -> String, fallback: String| texts.as_ref().map_or(fallback, f);

    let config = match bb_store::default_config_path().and_then(|path| Config::load(&path)) {
        Ok(config) => config,
        Err(error) => {
            let detail = error.to_string();
            bb_win::dialog::error_box(
                TITLE,
                &text(&|texts| texts.settings_load_failed(&detail), detail.clone()),
            );
            return ExitCode::FAILURE;
        }
    };

    let mut accounts: Vec<&Account> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for target in targets {
        if let Some(account) = resolve(&config, target) {
            accounts.push(account);
        } else {
            let name = target.to_string();
            errors.push(text(
                &|texts| texts.unknown_account(&name),
                format!("There is no account \"{name}\"."),
            ));
        }
    }

    let Some(gw2_path) = config.gw2_path.clone() else {
        errors.push(text(
            &|texts| texts.launch_failure(&launcher::LaunchError::NoGamePath, ""),
            launcher::LaunchError::NoGamePath.to_string(),
        ));
        show_errors(texts.as_ref(), &errors);
        return ExitCode::FAILURE;
    };

    let shared = Arc::new(SharedInstances::default());
    let (companion_errors, receiver) = mpsc::channel::<(String, std::io::Error)>();
    let mut monitors = Vec::new();
    for account in accounts {
        match launcher::launch(
            &gw2_path,
            account,
            LaunchMode::Play,
            config.fps_limit.frames_per_second(),
        ) {
            Ok(launched) => {
                let apps = config
                    .companions
                    .iter()
                    .filter(|app| account.companions.contains(&app.id))
                    .cloned()
                    .collect();
                let mut client = launched.client;
                let mut session =
                    companions::Session::new(Arc::clone(&shared), apps, account, client.pid());
                let sender = companion_errors.clone();
                monitors.push(thread::spawn(move || {
                    for error in companions::start_for(&mut session, &mut client) {
                        let _ = sender.send(error);
                    }
                    let _ = client.wait_for_exit();
                    session.stop();
                }));
            }
            Err(error) => {
                crate::log::write(format!("starting {} failed: {error}", account.name));
                errors.push(text(
                    &|texts| texts.launch_failure(&error, &account.name),
                    error.to_string(),
                ));
            }
        }
    }
    drop(companion_errors);

    let failed = !errors.is_empty();
    if failed {
        show_errors(texts.as_ref(), &errors);
    }
    // Companion errors arrive while the clients run; the loop ends once every monitor is done.
    for (app, error) in receiver {
        let message = text(
            &|texts| texts.companion_failed(&app, &error),
            format!("{app}: {error}"),
        );
        show_errors(texts.as_ref(), &[message]);
    }
    for monitor in monitors {
        let _ = monitor.join();
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn resolve<'a>(config: &'a Config, target: &LaunchTarget) -> Option<&'a Account> {
    config.accounts.iter().find(|account| match target {
        LaunchTarget::Id(id) => account.id.0 == *id,
        LaunchTarget::Name(name) => account.name.eq_ignore_ascii_case(name),
    })
}

fn show_errors(texts: Option<&Texts>, errors: &[String]) {
    let title = texts.map_or_else(|| "Start failed".to_owned(), Texts::start_failed);
    bb_win::dialog::error_box(&format!("{TITLE} – {title}"), &errors.join("\n\n"));
}
