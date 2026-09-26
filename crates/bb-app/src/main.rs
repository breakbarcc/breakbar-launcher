// Release builds use the GUI subsystem so no console window flashes up.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod companions;
mod game;
mod gui;
mod headless;
mod launcher;
mod overlay;
mod profile_link;
mod steam_setup;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

use cli::Command;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(error) => {
            bb_win::console::attach_parent_console();
            eprintln!("error: {error}\n\n{}", cli::HELP);
            return ExitCode::FAILURE;
        }
    };

    if matches!(command, Command::Gui | Command::Launch(_)) {
        // Best-effort: a failure here shows up again when a launch needs the folder.
        if let Err(error) = launcher::recover_profile_link() {
            eprintln!("could not check Guild Wars 2's data folder: {error}");
        }
    }

    match command {
        Command::Help => {
            bb_win::console::attach_parent_console();
            print!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Command::Version => {
            bb_win::console::attach_parent_console();
            println!("breakbar-launcher {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Launch(targets) => headless::run(&targets),
        Command::Gui => match gui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
