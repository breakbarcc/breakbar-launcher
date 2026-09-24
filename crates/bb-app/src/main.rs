// Release builds use the GUI subsystem so no console window flashes up.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod game;
mod gui;
mod launcher;

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
        Command::Gui => match gui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
