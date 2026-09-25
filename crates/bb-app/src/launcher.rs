//! Spawning game clients and observing their lifetime.

use std::fs::OpenOptions;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::OwnedHandle;
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use bb_core::{Account, LaunchOptions, Provider, game_args, steam};
use bb_win::process::Process;

use crate::profile_link::{self, ProfileLinkError};

/// How often to check whether a starting client has taken its `Local.dat`.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long `Local.dat` must stay locked before the client is considered to own it for good,
/// guarding against the client briefly opening and re-opening it during startup.
const LOCK_STABLE_FOR: Duration = Duration::from_secs(3);
/// Upper bound for startup; generous because a client may check for updates first.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a client that exited successfully during startup gets to show up again as a new
/// process (see [`restarted_client`]).
const RESTART_GRACE: Duration = Duration::from_secs(15);
/// Named mutex serializing launches across Breakbar processes (the window and shortcut starts),
/// because each launch points `%APPDATA%\Guild Wars 2` at its account for a few seconds.
const LAUNCH_LOCK: &str = "Breakbar-Launch";
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
    #[error("{0} is already running.")]
    AlreadyRunning(String),
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
    process: Process,
}

impl RunningClient {
    /// The client's process id. Not necessarily the one of the process Breakbar spawned: a client
    /// that updates itself starts over as a new process (see [`restarted_client`]).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Raw OS handle of the client process, valid until this value is dropped. Used to stop the
    /// process without the PID-reuse race of re-opening it.
    pub fn raw_handle(&self) -> isize {
        self.process.raw_handle()
    }

    /// Blocks until the client shows its game window (not the launcher/patcher window, which has
    /// a different class). Returns `false` if the client exits first.
    ///
    /// Polls a few times per second: the enumeration costs microseconds, and this only runs
    /// until the game window appears, and only for clients with companions waiting for it.
    pub fn wait_for_game_window(&mut self) -> bool {
        loop {
            if !matches!(self.process.try_wait(), Ok(None)) {
                return false;
            }
            if bb_win::window::has_visible_window(self.pid, GAME_WINDOW_CLASSES).unwrap_or(false) {
                return true;
            }
            thread::sleep(WINDOW_POLL_INTERVAL);
        }
    }

    /// Blocks the calling thread until the client process exits.
    pub fn wait_for_exit(self) -> io::Result<ExitStatus> {
        let code = self.process.wait()?;
        Ok(ExitStatus::from_raw(code))
    }
}

/// Window classes of the game window (DirectX 11 and DirectX 9 renderer). The launcher and
/// patcher window uses the class `ArenaNet` instead — the same distinction Blish HUD makes.
pub(crate) const GAME_WINDOW_CLASSES: &[&str] =
    &["ArenaNet_Gr_Window_Class", "ArenaNet_Dx_Window_Class"];
const WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
    pub warning: Option<LaunchWarning>,
}

/// A problem that doesn't stop the client from running.
#[derive(Debug)]
pub enum LaunchWarning {
    /// The client took unusually long to take its `Local.dat`; it may have picked up another
    /// account's login if the profile was switched meanwhile.
    SlowStart,
    /// `%APPDATA%\Guild Wars 2` could not be pointed back at the shared profile.
    ProfileNotRestored(ProfileLinkError),
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

    // Waits while another Breakbar process launches; released when this launch returns.
    let _lock = bb_win::mutex::OwnedMutex::acquire(LAUNCH_LOCK).inspect_err(|error| {
        eprintln!("could not take the launch lock, launching anyway: {error}")
    });

