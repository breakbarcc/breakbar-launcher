//! Companion apps started together with a game client (e.g. Blish HUD).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::launch::split_args;

/// Stable identifier of a companion app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompanionId(pub u32);

/// How many instances of a companion app run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// One instance per game client, bound to that client's lifetime.
    #[default]
    PerClient,
    /// A single instance shared by all clients, closed after the last client exits.
    Shared,
}

/// When a companion app is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Right after the game process has been created.
    ProcessStarted,
    /// As soon as the game window is shown.
    #[default]
    WindowShown,
}

/// A program launched alongside the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionApp {
    pub id: CompanionId,
    pub name: String,
    pub exe: PathBuf,
    /// Argument template. Supported placeholders: `{pid}`, `{mumble}`, `{account}`.
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub start_when: Trigger,
    #[serde(default = "default_true")]
    pub close_with_game: bool,
}

fn default_true() -> bool {
    true
}

/// Values substituted into a companion app's argument template.
#[derive(Debug, Clone, Copy)]
pub struct ArgContext<'a> {
    pub pid: u32,
    pub mumble: &'a str,
    pub account: &'a str,
}

/// Display name of the Blish HUD preset.
pub const BLISH_HUD: &str = "Blish HUD";

impl CompanionApp {
    /// Preset for Blish HUD: one instance per client, attached via PID and `MumbleLink` name.
    ///
    /// Blish HUD's single-instance mutex includes the `MumbleLink` name, so one instance per
    /// client works as long as every client has its own name (see
    /// [`crate::Account::mumble_link_name`]). It waits for the game window itself, but starting
    /// it only once that window is shown keeps it from starting for a client that never gets
    /// past the launcher.
    pub fn blish_hud(id: CompanionId, exe: impl Into<PathBuf>) -> Self {
        Self {
            id,
            name: BLISH_HUD.to_owned(),
            exe: exe.into(),
            args: "--pid {pid} --mumble {mumble}".to_owned(),
            scope: Scope::PerClient,
            start_when: Trigger::WindowShown,
            close_with_game: true,
        }
    }

    /// Expands the argument template for a concrete client into individual argv entries.
    ///
    /// The template is split first, so a substituted value containing spaces (such as an
    /// account name) stays a single argument.
    #[must_use]
    pub fn expand_args(&self, ctx: &ArgContext<'_>) -> Vec<String> {
        let pid = ctx.pid.to_string();
        split_args(&self.args)
            .into_iter()
            .map(|token| {
                token
                    .replace("{pid}", &pid)
                    .replace("{mumble}", ctx.mumble)
                    .replace("{account}", ctx.account)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blish_hud_args_are_expanded() {
        let blish = CompanionApp::blish_hud(CompanionId(1), r"C:\Blish HUD\Blish HUD.exe");
        let ctx = ArgContext {
            pid: 4242,
            mumble: "Breakbar_3",
            account: "Main",
        };
        assert_eq!(
            blish.expand_args(&ctx),
            ["--pid", "4242", "--mumble", "Breakbar_3"]
        );
    }

    #[test]
    fn substituted_values_stay_single_arguments() {
        let mut app = CompanionApp::blish_hud(CompanionId(1), "x.exe");
        app.args = r#"--settings "C:\Blish\{account}" --pid {pid}"#.to_owned();
        let ctx = ArgContext {
            pid: 7,
            mumble: "Breakbar_1",
            account: "My Alt",
        };
        assert_eq!(
            app.expand_args(&ctx),
            ["--settings", r"C:\Blish\My Alt", "--pid", "7"]
        );
    }
}
