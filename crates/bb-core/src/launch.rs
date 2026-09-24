//! Building the game's command line.

use crate::account::{Account, Provider};

/// Per-launch switches that depend on runtime state rather than on the account itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchOptions {
    /// Open `Gw2.dat` read-only so several clients can share it.
    pub share_archive: bool,
    /// Log in automatically with the credentials remembered in `Local.dat`.
    pub autologin: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            share_archive: true,
            autologin: true,
        }
    }
}

/// Builds the command line arguments (without the executable) for launching `account`.
pub fn game_command_line(account: &Account, options: LaunchOptions) -> String {
    let mut args = Vec::with_capacity(6);
    if options.share_archive {
        args.push("-shareArchive".to_owned());
    }
    if options.autologin && account.provider == Provider::ArenaNet {
        args.push("-autologin".to_owned());
    }
    if account.provider == Provider::Steam {
        args.push("-provider Steam".to_owned());
    }
    args.push(format!("-mumble \"{}\"", account.mumble_link_name()));
    let extra = account.extra_args.trim();
    if !extra.is_empty() {
        args.push(extra.to_owned());
    }
    args.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountId;

    #[test]
    fn arenanet_account_uses_autologin() {
        let account = Account::new(AccountId(1), "Main");
        assert_eq!(
            game_command_line(&account, LaunchOptions::default()),
            r#"-shareArchive -autologin -mumble "Breakbar_1""#
        );
    }

    #[test]
    fn steam_account_uses_provider_instead_of_autologin() {
        let mut account = Account::new(AccountId(2), "Steam");
        account.provider = Provider::Steam;
        account.extra_args = " -dx11 ".to_owned();
        assert_eq!(
            game_command_line(&account, LaunchOptions::default()),
            r#"-shareArchive -provider Steam -mumble "Breakbar_2" -dx11"#
        );
    }

    #[test]
    fn first_launch_can_skip_share_archive() {
        let account = Account::new(AccountId(3), "Alt");
        let options = LaunchOptions {
            share_archive: false,
            autologin: false,
        };
        assert_eq!(
            game_command_line(&account, options),
            r#"-mumble "Breakbar_3""#
        );
    }
}
