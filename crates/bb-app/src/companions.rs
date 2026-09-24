//! Starting and stopping companion apps (such as Blish HUD) alongside game clients.
//!
//! Every client gets a [`Session`] on its monitor thread: it starts the account's companions
//! once their trigger is reached and closes them again after the client exits. Apps with
//! [`Scope::Shared`] run once for all clients and are reference-counted in [`SharedInstances`].

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bb_core::{Account, ArgContext, CompanionApp, CompanionId, Scope, Trigger};

/// How long a companion may take to exit after being asked to close before it is terminated.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Companion apps with [`Scope::Shared`] that are currently running, across all clients.
#[derive(Debug, Default)]
pub struct SharedInstances {
    running: Mutex<HashMap<CompanionId, SharedInstance>>,
}

#[derive(Debug)]
struct SharedInstance {
    child: Child,
    /// Number of running clients that use this instance.
    users: usize,
}

/// The companion apps of one running client.
#[derive(Debug)]
pub struct Session {
    shared: Arc<SharedInstances>,
    apps: Vec<CompanionApp>,
    pid: u32,
    mumble: String,
    account: String,
    /// Per-client instances started by this session, with whether to close them with the game.
    owned: Vec<(Child, bool)>,
    /// Shared instances this session counts as a user of.
    joined: Vec<(CompanionId, bool)>,
}

impl Session {
    /// Prepares the companions `apps` for the client of `account` running as `pid`.
    pub fn new(
        shared: Arc<SharedInstances>,
        apps: Vec<CompanionApp>,
        account: &Account,
        pid: u32,
    ) -> Self {
        Self {
            shared,
            apps,
            pid,
            mumble: account.mumble_link_name(),
            account: account.name.clone(),
            owned: Vec::new(),
            joined: Vec::new(),
        }
    }

    /// Whether any companion waits for `trigger`.
    pub fn waits_for(&self, trigger: Trigger) -> bool {
        self.apps.iter().any(|app| app.start_when == trigger)
    }

    /// Starts every companion whose trigger is `trigger`. Returns one message per app that could
    /// not be started; the others are started regardless.
    pub fn start(&mut self, trigger: Trigger) -> Vec<String> {
        let apps: Vec<CompanionApp> = self
            .apps
            .iter()
            .filter(|app| app.start_when == trigger)
            .cloned()
            .collect();
        apps.iter()
            .filter_map(|app| {
                self.start_one(app)
                    .err()
                    .map(|error| format!("{} could not be started: {error}", app.name))
            })
            .collect()
    }

    fn start_one(&mut self, app: &CompanionApp) -> io::Result<()> {
        match app.scope {
            Scope::PerClient => {
                let child = self.spawn(app)?;
                self.owned.push((child, app.close_with_game));
            }
            Scope::Shared => {
                let mut running = lock(&self.shared.running);
                // An instance the user closed meanwhile is replaced by a new one.
                if let Some(instance) = running.get_mut(&app.id)
                    && is_running(&mut instance.child)
                {
                    instance.users += 1;
                } else {
                    let child = self.spawn(app)?;
                    // Clients still counted on a closed instance now use the new one.
                    let users = running.get(&app.id).map_or(0, |instance| instance.users) + 1;
                    running.insert(app.id, SharedInstance { child, users });
                }
                self.joined.push((app.id, app.close_with_game));
            }
        }
        Ok(())
    }

    fn spawn(&self, app: &CompanionApp) -> io::Result<Child> {
        if !app.exe.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} does not exist", app.exe.display()),
            ));
        }
        let args = app.expand_args(&ArgContext {
            pid: self.pid,
            mumble: &self.mumble,
            account: &self.account,
        });
        // Many tools (Blish HUD among them) expect to run from their own folder.
        let working_dir = app.exe.parent().unwrap_or(Path::new("."));
        Command::new(&app.exe)
            .args(args)
            .current_dir(working_dir)
            .spawn()
    }

    /// Closes this client's companions after it has exited: its own instances, and shared ones
    /// no other client uses anymore. Blocks for up to [`CLOSE_GRACE`].
    pub fn stop(self) {
        let mut to_close: Vec<Child> = self
            .owned
            .into_iter()
            .filter_map(|(child, close)| close.then_some(child))
            .collect();

        {
            let mut running = lock(&self.shared.running);
            for (id, close) in self.joined {
                let Some(instance) = running.get_mut(&id) else {
                    continue;
                };
                instance.users = instance.users.saturating_sub(1);
                if instance.users == 0
                    && close
                    && let Some(instance) = running.remove(&id)
                {
                    to_close.push(instance.child);
                }
            }
        }

        close_all(to_close);
    }
}

