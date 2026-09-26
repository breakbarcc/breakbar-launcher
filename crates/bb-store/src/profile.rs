//! Per-account profile locations.
//!
//! Every account gets its own folder under `%LOCALAPPDATA%\Breakbar\profiles\<id>\`, holding its
//! `Local.dat`, its temp folder and the record of the game build it was set up for. The game only
//! ever reads `%APPDATA%\Guild Wars 2`, so that real folder is a junction which points at an
//! account's profile only while that account's client starts (see `profile_link` in the app).
//! The `shared` profile is what it points at the rest of the time.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use bb_core::AccountId;

use crate::StoreError;

/// The account's isolated profile folder. Does not create it; see [`ensure_profile_dir`].
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set.
pub fn profile_dir(account_id: AccountId) -> Result<PathBuf, StoreError> {
    Ok(profiles_root()?.join(account_id.0.to_string()))
}

/// The shared default profile: what `%APPDATA%\Guild Wars 2` points at whenever no launch is in
/// progress, so clients started outside Breakbar and settings written by running clients (such
/// as graphics settings) land here. Account ids are numeric, so this name never collides.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set.
pub fn shared_profile_dir() -> Result<PathBuf, StoreError> {
    Ok(profiles_root()?.join("shared"))
}

fn profiles_root() -> Result<PathBuf, StoreError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or(StoreError::NoLocalAppData)?;
    Ok(PathBuf::from(local_app_data)
        .join("Breakbar")
        .join("profiles"))
}

/// [`profile_dir`], creating it (and its parents) first if it doesn't exist yet.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set or the folder can't be created.
pub fn ensure_profile_dir(account_id: AccountId) -> Result<PathBuf, StoreError> {
    let dir = profile_dir(account_id)?;
    std::fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
        path: dir.clone(),
        source,
    })?;
    Ok(dir)
}

/// Part of the name of a profile folder that is being deleted (`<id>.deleting-<n>`). Account ids
/// are numbers, so it never collides with a live profile.
const TRASH_MARKER: &str = ".deleting-";

/// Permanently deletes the account's profile folder, including its `Local.dat` (the remembered
/// login), and returns once it is gone. A missing folder is not an error. Only ever touches
/// `profiles\<id>`: the shared profile has a non-numeric name and can't be addressed through an
/// [`AccountId`]. The window uses [`trash_profile`] instead, so it doesn't wait for the disk.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set or the folder can't be removed.
pub fn delete_profile(account_id: AccountId) -> Result<(), StoreError> {
    match trash_profile(account_id)? {
        Some(trash) => remove_folder(&trash),
        None => Ok(()),
    }
}

/// Takes the account's profile folder out of the way at once, for the caller to remove with
/// [`remove_folder`] in the background: the folder is renamed, which takes no time however much
/// it holds (the game launcher's cache in `Temp` can be large). Returns the folder to remove, or
/// `None` if the account had no profile. If the folder can't be renamed (something inside is
/// still open), the folder itself is returned.
///
/// A folder that is never removed (Breakbar ended in between) is picked up by
/// [`sweep_leftovers`] at the next start.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set.
pub fn trash_profile(account_id: AccountId) -> Result<Option<PathBuf>, StoreError> {
    let dir = profile_dir(account_id)?;
    Ok(trash_in(&dir))
}

/// Removes a folder returned by [`trash_profile`]. A missing folder is not an error.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the folder can't be removed.
pub fn remove_folder(dir: &Path) -> Result<(), StoreError> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io {
            path: dir.to_owned(),
            source,
        }),
    }
}

fn trash_in(dir: &Path) -> Option<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    if !dir.exists() {
        return None;
    }
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let trash = dir.with_file_name(format!(
        "{name}{TRASH_MARKER}{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    match std::fs::rename(dir, &trash) {
        Ok(()) => Some(trash),
        Err(_) => Some(dir.to_owned()),
    }
}

/// Removes the profile folders left behind by a deletion that didn't finish. Returns how many
/// were removed. Best-effort: what can't be removed now stays for the next start.
#[must_use]
pub fn sweep_leftovers() -> usize {
    profiles_root().map_or(0, |root| sweep_in(&root))
}

fn sweep_in(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(TRASH_MARKER))
        .filter(|entry| remove_folder(&entry.path()).is_ok())
        .count()
}

/// Removes what has been in the account's `Temp` folder (the temp folder of its client) for longer
/// than `older_than`, and returns how many entries that were. The folder only ever grows
/// otherwise. Best-effort: an entry that is still in use can't be removed and stays. Call it only
/// while no client of the account runs.
#[must_use]
pub fn prune_temp(account_id: AccountId, older_than: Duration) -> usize {
    profile_dir(account_id).map_or(0, |dir| prune_in(&dir.join("Temp"), older_than))
}

fn prune_in(temp: &Path, older_than: Duration) -> usize {
    let Some(cutoff) = SystemTime::now().checked_sub(older_than) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(temp) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified < cutoff)
        })
        .filter(|entry| {
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path).is_ok()
            } else {
                std::fs::remove_file(path).is_ok()
            }
        })
        .count()
}

