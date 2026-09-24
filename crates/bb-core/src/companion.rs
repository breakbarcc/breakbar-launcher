//! Companion apps started together with a game client (e.g. Blish HUD).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

impl CompanionApp {
    /// Preset for Blish HUD: one instance per client, attached via PID and MumbleLink name.
    pub fn blish_hud(id: CompanionId, exe: impl Into<PathBuf>) -> Self {
        Self {
            id,
            name: "Blish HUD".to_owned(),
            exe: exe.into(),
            args: "--pid {pid} --mumble {mumble}".to_owned(),
            scope: Scope::PerClient,
            start_when: Trigger::WindowShown,
            close_with_game: true,
        }
    }

    /// Expands the argument template for a concrete client.
    pub fn expand_args(&self, ctx: &ArgContext<'_>) -> String {
        self.args
            .replace("{pid}", &ctx.pid.to_string())
            .replace("{mumble}", ctx.mumble)
            .replace("{account}", ctx.account)
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
        assert_eq!(blish.expand_args(&ctx), "--pid 4242 --mumble Breakbar_3");
    }
}
