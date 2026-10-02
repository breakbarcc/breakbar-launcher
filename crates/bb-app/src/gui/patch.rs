//! The "Start now" button on the patch-available banner: starts Guild Wars 2 directly so it can
//! download a pending update, then watches for it to close and reports what happened.

use std::time::Instant;

use super::{
    App, ComponentHandle, Duration, MainWindow, Messages, Path, PathBuf, Rc, RefCell, ToastKind,
    check_for_patch, launch_failure, push_toast, thread,
};
use crate::launcher;

/// How often the background monitor checks whether the patch client is still running or whether
/// `Gw2.dat` has changed.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// The client can restart itself partway through applying a patch (briefly exiting before a new
/// process appears — the same behavior `launcher::RunningClient` handles for account launches).
/// When no client is found running, this much time is given for a successor to show up before
/// the patch is considered finished, so that gap doesn't get reported as an early close. A
/// self-restart happens near-instantly when it happens at all, so this only needs to be short.
const RESTART_GRACE: Duration = Duration::from_secs(2);
/// How long `Gw2.dat` must stay unchanged, while a client is still running, before the patch
/// counts as finished. There is no window-based "done" signal to wait for here the way
/// `RunningClient::wait_for_game_window` has for account launches: this launch has no account and
/// never logs in, so the client never reaches its own game window (`GAME_WINDOW_CLASSES`) — it
/// just sits at the login screen, in the very same `ArenaNet`-class window it patched in, once
/// it's done. Watching `Gw2.dat` stop growing is the only signal available instead. Only applies
/// once real activity has actually been seen — see `NO_ACTIVITY_GRACE` for before that.
const PATCH_IDLE_GRACE: Duration = Duration::from_secs(4);
/// How long to wait for `Gw2.dat` to start changing at all before giving up on it ever doing so.
/// Confirmed live: the client needs a few seconds to connect and negotiate with `ArenaNet`'s
/// servers before it writes anything, and "hasn't started yet" looks identical to "finished and
/// stopped" from the file alone — there being nothing to tell them apart is exactly why this
/// needs to be far more generous than `PATCH_IDLE_GRACE`, which only applies once real activity
/// (an actual change) has been seen at least once.
const NO_ACTIVITY_GRACE: Duration = Duration::from_secs(20);
/// The grace period between asking the client to close (`WM_CLOSE`) and giving up on it and
/// terminating it instead. `companions::close_all` gives a companion app 15s here, but a real
/// close (verified live, see `breakbar.log`) reacted in under a second, so this only needs to
/// cover a genuinely unresponsive client, not the normal case.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// Starts the game directly (see `launcher::launch_to_patch`) and, once it's running, hands off
/// to `watch` to notice when it closes again.
fn start_now(window: &MainWindow, app: &Rc<RefCell<App>>) {
    let Some(gw2_path) = app.borrow().config.gw2_path.clone() else {
        return;
    };

    match launcher::launch_to_patch(&gw2_path) {
        Ok(()) => {
            window.set_patching(true);
            watch(window, gw2_path);
        }
        Err(error) => {
            let messages = window.global::<Messages>();
            let (kind, detail) = launch_failure(&error);
            push_toast(
                window,
                ToastKind::Error,
                messages.invoke_start_failed(),
                messages.invoke_launch_failure(kind, "Guild Wars 2".into(), detail),
            );
        }
    }
}

/// Waits for the patch client to finish and close (see `wait_for_patch_to_finish`), then reports
/// the result: if `Gw2.dat` actually changed, re-checks the normal way (`check_for_patch`); if it
/// never did, the client itself just confirmed it has nothing to download, which settles it more
/// reliably than comparing build ids (see `confirm_up_to_date`).
fn watch(window: &MainWindow, gw2_path: PathBuf) {
    let weak = window.as_weak();
    let _ = thread::Builder::new()
        .name("patch-watch".to_owned())
        .spawn(move || {
            let patched = wait_for_patch_to_finish(&gw2_path);
            let available = if patched {
                check_for_patch(&gw2_path)
            } else {
                confirm_up_to_date(&gw2_path)
            };
            let _ = weak.upgrade_in_event_loop(move |window| {
                window.set_patching(false);
                if let Some(available) = available {
                    window.set_patch_available(available);
                    report(&window, available);
                }
            });
        });
}

/// Accepts the freshly fetched server build as current without comparing it against the stored
/// baseline: called after a direct launch confirmed firsthand that `Gw2.dat` never changed, i.e.
/// the client itself found nothing to download. That is a more reliable sign of being up to date
/// than `ArenaNet`'s build id, which also advances for server-side-only changes that need no
/// client download at all — comparing against it forever would otherwise leave the "update
/// available" banner stuck even though starting the game again would keep confirming the same
/// thing. `None` if the local build or the server build can't be read right now.
fn confirm_up_to_date(gw2_path: &Path) -> Option<bool> {
    let local_mtime = crate::game::game_build(gw2_path)?;
    let server_build = crate::gw2_build::fetch_current_build()?;
    let _ = bb_store::set_known_server_build(server_build, local_mtime);
    Some(false)
}