/// Path to the account's `Local.dat`, inside its profile, once GW2 has created it.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set.
pub fn local_dat_path(account_id: AccountId) -> Result<PathBuf, StoreError> {
    Ok(profile_dir(account_id)?
        .join("Guild Wars 2")
        .join("Local.dat"))
}

/// Whether the account has been set up, i.e. its profile has its own `Local.dat`.
///
/// A client started with `-shareArchive` cannot create a missing `Local.dat` (it fails with
/// "data archive cannot be opened"), so an account without one needs a single setup launch
/// without `-shareArchive` first. Whether the file also holds remembered credentials can't be
/// told from outside; that only decides whether `-autologin` skips the login screen.
#[must_use]
pub fn is_set_up(account_id: AccountId) -> bool {
    local_dat_path(account_id).is_ok_and(|path| path.is_file())
}

/// The file that records the game build a setup launch of this account last finished against.
fn verified_build_path(account_id: AccountId) -> Result<PathBuf, StoreError> {
    Ok(profile_dir(account_id)?.join("build-verified.txt"))
}

/// The game build (see `game_build` in the app) the account's `Local.dat` was last brought up to
/// date against by a setup launch, if it has been. A `Local.dat` that is older than the game but
/// was verified for this very build is not out of date: the client only rewrites it when it has
/// something to change.
#[must_use]
pub fn verified_build(account_id: AccountId) -> Option<u64> {
    std::fs::read_to_string(verified_build_path(account_id).ok()?)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Records that a setup launch of the account finished against game build `build`.
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set or the record can't be written.
pub fn mark_build_verified(account_id: AccountId, build: u64) -> Result<(), StoreError> {
    ensure_profile_dir(account_id)?;
    let path = verified_build_path(account_id)?;
    std::fs::write(&path, build.to_string()).map_err(|source| StoreError::Io { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dir_is_under_local_app_data() {
        let local_app_data = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA is set");
        let dir = profile_dir(AccountId(42)).unwrap();
        assert!(dir.starts_with(&local_app_data));
        assert!(dir.ends_with(r"Breakbar\profiles\42"));
    }

    #[test]
    fn deleting_a_profile_removes_its_folder() {
        // A never-used id, so no real profile is touched.
        let id = AccountId(u32::MAX - 7);
        let dir = ensure_profile_dir(id).unwrap();
        std::fs::create_dir_all(dir.join("Guild Wars 2")).unwrap();
        std::fs::write(dir.join("Guild Wars 2").join("Local.dat"), b"x").unwrap();

        delete_profile(id).unwrap();
        assert!(!dir.exists());
        // Deleting again is fine.
        delete_profile(id).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("breakbar-profiles-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_trashed_folder_is_out_of_the_way_at_once() {
        let root = scratch("trash");
        let profile = root.join("7");
        std::fs::create_dir_all(profile.join("Temp")).unwrap();
        std::fs::write(profile.join("Temp").join("cache"), b"x").unwrap();

        let trash = trash_in(&profile).unwrap();

        assert!(
            !profile.exists(),
            "the profile is gone under its name right away"
        );
        assert!(trash.join("Temp").join("cache").is_file());
        assert!(
            trash
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("7.deleting-")
        );
        remove_folder(&trash).unwrap();
        assert!(!trash.exists());
        // Nothing to trash, and removing what is gone, are both fine.
        assert_eq!(trash_in(&profile), None);
        remove_folder(&trash).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn leftovers_of_a_deletion_are_swept_but_profiles_are_not() {
        let root = scratch("sweep");
        for name in ["1", "shared", "2.deleting-99-0", "3.deleting-99-1"] {
            std::fs::create_dir_all(root.join(name).join("Guild Wars 2")).unwrap();
        }

        assert_eq!(sweep_in(&root), 2);

        assert!(root.join("1").is_dir());
        assert!(root.join("shared").is_dir());
        assert!(!root.join("2.deleting-99-0").exists());
        assert!(!root.join("3.deleting-99-1").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_old_entries_of_the_temp_folder_are_pruned() {
        let temp = scratch("prune");
        let old = SystemTime::now() - Duration::from_hours(30 * 24);
        for name in ["old-file", "new-file"] {
            std::fs::write(temp.join(name), b"x").unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(temp.join("old-file"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::fs::create_dir_all(temp.join("old-dir")).unwrap();
        std::fs::write(temp.join("old-dir").join("inner"), b"x").unwrap();
        // A folder's own time changes when something is added to it, so age the folder last.
        std::fs::File::open(temp.join("old-dir"))
            .and_then(|dir| dir.set_modified(old))
            .ok();

        let removed = prune_in(&temp, Duration::from_hours(14 * 24));

        assert!(removed >= 1);
        assert!(!temp.join("old-file").exists());
        assert!(temp.join("new-file").is_file());
        assert_eq!(prune_in(&temp.join("missing"), Duration::ZERO), 0);
        std::fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn fresh_account_is_not_set_up() {
        // A random, never-used id: nothing has ever created this profile's Local.dat.
        assert!(!is_set_up(AccountId(u32::MAX)));
    }

    #[test]
    fn ensure_profile_dir_creates_the_folder() {
        let id = AccountId(u32::MAX - 1);
        let dir = ensure_profile_dir(id).unwrap();
        assert!(dir.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
