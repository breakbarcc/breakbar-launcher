// Release builds use the GUI subsystem so no console window flashes up.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod companions;
mod game;
mod gui;
mod headless;
mod launcher;
mod log;
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
            let _ = bb_win::console::attach_parent_console();
            eprintln!("error: {error}\n\n{}", cli::HELP);
            return ExitCode::FAILURE;
        }
    };

    if matches!(command, Command::Gui | Command::Launch(_)) {
        // Best-effort: a failure here shows up again when a launch needs the folder.
        match launcher::recover_profile_link() {
            Ok(true) => log::write(
                "pointed Guild Wars 2's data folder back at the shared profile \
                 (an earlier start had not finished)",
            ),
            Ok(false) => {}
            Err(error) => log::write(format!(
                "could not check Guild Wars 2's data folder: {error}"
            )),
        }
    }

    match command {
        Command::Help => {
            let _ = bb_win::console::attach_parent_console();
            print!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Command::Version => {
            let _ = bb_win::console::attach_parent_console();
            println!("breakbar-launcher {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Launch(targets) => headless::run(&targets),
        Command::Gui => match gui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log::write(format!("the window failed: {error}"));
                ExitCode::FAILURE
            }
        },
    }
}
