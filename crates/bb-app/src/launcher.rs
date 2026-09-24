//! Spawning game clients and observing their lifetime.

use std::fs::OpenOptions;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use bb_core::{Account, LaunchOptions, Provider, game_args, steam};

use crate::profile_link::{self, ProfileLinkError};

/// How often to check whether a starting client has taken its `Local.dat`.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long `Local.dat` must stay locked before the client is considered to own it for good,
/// guarding against the client briefly opening and re-opening it during startup.
const LOCK_STABLE_FOR: Duration = Duration::from_secs(3);
/// Upper bound for startup; generous because a client may check for updates first.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
/// `ERROR_SHARING_VIOLATION`.
const ERROR_SHARING_VIOLATION: i32 = 32;

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("Guild Wars 2 is not configured yet.")]
    NoGamePath,
    #[error(
        "{0} is a Steam account: start Steam and sign in with the Steam user it is linked to first."
    )]
    SteamNotRunning(String),
    #[error(
        "{0} is a Steam account, but no Guild Wars 2 installation with Steam support was found. \
         Install Guild Wars 2 through Steam; that installation can be used for all accounts."
    )]
    SteamInstallMissing(String),
    #[error("Setting up the login of {0} needs all other Guild Wars 2 clients to be closed first.")]
    SetupNeedsExclusive(String),
    #[error("An account is currently being set up. Close that client before starting another one.")]
    SetupClientRunning,
    #[error("could not prepare the account's profile folder: {0}")]
    Profile(#[from] bb_store::StoreError),
    #[error("could not switch Guild Wars 2 to this account's profile: {0}")]
    ProfileLink(#[from] ProfileLinkError),
    #[error("failed to start Guild Wars 2: {0}")]
    Spawn(#[source] io::Error),
    #[error("Guild Wars 2 closed during startup ({0}).")]
    ExitedDuringStartup(ExitStatus),
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
    /// from) is dropped. Used to stop the process without the PID-reuse race of re-opening it.
    pub fn raw_handle(&self) -> isize {
        self.child.as_raw_handle() as isize
    }

    /// Blocks the calling thread until the client process exits.
    pub fn wait_for_exit(mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// How to start a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Normal multi-launch start with `-shareArchive` (falls back to [`LaunchMode::SetUpLogin`]
    /// if the account has no `Local.dat` yet).
    Play,
    /// Start without `-shareArchive` so the client can write its `Local.dat` — the only way to
    /// save a remembered login. Requires that no other client runs.
    SetUpLogin,
}

/// Result of a successful [`launch`].
#[derive(Debug)]
pub struct Launched {
    pub client: RunningClient,
    /// Started without `-shareArchive` to set up / save the account's login.
    pub setup: bool,
    /// Something the user should know even though the client is running.
    pub warning: Option<String>,
}

/// Starts `account`'s client and waits until it has taken its own `Local.dat`.
///
/// **Blocks for several seconds** — call it off the UI thread, and never run two launches at
/// once: `%APPDATA%\Guild Wars 2` points at the launching account for the whole call (see
/// [`crate::profile_link`]) and is pointed back at the shared profile before returning, on every
/// path.
///
/// A client started with `-shareArchive` opens `Local.dat` read-only: it can't create a missing
/// one ("data archive cannot be opened") and never writes a remembered login back. Setting up or
/// renewing a login therefore needs a launch without it ([`LaunchMode::SetUpLogin`]), which is
/// only possible while no other client runs, because such a client locks `Gw2.dat` exclusively.
pub fn launch(
    gw2_path: &Path,
    account: &Account,
    mode: LaunchMode,
) -> Result<Launched, LaunchError> {
    if !gw2_path.is_file() {
        return Err(LaunchError::NoGamePath);
    }
    let client_exe = match account.provider {
        Provider::ArenaNet => gw2_path.to_owned(),
        Provider::Steam => crate::game::steam_client(gw2_path)
            .ok_or_else(|| LaunchError::SteamInstallMissing(account.name.clone()))?,
    };
    let gw2_path = client_exe.as_path();
    if account.provider == Provider::Steam && !steam_running() {
        return Err(LaunchError::SteamNotRunning(account.name.clone()));
    }

    let setup = mode == LaunchMode::SetUpLogin || !bb_store::is_set_up(account.id);
    let others_running = !running_clients(gw2_path).is_empty();
    if setup && others_running {
        return Err(LaunchError::SetupNeedsExclusive(account.name.clone()));
    }
    if !setup && archive_held_exclusively(gw2_path) {
        return Err(LaunchError::SetupClientRunning);
    }

    let options = LaunchOptions {
        share_archive: !setup,
        autologin: !setup,
    };
    let local_dat = bb_store::local_dat_path(account.id)?;
    let temp_dir = bb_store::ensure_profile_dir(account.id)?.join("Temp");
    std::fs::create_dir_all(&temp_dir).map_err(LaunchError::Spawn)?;

    ensure_mutex_clear(gw2_path);
    profile_link::point_to_account(account.id)?;

    let started = start_and_wait(gw2_path, account, options, &temp_dir, &local_dat);

    let restored = profile_link::point_to_shared();
    let (client, mut warning) = started?;
    if let Err(error) = restored {
        warning = Some(format!(
            "Guild Wars 2's data folder could not be switched back to the shared profile: {error}"
        ));
    }

    // Release this client's single-instance mutex right away, so the next launch doesn't have
    // to search for it (best-effort; the next launch checks again anyway).
    let _ = bb_win::mutex::kill_gw2_mutex(client.pid());

    Ok(Launched {
        client,
        setup,
        warning,
    })
}

/// Spawns the client and waits until it owns `local_dat`. On a timeout the client is kept (it may
/// just be updating), with a warning instead of an error.
fn start_and_wait(
    gw2_path: &Path,
    account: &Account,
    options: LaunchOptions,
    temp_dir: &Path,
    local_dat: &Path,
) -> Result<(RunningClient, Option<String>), LaunchError> {
    let working_dir = gw2_path.parent().unwrap_or(gw2_path);
    let mut command = Command::new(gw2_path);
    command
        .args(game_args(account, options))
        .current_dir(working_dir)
        .env("TMP", temp_dir)
        .env("TEMP", temp_dir);
    for (key, value) in provider_env(account) {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(LaunchError::Spawn)?;

    let warning = match wait_until_locked(&mut child, local_dat)? {
        true => None,
        false => Some(
            "Guild Wars 2 took unusually long to start. If it logs in with the wrong account, \
             close it and start it again."
                .to_owned(),
        ),
    };

    Ok((
        RunningClient {
            pid: child.id(),
            child,
        },
        warning,
    ))
}

/// Waits until `path` has been locked for [`LOCK_STABLE_FOR`] without interruption. Returns
/// `Ok(false)` on timeout and an error if the client exits first.
fn wait_until_locked(child: &mut Child, path: &Path) -> Result<bool, LaunchError> {
    let started = Instant::now();
    let mut locked_since: Option<Instant> = None;
    loop {
        if let Some(status) = child.try_wait().map_err(LaunchError::Spawn)? {
            return Err(LaunchError::ExitedDuringStartup(status));
        }
        if is_locked(path) {
            let since = *locked_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= LOCK_STABLE_FOR {
                return Ok(true);
            }
        } else {
            locked_since = None;
        }
        if started.elapsed() >= STARTUP_TIMEOUT {
            return Ok(false);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Whether another process holds `path` open without allowing others to read it.
fn is_locked(path: &Path) -> bool {
    match OpenOptions::new().read(true).share_mode(0).open(path) {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// Whether a client started without `-shareArchive` (a setup launch) holds `Gw2.dat`, which
/// would make every other client fail to open it.
fn archive_held_exclusively(gw2_path: &Path) -> bool {
    const FILE_SHARE_READ: u32 = 1;
    let archive = gw2_path.with_file_name("Gw2.dat");
    match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(archive)
    {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// Extra environment for the client, depending on how the account authenticates.
///
/// A Steam account is started directly (not through `steam.exe -applaunch`, which asks for
/// confirmation of custom arguments and only allows one instance). `SteamAppId` tells the
/// client's Steam API which app it is, so it attaches to the running Steam client, which then
/// signs the game in with its currently signed-in Steam user — the same approach gw2launcher uses.
fn provider_env(account: &Account) -> Vec<(&'static str, String)> {
    match account.provider {
        Provider::ArenaNet => Vec::new(),
        Provider::Steam => vec![("SteamAppId", steam::GW2_APP_ID.to_string())],
    }
}

/// Whether the Steam client is running. If the process list can't be read, launching proceeds
/// and GW2 reports the problem itself.
fn steam_running() -> bool {
    bb_win::process::find_processes_by_name("steam.exe").map_or(true, |pids| !pids.is_empty())
}

fn running_clients(gw2_path: &Path) -> Vec<u32> {
    let exe_name = gw2_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(crate::game::GW2_EXE);
    bb_win::process::find_processes_by_name(exe_name).unwrap_or_else(|error| {
        eprintln!("could not list running Guild Wars 2 clients: {error}");
        Vec::new()
    })
}

/// If a GW2 client currently holds the single-instance mutex, closes that handle so a new
/// client can create its own instead of being refused.
///
/// Looks at every running client, not just ones Breakbar started, so a client left over from a
/// previous session doesn't block a new launch either. Best-effort: if this fails, launching
/// proceeds anyway — worst case GW2 itself refuses to start.
fn ensure_mutex_clear(gw2_path: &Path) {
    match bb_win::mutex::gw2_mutex_exists() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("could not check the Guild Wars 2 mutex, launching anyway: {error}");
            return;
        }
    }
    for pid in running_clients(gw2_path) {
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
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn missing_game_path_is_rejected() {
        // No isolation needed: rejected before anything touches a profile.
        let account = Account::new(AccountId(1), "Main");
        let result = launch(
            &PathBuf::from(r"C:\does\not\exist\Gw2-64.exe"),
            &account,
            LaunchMode::Play,
        );
        assert!(matches!(result, Err(LaunchError::NoGamePath)));
    }

    #[test]
    fn exclusive_open_counts_as_locked() {
        let path = std::env::temp_dir().join(format!("breakbar-lock-{}", std::process::id()));
        fs::write(&path, b"x").unwrap();
        assert!(!is_locked(&path));

        let holder = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(is_locked(&path));

        drop(holder);
        assert!(!is_locked(&path));
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn only_steam_accounts_get_the_steam_app_id() {
        let arenanet = Account::new(AccountId(1), "Main");
        assert!(provider_env(&arenanet).is_empty());

        let mut steam_account = Account::new(AccountId(2), "Steam");
        steam_account.provider = Provider::Steam;
        assert_eq!(
            provider_env(&steam_account),
            [("SteamAppId", "1284210".to_owned())]
        );
    }

    #[test]
    fn missing_file_is_not_locked() {
        assert!(!is_locked(Path::new(r"C:\does\not\exist\Local.dat")));
    }

    /// Stands in for the game client with `ping.exe`, which rejects the GW2 switches and exits
    /// at once: startup must report that instead of waiting, and the real GW2 folder must end up
    /// pointing at the shared profile again.
    #[test]
    fn client_exiting_during_startup_restores_the_shared_profile() {
        with_isolated_appdata("launch-exit", |root| {
            let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
            let stand_in = PathBuf::from(&windir).join("System32").join("ping.exe");
            let account = Account::new(AccountId(1), "Main");

            let result = launch(&stand_in, &account, LaunchMode::Play);

            assert!(matches!(result, Err(LaunchError::ExitedDuringStartup(_))));
            let real = root.join("Roaming").join("Guild Wars 2");
            let shared = bb_store::shared_profile_dir().unwrap().join("Guild Wars 2");
            assert_eq!(
                fs::canonicalize(&real).unwrap(),
                fs::canonicalize(&shared).unwrap()
            );
        });
    }
}
