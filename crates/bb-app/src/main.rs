// Release builds use the GUI subsystem so no console window flashes up.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;

use std::process::ExitCode;
use std::rc::Rc;

use bb_store::Config;
use cli::Command;
use slint::ComponentHandle;
use ui::{AccountRow, MainWindow};

/// Code generated from `ui/app.slint`.
///
/// Slint's generated component types don't implement `Debug`, and generated code can't be
/// edited, so our workspace-wide `missing_debug_implementations` lint is disabled here only.
mod ui {
    #![allow(missing_debug_implementations)]
    slint::include_modules!();
}

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(error) => {
            bb_win::console::attach_parent_console();
            eprintln!("error: {error}\n\n{}", cli::HELP);
            return ExitCode::FAILURE;
        }
    };

    match command {
        Command::Help => {
            bb_win::console::attach_parent_console();
            print!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Command::Version => {
            bb_win::console::attach_parent_console();
            println!("breakbar {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Launch(names) => {
            bb_win::console::attach_parent_console();
            eprintln!("launching from the command line is not implemented yet: {names:?}");
            ExitCode::FAILURE
        }
        Command::Gui => match run_gui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_gui() -> Result<(), slint::PlatformError> {
    let config = bb_store::default_config_path()
        .and_then(|path| Config::load(&path))
        .unwrap_or_else(|error| {
            eprintln!("failed to load config, using defaults: {error}");
            Config::default()
        });

    let window = MainWindow::new()?;

    let rows: Vec<AccountRow> = config
        .accounts
        .iter()
        .map(|account| AccountRow {
            id: account.id.0 as i32,
            name: account.name.as_str().into(),
            provider: account.provider.display_name().into(),
            status: "Idle".into(),
        })
        .collect();
    window.set_accounts(Rc::new(slint::VecModel::from(rows)).into());
    window.set_gw2_path(
        config
            .gw2_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
            .into(),
    );

    // Placeholders until the launch pipeline is implemented (phase 3).
    window.on_launch_account(|id| eprintln!("launch account {id}: not implemented yet"));
    window.on_launch_all(|| eprintln!("launch all: not implemented yet"));
    window.on_choose_gw2_path(|| eprintln!("choose GW2 path: not implemented yet"));

    window.run()
}
