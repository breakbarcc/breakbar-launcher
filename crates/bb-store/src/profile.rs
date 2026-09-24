//! Per-account profile locations.
//!
//! Every account gets its own folder under `%LOCALAPPDATA%\Breakbar\profiles\<id>\`. Pointing a
//! client's `APPDATA` (and `TMP`/`TEMP`) environment variables at that folder instead of the
//! real ones gives it its own `Local.dat`, GFX settings, and temp files — the mechanism the
//! architecture doc calls "Spike S1, Approach A". It needs no symlink, no Developer Mode/admin
//! rights, and places no restriction on how many clients can be launched at once, unlike
//! swapping a single shared `Local.dat` file in and out between launches.

use std::path::PathBuf;

use bb_core::AccountId;

use crate::StoreError;

/// The account's isolated profile folder. Does not create it; see [`ensure_profile_dir`].
pub fn profile_dir(account_id: AccountId) -> Result<PathBuf, StoreError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or(StoreError::NoLocalAppData)?;
    Ok(PathBuf::from(local_app_data)
        .join("Breakbar")
        .join("profiles")
        .join(account_id.0.to_string()))
}

/// [`profile_dir`], creating it (and its parents) first if it doesn't exist yet.
pub fn ensure_profile_dir(account_id: AccountId) -> Result<PathBuf, StoreError> {
    let dir = profile_dir(account_id)?;
    std::fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
        path: dir.clone(),
        source,
    })?;
    Ok(dir)
}

/// Path to the account's `Local.dat`, inside its profile, once GW2 has created it.
pub fn local_dat_path(account_id: AccountId) -> Result<PathBuf, StoreError> {
    Ok(profile_dir(account_id)?
        .join("Guild Wars 2")
        .join("Local.dat"))
}

/// Whether the account has a saved login yet, i.e. whether `-autologin` will do anything.
///
/// `false` before the account's first manual login, or if the account's profile can't be
/// located at all (treated as "needs login" rather than an error, since the caller's next step
/// is the same either way: let the user log in normally).
pub fn has_saved_login(account_id: AccountId) -> bool {
    local_dat_path(account_id).is_ok_and(|path| path.is_file())
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
    fn fresh_account_has_no_saved_login() {
        // A random, never-used id: nothing has ever created this profile's Local.dat.
        assert!(!has_saved_login(AccountId(u32::MAX)));
    }

    #[test]
    fn ensure_profile_dir_creates_the_folder() {
        let id = AccountId(u32::MAX - 1);
        let dir = ensure_profile_dir(id).unwrap();
        assert!(dir.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
