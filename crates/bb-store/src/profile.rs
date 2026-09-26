//! Per-account profile locations.
//!
//! Every account gets its own folder under `%LOCALAPPDATA%\Breakbar\profiles\<id>\`, holding its
//! `Local.dat`, its temp folder and the record of the game build it was set up for. The game only
//! ever reads `%APPDATA%\Guild Wars 2`, so that real folder is a junction which points at an
//! account's profile only while that account's client starts (see `profile_link` in the app).
//! The `shared` profile is what it points at the rest of the time.

use std::path::PathBuf;

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

/// Permanently deletes the account's profile folder, including its `Local.dat` (the remembered
/// login). A missing folder is not an error. Only ever touches `profiles\<id>`: the shared
/// profile has a non-numeric name and can't be addressed through an [`AccountId`].
///
/// # Errors
///
/// Returns [`StoreError`] if `%LOCALAPPDATA%` is not set or the folder can't be removed.
pub fn delete_profile(account_id: AccountId) -> Result<(), StoreError> {
    let dir = profile_dir(account_id)?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io { path: dir, source }),
    }
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