    // A client of this account already runs (e.g. started from a shortcut): it holds the
    // account's `Local.dat`, and a second one would log in with the same account.
    if bb_store::local_dat_path(account.id).is_ok_and(|path| is_locked(&path)) {
        return Err(LaunchError::AlreadyRunning(account.name.clone()));
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
        warning = Some(LaunchWarning::ProfileNotRestored(error));
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
) -> Result<(RunningClient, Option<LaunchWarning>), LaunchError> {
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
    let exe_name = gw2_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(crate::game::GW2_EXE);
    let already_running = running_clients(gw2_path);
    let child = command.spawn().map_err(LaunchError::Spawn)?;
    let process = Process::from(OwnedHandle::from(child));

    let (process, locked) = wait_until_locked(process, exe_name, &already_running, local_dat)?;
    let warning = (!locked).then_some(LaunchWarning::SlowStart);

    Ok((
        RunningClient {
            pid: process.pid(),
            process,
        },
        warning,
    ))
}

/// Waits until `path` has been locked for [`LOCK_STABLE_FOR`] without interruption. Returns the
/// process that holds it (which is not `process` if that one restarted itself, see
/// [`restarted_client`]) and whether it did, `false` on a timeout. An error if the client exits
/// first.
fn wait_until_locked(
    mut process: Process,
    exe_name: &str,
    already_running: &[u32],
    path: &Path,
) -> Result<(Process, bool), LaunchError> {
    let started = Instant::now();
    let mut locked_since: Option<Instant> = None;
    loop {
        if let Some(code) = process
            .try_wait()
            .map_err(io::Error::from)
            .map_err(LaunchError::Spawn)?
        {
            process = restarted_client(process.pid(), code, exe_name, already_running)?;
            locked_since = None;
            continue;
        }
        if is_locked(path) {
            let since = *locked_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= LOCK_STABLE_FOR {
                return Ok((process, true));
            }
        } else {
            locked_since = None;
        }
        if started.elapsed() >= STARTUP_TIMEOUT {
            return Ok((process, false));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// The client that took over from `pid`, which exited with `code` during startup.
///
/// A client whose executable is out of date replaces it with the current one and starts it (with
/// the same arguments), then exits successfully itself. That happens when Steam has just
/// (re)installed its own older `Gw2-64.exe`, and can happen with any game patch. The new process is
/// the client; the old one's exit is not a failed start.
///
/// The new process is looked for by its parent id (Windows keeps it after the parent has exited),
/// and failing that as any still running process of the same executable that was not running
/// before this launch (`already_running`): the client may start it through a helper process.
///
/// Any other exit during startup is an error, and so is a successful exit that no new client
/// follows within [`RESTART_GRACE`] (the user closed the launcher window, say).
fn restarted_client(
    pid: u32,
    code: u32,
    exe_name: &str,
    already_running: &[u32],
) -> Result<Process, LaunchError> {
    let exited = || LaunchError::ExitedDuringStartup(ExitStatus::from_raw(code));
    if code != 0 {
        return Err(exited());
    }
    let started = Instant::now();
    loop {
        let mut candidates =
            bb_win::process::find_child_processes(pid, exe_name).unwrap_or_default();
        candidates.extend(
            bb_win::process::find_processes_by_name(exe_name)
                .unwrap_or_default()
                .into_iter()
                .filter(|other| *other != pid && !already_running.contains(other)),
        );
        let successor = candidates
            .into_iter()
            .filter_map(|candidate| Process::open(candidate).ok())
            .find(|candidate| matches!(candidate.try_wait(), Ok(None)));
        if let Some(successor) = successor {
            return Ok(successor);
        }
        if started.elapsed() >= RESTART_GRACE {
            return Err(exited());
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

    /// A client that exits successfully and was followed by a new process of the same executable
    /// (started by it) hands over to that process; a failing exit never does. The test process
    /// plays the exited client, a renamed copy of `ping.exe` its successor. The copy has its own
    /// name so the other tests, which use `ping.exe` as a stand-in client, do not see it running.
    #[test]
    fn a_restarted_client_is_adopted() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let name = format!("breakbar-restart-{}.exe", std::process::id());
        let exe = std::env::temp_dir().join(&name);
        fs::copy(
            PathBuf::from(windir).join("System32").join("ping.exe"),
            &exe,
        )
        .unwrap();
        let mut successor = Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let adopted = restarted_client(std::process::id(), 0, &name, &[]).unwrap();
        let failed = restarted_client(std::process::id(), 1, &name, &[]);

        let pid = adopted.pid();
        bb_win::process::terminate(adopted.raw_handle()).unwrap();
        let _ = successor.wait();
        let _ = fs::remove_file(&exe);
        assert_eq!(pid, successor.id());
        assert!(matches!(failed, Err(LaunchError::ExitedDuringStartup(_))));
    }

    /// The successor is also found when it was not started by the exited client itself (a helper
    /// process in between), as any new process of the executable.
    #[test]
    fn a_restarted_client_is_found_without_its_parent() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let name = format!("breakbar-helper-{}.exe", std::process::id());
        let exe = std::env::temp_dir().join(&name);
        fs::copy(
            PathBuf::from(windir).join("System32").join("ping.exe"),
            &exe,
        )
        .unwrap();
        let mut successor = Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let unrelated_parent = u32::MAX - 1;
        let adopted = restarted_client(unrelated_parent, 0, &name, &[]).unwrap();

        let pid = adopted.pid();
        bb_win::process::terminate(adopted.raw_handle()).unwrap();
        let _ = successor.wait();
        let _ = fs::remove_file(&exe);
        assert_eq!(pid, successor.id());
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
