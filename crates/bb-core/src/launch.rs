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

/// Builds the individual command line arguments for launching `account`.
///
/// Each element is a single argv entry (no manual quoting): pass them to a process API that
/// quotes as needed, such as [`std::process::Command::args`].
pub fn game_args(account: &Account, options: LaunchOptions) -> Vec<String> {
    let mut args = Vec::with_capacity(8);
    if options.share_archive {
        args.push("-shareArchive".to_owned());
    }
    if options.autologin && account.provider == Provider::ArenaNet {
        args.push("-autologin".to_owned());
    }
    if account.provider == Provider::Steam {
        args.push("-provider".to_owned());
        args.push("Steam".to_owned());
    }
    args.push("-mumble".to_owned());
    args.push(account.mumble_link_name());
    args.extend(split_args(&account.extra_args));
    args
}

/// Splits a free-form argument string into individual tokens, similar to a shell's word
/// splitting: whitespace separates tokens, and `"..."` groups a token that contains whitespace.
/// There is no escape character; a quote cannot be embedded in a quoted token.
pub fn split_args(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        let mut token = String::new();
        if c == '"' {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                token.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        tokens.push(token);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountId;

    #[test]
    fn arenanet_account_uses_autologin() {
        let account = Account::new(AccountId(1), "Main");
        assert_eq!(
            game_args(&account, LaunchOptions::default()),
            ["-shareArchive", "-autologin", "-mumble", "Breakbar_1"]
        );
    }

    #[test]
    fn steam_account_uses_provider_instead_of_autologin() {
        let mut account = Account::new(AccountId(2), "Steam");
        account.provider = Provider::Steam;
        account.extra_args = " -dx11 ".to_owned();
        assert_eq!(
            game_args(&account, LaunchOptions::default()),
            [
                "-shareArchive",
                "-provider",
                "Steam",
                "-mumble",
                "Breakbar_2",
                "-dx11"
            ]
        );
    }

    #[test]
    fn first_launch_can_skip_share_archive() {
        let account = Account::new(AccountId(3), "Alt");
        let options = LaunchOptions {
            share_archive: false,
            autologin: false,
        };
        assert_eq!(game_args(&account, options), ["-mumble", "Breakbar_3"]);
    }

    #[test]
    fn split_args_handles_quoted_tokens() {
        assert_eq!(
            split_args(r#"  -windowed  -customText "hello world"  "#),
            ["-windowed", "-customText", "hello world"]
        );
    }

    #[test]
    fn split_args_of_empty_string_is_empty() {
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn split_args_tolerates_unterminated_quote() {
        assert_eq!(split_args(r#"-a "open end"#), ["-a", "open end"]);
    }
}
