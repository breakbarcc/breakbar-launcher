//! Pointing the real `%APPDATA%\Guild Wars 2` at an account's own profile.
//!
//! Guild Wars 2 only ever reads and writes `%APPDATA%\Guild Wars 2` — see [`bb_win::junction`]'s
//! module docs for why redirecting a child process's own environment doesn't work. This module
//! instead retargets that real folder itself, via an NTFS junction, to whichever account is
//! about to launch.

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bb_core::AccountId;

/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, thiserror::Error)]
pub enum ProfileLinkError {
    #[error("the APPDATA environment variable is not set")]
    NoAppData,
    #[error("could not determine the account's profile folder: {0}")]
    Profile(#[from] bb_store::StoreError),
    #[error("could not prepare {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} already holds data for a different account; \
         not overwriting it with {other_account_dir}"
    )]
    WouldOverwriteAnotherAccount {
        path: PathBuf,
        other_account_dir: PathBuf,
    },
    #[error("could not link Guild Wars 2's data folder to this account's profile: {0}")]
    Link(String),
}

/// Points the real `%APPDATA%\Guild Wars 2` at `account_id`'s own profile folder, so the next
/// launch reads and writes that account's own `Local.dat` instead of whichever account (if any)
/// used it last.
///
/// Does nothing if it's already pointed there. The very first time this runs for any account, if
/// `%APPDATA%\Guild Wars 2` is still a real, ordinary folder — i.e. Breakbar has never linked it
/// before — that folder, and whatever login it already holds, is *moved* (not deleted, not
/// copied) into `account_id`'s profile, so the first account you launch through Breakbar keeps
/// its existing login instead of losing it.
pub fn activate(account_id: AccountId) -> Result<(), ProfileLinkError> {
    let real_gw2_dir = real_gw2_dir()?;
    let account_gw2_dir = bb_store::profile_dir(account_id)?.join("Guild Wars 2");

    if already_linked_to(&real_gw2_dir, &account_gw2_dir) {
        return Ok(());
    }

    if is_reparse_point(&real_gw2_dir)? {
        // Already linked to a *different* account (or a stale link) — just retarget it.
        ensure_dir(&account_gw2_dir)?;
    } else if real_gw2_dir.is_dir() {
        // First run: adopt the existing installation's data instead of orphaning it.
        if account_gw2_dir.exists() {
            return Err(ProfileLinkError::WouldOverwriteAnotherAccount {
                path: real_gw2_dir,
                other_account_dir: account_gw2_dir,
            });
        }
        if let Some(parent) = account_gw2_dir.parent() {
            ensure_dir(parent)?;
        }
        fs::rename(&real_gw2_dir, &account_gw2_dir).map_err(|source| ProfileLinkError::Io {
            path: real_gw2_dir.clone(),
            source,
        })?;
    } else {
        ensure_dir(&account_gw2_dir)?;
    }

    // `real_gw2_dir` no longer exists (just moved away above) or never did: recreate it as a
    // plain empty directory so the junction has something to attach its reparse point to.
    ensure_dir(&real_gw2_dir)?;
    bb_win::junction::create(&real_gw2_dir, &account_gw2_dir)
        .map_err(|error| ProfileLinkError::Link(error.to_string()))?;
    Ok(())
}

fn real_gw2_dir() -> Result<PathBuf, ProfileLinkError> {
    let appdata = std::env::var_os("APPDATA").ok_or(ProfileLinkError::NoAppData)?;
    Ok(PathBuf::from(appdata).join("Guild Wars 2"))
}

fn ensure_dir(path: &Path) -> Result<(), ProfileLinkError> {
    fs::create_dir_all(path).map_err(|source| ProfileLinkError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Whether `path` currently has a reparse point (junction or symlink) set on it.
fn is_reparse_point(path: &Path) -> Result<bool, ProfileLinkError> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path).map_err(|source| ProfileLinkError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// Whether `link` is a reparse point that already resolves to `target`, without needing to parse
/// the reparse point's raw data ourselves: `canonicalize` follows it and returns the real path.
fn already_linked_to(link: &Path, target: &Path) -> bool {
    let (Ok(link), Ok(target)) = (fs::canonicalize(link), fs::canonicalize(target)) else {
        return false;
    };
    link == target
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_isolated_appdata;
    use bb_core::AccountId;

    #[test]
    fn fresh_install_just_creates_and_links_the_account_folder() {
        with_isolated_appdata("fresh", |root| {
            let id = AccountId(1);

            activate(id).unwrap();

            let real = root.join("Roaming").join("Guild Wars 2");
            let account = root
                .join("Local")
                .join("Breakbar")
                .join("profiles")
                .join("1")
                .join("Guild Wars 2");
            assert!(is_reparse_point(&real).unwrap());
            assert!(already_linked_to(&real, &account));
        });
    }

    #[test]
    fn existing_installation_is_moved_not_deleted() {
        with_isolated_appdata("migrate", |root| {
            let real = root.join("Roaming").join("Guild Wars 2");
            fs::create_dir_all(&real).unwrap();
            fs::write(real.join("Local.dat"), b"pretend save data").unwrap();

            activate(AccountId(1)).unwrap();

            let account = root
                .join("Local")
                .join("Breakbar")
                .join("profiles")
                .join("1")
                .join("Guild Wars 2");
            assert_eq!(
                fs::read(account.join("Local.dat")).unwrap(),
                b"pretend save data"
            );
            assert!(is_reparse_point(&real).unwrap());
            assert_eq!(
                fs::read(real.join("Local.dat")).unwrap(),
                b"pretend save data"
            );
        });
    }

    #[test]
    fn switching_accounts_retargets_without_touching_either_profile() {
        with_isolated_appdata("switch", |root| {
            activate(AccountId(1)).unwrap();
            let real = root.join("Roaming").join("Guild Wars 2");
            fs::write(real.join("Local.dat"), b"account one's data").unwrap();

            activate(AccountId(2)).unwrap();
            fs::write(real.join("Local.dat"), b"account two's data").unwrap();

            activate(AccountId(1)).unwrap();

            let profile_1 = root
                .join("Local")
                .join("Breakbar")
                .join("profiles")
                .join("1")
                .join("Guild Wars 2");
            let profile_2 = root
                .join("Local")
                .join("Breakbar")
                .join("profiles")
                .join("2")
                .join("Guild Wars 2");
            assert_eq!(
                fs::read(profile_1.join("Local.dat")).unwrap(),
                b"account one's data"
            );
            assert_eq!(
                fs::read(profile_2.join("Local.dat")).unwrap(),
                b"account two's data"
            );
            assert!(already_linked_to(&real, &profile_1));
        });
    }

    #[test]
    fn activating_the_same_account_twice_is_a_cheap_no_op() {
        with_isolated_appdata("idempotent", |root| {
            activate(AccountId(1)).unwrap();
            let real = root.join("Roaming").join("Guild Wars 2");
            fs::write(real.join("Local.dat"), b"data").unwrap();

            activate(AccountId(1)).unwrap();

            // Still there and unchanged — a real re-link would have gone through the
            // create-empty-dir-then-junction path again, which is harmless here anyway, but the
            // early return means it never even touched the filesystem a second time.
            assert_eq!(fs::read(real.join("Local.dat")).unwrap(), b"data");
        });
    }
}
