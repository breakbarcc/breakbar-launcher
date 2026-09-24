//! Spawning game clients and observing their lifetime.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};

use bb_core::{Account, LaunchOptions, game_args};

use crate::profile_link::{self, ProfileLinkError};

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("Guild Wars 2 is not configured yet.")]
    NoGamePath,
    #[error("could not prepare the account's profile folder: {0}")]
    Profile(#[from] bb_store::StoreError),
    #[error("could not switch Guild Wars 2 to this account's profile: {0}")]
    ProfileLink(#[from] ProfileLinkError),
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
/// starts it, so the client finds its side-by-side files. If another client already holds GW2's
/// single-instance mutex, that mutex is closed first (see [`ensure_mutex_clear`]) so this
/// client doesn't just get refused.
///
/// The real `%APPDATA%\Guild Wars 2` is pointed at the account's own profile folder first (see
/// [`crate::profile_link::activate`] for why that, rather than redirecting the child's own
/// `APPDATA`, is what's actually needed), so each account keeps its own `Local.dat` and never
/// overwrites another account's. `TMP`/`TEMP` are additionally redirected to the account's
/// profile, which the client's embedded browser component does respect, keeping concurrent
/// clients from fighting over the same cache. `options.autologin` is overridden based on
/// whether that profile already has a saved login — passing `-autologin` before one exists
/// would do nothing but isn't harmful either, so this only matters for showing the right thing
/// to the user, not for correctness.
///
/// Only one account's data can be linked in at a time, so launching two accounts that have
/// *never* logged in before at the exact same moment can race (see the module-level caveat in
/// [`crate::profile_link`]); accounts that already have a saved login are unaffected once
/// running, since by then they've already read what they need.
pub fn spawn(
    gw2_path: &Path,
    account: &Account,
    mut options: LaunchOptions,
) -> Result<RunningClient, LaunchError> {
    if !gw2_path.is_file() {
        return Err(LaunchError::NoGamePath);
    }
    let working_dir = gw2_path.parent().unwrap_or(gw2_path);

    ensure_mutex_clear(gw2_path);
    profile_link::activate(account.id)?;

    let profile_dir = bb_store::ensure_profile_dir(account.id)?;
    let temp_dir = profile_dir.join("Temp");
    std::fs::create_dir_all(&temp_dir).map_err(LaunchError::Spawn)?;
    options.autologin = bb_store::has_saved_login(account.id);

    let child = Command::new(gw2_path)
        .args(game_args(account, options))
        .current_dir(working_dir)
        .env("TMP", &temp_dir)
        .env("TEMP", &temp_dir)
        .spawn()
        .map_err(LaunchError::Spawn)?;

    Ok(RunningClient {
        pid: child.id(),
        child,
    })
}

/// If a GW2 client currently holds the single-instance mutex, closes that handle so a new
/// client can create its own instead of being refused.
///
/// This looks at every running process with the same executable name, not just ones Breakbar
/// itself started, so a client already running from a previous session (or started outside
/// Breakbar) doesn't block a new launch either. Best-effort: if the check or the close attempt
/// fails, launching proceeds anyway — worst case GW2 itself refuses to start, which is no worse
/// than before this step existed.
fn ensure_mutex_clear(gw2_path: &Path) {
    match bb_win::mutex::gw2_mutex_exists() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("could not check the Guild Wars 2 mutex, launching anyway: {error}");
            return;
        }
    }

    let exe_name = gw2_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(crate::game::GW2_EXE);
    let pids = match bb_win::process::find_processes_by_name(exe_name) {
        Ok(pids) => pids,
        Err(error) => {
            eprintln!("could not list running Guild Wars 2 clients: {error}");
            return;
        }
    };

    for pid in pids {
        match bb_win::mutex::kill_gw2_mutex(pid) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => eprintln!("could not close the mutex held by PID {pid}: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_isolated_appdata;
    use bb_core::AccountId;
    use std::path::PathBuf;

    #[test]
    fn missing_game_path_is_rejected() {
        // No isolation needed: rejected before anything touches a profile.
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
        with_isolated_appdata("spawn-wait", |_| {
            let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
            let stand_in = PathBuf::from(&windir).join("System32").join("ping.exe");
            let account = Account::new(AccountId(1), "Main");

            let running = spawn(&stand_in, &account, LaunchOptions::default()).unwrap();
            assert!(running.pid() > 0);

            running.wait_for_exit().unwrap();
        });
    }

    #[test]
    fn spawn_links_the_account_into_the_real_gw2_appdata_folder() {
        with_isolated_appdata("spawn-profile", |root| {
            let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
            let stand_in = PathBuf::from(&windir).join("System32").join("ping.exe");
            let account = Account::new(AccountId(1), "Main");
            assert!(!bb_store::has_saved_login(account.id));

            let running = spawn(&stand_in, &account, LaunchOptions::default()).unwrap();
            running.wait_for_exit().unwrap();

            let real_gw2_dir = root.join("Roaming").join("Guild Wars 2");
            let account_gw2_dir = bb_store::profile_dir(account.id)
                .unwrap()
                .join("Guild Wars 2");
            assert_eq!(
                std::fs::canonicalize(&real_gw2_dir).unwrap(),
                std::fs::canonicalize(&account_gw2_dir).unwrap(),
                "the real GW2 AppData folder should be linked to the account's profile",
            );
        });
    }
}
