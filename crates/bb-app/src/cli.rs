//! Command line interface.

use std::ffi::OsString;

pub const HELP: &str = "\
Breakbar - Guild Wars 2 multi-launcher

USAGE:
    breakbar [OPTIONS]

OPTIONS:
    -l, --launch <NAMES>    Launch the given accounts (comma separated) without opening the window
        --launch-id <ID>    Launch the account with this id without opening the window
                            (what desktop shortcuts use; ids survive renaming)
    -h, --help              Print this help
    -V, --version           Print the version
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Gui,
    Help,
    Version,
    Launch(Vec<LaunchTarget>),
}

/// An account to launch from the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    Name(String),
    Id(u32),
}

impl std::fmt::Display for LaunchTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchTarget::Name(name) => f.write_str(name),
            LaunchTarget::Id(id) => write!(f, "#{id}"),
        }
    }
}

/// Parses the process arguments. The first item is the program name and is skipped.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, lexopt::Error> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_iter(args);
    let mut command = Command::Gui;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => return Ok(Command::Help),
            Short('V') | Long("version") => return Ok(Command::Version),
            Short('l') | Long("launch") => {
                let value = parser.value()?.string()?;
                let names: Vec<LaunchTarget> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| LaunchTarget::Name(name.to_owned()))
                    .collect();
                if names.is_empty() {
                    return Err("--launch requires at least one account name".into());
                }
                command = Command::Launch(names);
            }
            Long("launch-id") => {
                let id = parser.value()?.parse()?;
                command = Command::Launch(vec![LaunchTarget::Id(id)]);
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, lexopt::Error> {
        parse(
            std::iter::once("breakbar")
                .chain(args.iter().copied())
                .map(OsString::from),
        )
    }

    #[test]
    fn no_arguments_opens_gui() {
        assert_eq!(parse_args(&[]).unwrap(), Command::Gui);
    }

    #[test]
    fn launch_splits_and_trims_names() {
        assert_eq!(
            parse_args(&["--launch", "Main, Alt 1,,"]).unwrap(),
            Command::Launch(vec![
                LaunchTarget::Name("Main".into()),
                LaunchTarget::Name("Alt 1".into())
            ])
        );
    }

    #[test]
    fn launch_id_is_parsed() {
        assert_eq!(
            parse_args(&["--launch-id", "3"]).unwrap(),
            Command::Launch(vec![LaunchTarget::Id(3)])
        );
        assert!(parse_args(&["--launch-id", "x"]).is_err());
    }

    #[test]
    fn empty_launch_list_is_rejected() {
        assert!(parse_args(&["-l", " , "]).is_err());
    }

    #[test]
    fn unknown_option_is_rejected() {
        assert!(parse_args(&["--nope"]).is_err());
    }
}