/// Waits for the patch client to finish and go away, and returns whether `Gw2.dat` was ever seen
/// to actually change (i.e. whether anything was downloaded at all). Once `Gw2.dat` has stopped
/// changing (for `PATCH_IDLE_GRACE` if it had actually been changing, or `NO_ACTIVITY_GRACE` if it
/// never started) while a client is still running, the download is done and closing it is
/// attempted (`close_patch_client`) — but this loop is what actually decides the client is gone, by
/// seeing `running_clients` become empty, exactly like a manual close. A close is only ever
/// attempted once per distinct set of running client PIDs, so a client that doesn't react to it
/// isn't hammered with repeated attempts every tick — but a *new* process (the client can restart
/// itself, including right after being closed, not just partway through) still gets its own
/// attempt, since it's a different PID set.
fn wait_for_patch_to_finish(gw2_path: &Path) -> bool {
    let mut last_stamp = archive_stamp(gw2_path);
    let mut stable_since = Instant::now();
    let mut ever_changed = false;
    let mut close_attempted_for = Vec::new();
    loop {
        let running = launcher::running_clients(gw2_path);
        if running.is_empty() {
            thread::sleep(RESTART_GRACE);
            if launcher::running_clients(gw2_path).is_empty() {
                return ever_changed;
            }
            continue;
        }

        let stamp = archive_stamp(gw2_path);
        if stamp == last_stamp {
            let grace = if ever_changed {
                PATCH_IDLE_GRACE
            } else {
                NO_ACTIVITY_GRACE
            };
            if stable_since.elapsed() >= grace && running != close_attempted_for {
                close_patch_client(gw2_path);
                close_attempted_for = running;
            }
        } else {
            last_stamp = stamp;
            stable_since = Instant::now();
            ever_changed = true;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Size and modification time of `Gw2.dat`, `None` if it can't be read right now (e.g. briefly
/// while the client replaces it).
fn archive_stamp(gw2_path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(gw2_path.with_file_name("Gw2.dat")).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Attempts to close every remaining client of `gw2_path` once: `WM_CLOSE` first, `CLOSE_GRACE` to
/// react, then terminated if still running — `companions::close_all`'s escalation, minus its first
/// "let it exit on its own" step, since Breakbar itself is the one asking here. Every step is
/// logged (not just the outcome) so a client that doesn't actually close can be diagnosed from
/// `breakbar.log` — whether it ignored `WM_CLOSE`, or `terminate` itself failed, is otherwise
/// invisible from outside.
fn close_patch_client(gw2_path: &Path) {
    let pids = launcher::running_clients(gw2_path);
    crate::log::write(format!("update finished, asking {pids:?} to close"));
    for pid in &pids {
        match bb_win::window::request_close(*pid) {
            Ok(count) => crate::log::write(format!("asked {count} window(s) of {pid} to close")),
            Err(error) => crate::log::write(format!("could not ask {pid} to close: {error}")),
        }
    }

    let deadline = Instant::now() + CLOSE_GRACE;
    while Instant::now() < deadline && !launcher::running_clients(gw2_path).is_empty() {
        thread::sleep(POLL_INTERVAL);
    }

    let remaining = launcher::running_clients(gw2_path);
    if remaining.is_empty() {
        crate::log::write("update client closed on its own".to_owned());
        return;
    }
    crate::log::write(format!(
        "{remaining:?} still running after {CLOSE_GRACE:?}, terminating"
    ));
    for pid in remaining {
        match bb_win::process::Process::open(pid) {
            Ok(process) => match bb_win::process::terminate(process.raw_handle()) {
                Ok(()) => crate::log::write(format!("terminated {pid}")),
                Err(error) => crate::log::write(format!("could not terminate {pid}: {error}")),
            },
            Err(error) => {
                crate::log::write(format!("could not open {pid} to terminate it: {error}"));
            }
        }
    }
}

/// Tells the user whether the update landed: the client only ever patches on a normal start, so
/// a still-available update most likely means it was closed before finishing, or had nothing to
/// download over (e.g. no connection).
fn report(window: &MainWindow, available: bool) {
    let messages = window.global::<Messages>();
    if available {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_patch_unchanged_title(),
            messages.invoke_patch_unchanged(),
        );
    } else {
        push_toast(
            window,
            ToastKind::Success,
            messages.invoke_patch_done_title(),
            messages.invoke_patch_done(),
        );
    }
}

/// Connects the window's callback for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_start_patch({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                start_now(&window, &app);
            }
        }
    });
}
