//! Spawning game clients and observing their lifetime.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};

use bb_core::{Account, LaunchOptions, game_args};

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("Guild Wars 2 is not configured yet.")]
    NoGamePath,
    #[error("failed to start Guild Wars 2: {0}")]
    Spawn(#[source] io::Error),
}

/// A game client that was just started.
///
/// [`RunningClient::wait_for_exit`] must be called (on a background thread) to observe when the
/// client exits; dropping this value without waiting leaks no resources but never reports exit.
#[derive(Debug)]
pub struct RunningClient {
    pid: u32,
    child: Child,
}

impl RunningClient {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Raw OS handle of the client process, valid until this value (or the `Child` it borrows
    /// from) is dropped. Used to stop the process without the PID-reuse race described above.
    pub fn raw_handle(&self) -> isize {
        self.child.as_raw_handle() as isize
    }

    /// Blocks the calling thread until the client process exits.
    pub fn wait_for_exit(mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// Starts `account`'s game client.
///
/// The working directory is set to the client's own folder, matching how the ArenaNet launcher
/// starts it, so the client finds its side-by-side files.
pub fn spawn(
    gw2_path: &Path,
    account: &Account,
    options: LaunchOptions,
) -> Result<RunningClient, LaunchError> {
    if !gw2_path.is_file() {
        return Err(LaunchError::NoGamePath);
    }
    let working_dir = gw2_path.parent().unwrap_or(gw2_path);

    let child = Command::new(gw2_path)
        .args(game_args(account, options))
        .current_dir(working_dir)
        .spawn()
        .map_err(LaunchError::Spawn)?;

    Ok(RunningClient {
        pid: child.id(),
        child,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_core::AccountId;
    use std::path::PathBuf;

    #[test]
    fn missing_game_path_is_rejected() {
        let account = Account::new(AccountId(1), "Main");
        let result = spawn(
            &PathBuf::from(r"C:\does\not\exist\Gw2-64.exe"),
            &account,
            LaunchOptions::default(),
        );
        assert!(matches!(result, Err(LaunchError::NoGamePath)));
    }

    /// Stands in for the game client: any executable can be waited on the same way, so the
    /// spawn → observe-exit path is exercised without starting GW2. `ping.exe` rejects the
    /// (nonsensical, for it) GW2 switches `spawn` always prepends and exits immediately with
    /// an error, which is all this test needs: a fast, deterministic exit.
    #[test]
    fn spawned_process_can_be_waited_on() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let stand_in = PathBuf::from(&windir).join("System32").join("ping.exe");
        let account = Account::new(AccountId(1), "Main");

        let running = spawn(&stand_in, &account, LaunchOptions::default()).unwrap();
        assert!(running.pid() > 0);

        running.wait_for_exit().unwrap();
    }
}