/// Asks every process in `children` to close, then terminates the ones still running after
/// [`CLOSE_GRACE`]. A program closed through its window can save its settings first, which
/// terminating it outright would prevent.
pub fn close_all(mut children: Vec<Child>) {
    children.retain_mut(is_running);
    for child in &children {
        let _ = bb_win::window::request_close(child.id());
    }

    let deadline = Instant::now() + CLOSE_GRACE;
    while !children.is_empty() && Instant::now() < deadline {
        thread::sleep(EXIT_POLL_INTERVAL);
        children.retain_mut(is_running);
    }

    for mut child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn is_running(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// A panic while holding the lock leaves the map consistent (every update is a single step),
/// so a poisoned lock is simply taken over.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_core::AccountId;
    use std::path::PathBuf;

    /// Process name of the companion stand-in.
    const STAND_IN: &str = "breakbar-companion-stand-in.exe";

    /// A copy of `ping.exe` stands in for a companion: it runs for a while and has no window, so
    /// closing it has to fall back to terminating it. It is renamed so the launcher tests, which
    /// use `ping.exe` as a stand-in *game client*, don't mistake it for a running client.
    fn stand_in_exe() -> &'static Path {
        static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        EXE.get_or_init(|| {
            let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
            let dir =
                std::env::temp_dir().join(format!("breakbar-companions-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let exe = dir.join(STAND_IN);
            std::fs::copy(
                PathBuf::from(windir).join("System32").join("ping.exe"),
                &exe,
            )
            .unwrap();
            exe
        })
    }

    fn running_stand_ins() -> Vec<u32> {
        bb_win::process::find_processes_by_name(STAND_IN).unwrap()
    }

    fn ping(id: u32, scope: Scope) -> CompanionApp {
        CompanionApp {
            id: CompanionId(id),
            name: format!("ping {id}"),
            exe: stand_in_exe().to_owned(),
            args: "-n 60 127.0.0.1".to_owned(),
            scope,
            start_when: Trigger::ProcessStarted,
            close_with_game: true,
        }
    }

    fn session(shared: &Arc<SharedInstances>, apps: Vec<CompanionApp>, id: u32) -> Session {
        let account = Account::new(AccountId(id), format!("Account {id}"));
        Session::new(Arc::clone(shared), apps, &account, 1000 + id)
    }

    #[test]
    fn per_client_companions_are_closed_with_their_client() {
        let shared = Arc::new(SharedInstances::default());
        let mut session = session(&shared, vec![ping(1, Scope::PerClient)], 1);

        assert!(session.start(Trigger::WindowShown).is_empty());
        assert!(session.owned.is_empty(), "waits for its own trigger");
        assert!(session.start(Trigger::ProcessStarted).is_empty());
        let pid = session.owned[0].0.id();
        assert!(running_stand_ins().contains(&pid));

        session.stop();
        assert!(!running_stand_ins().contains(&pid));
    }

    #[test]
    fn shared_companion_runs_until_its_last_client_exits() {
        let shared = Arc::new(SharedInstances::default());
        let mut first = session(&shared, vec![ping(2, Scope::Shared)], 1);
        let mut second = session(&shared, vec![ping(2, Scope::Shared)], 2);

        assert!(first.start(Trigger::ProcessStarted).is_empty());
        assert!(second.start(Trigger::ProcessStarted).is_empty());
        let pid = {
            let running = lock(&shared.running);
            assert_eq!(running.len(), 1, "one instance for both clients");
            assert_eq!(running[&CompanionId(2)].users, 2);
            running[&CompanionId(2)].child.id()
        };

        first.stop();
        assert_eq!(lock(&shared.running)[&CompanionId(2)].users, 1);
        assert!(running_stand_ins().contains(&pid));

        second.stop();
        assert!(lock(&shared.running).is_empty());
    }

    #[test]
    fn missing_companion_is_reported_and_others_still_start() {
        let shared = Arc::new(SharedInstances::default());
        let mut missing = ping(3, Scope::PerClient);
        missing.exe = PathBuf::from(r"C:\does\not\exist\Blish HUD.exe");
        let mut session = session(&shared, vec![missing, ping(4, Scope::PerClient)], 1);

        let errors = session.start(Trigger::ProcessStarted);

        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("ping 3 could not be started"));
        assert_eq!(session.owned.len(), 1);
        session.stop();
    }

    #[test]
    fn companions_kept_after_the_game_are_left_running() {
        let shared = Arc::new(SharedInstances::default());
        let mut app = ping(5, Scope::PerClient);
        app.close_with_game = false;
        // Short-lived, as nothing will end it.
        app.args = "-n 5 127.0.0.1".to_owned();
        let mut session = session(&shared, vec![app], 1);
        assert!(session.start(Trigger::ProcessStarted).is_empty());
        let pid = session.owned[0].0.id();

        session.stop();

        assert!(running_stand_ins().contains(&pid));
    }
}
