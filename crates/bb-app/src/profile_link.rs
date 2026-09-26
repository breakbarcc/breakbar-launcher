//! Pointing the real `%APPDATA%\Guild Wars 2` at an account's own profile during a launch.
//!
//! Guild Wars 2 only ever reads and writes `%APPDATA%\Guild Wars 2` (see [`bb_win::junction`] for
//! why the environment cannot redirect that). That real folder is therefore turned into an NTFS
//! junction which normally points at a shared default profile, and is pointed at an account's own
//! profile only for the few seconds it takes that account's client to start and take its
//! `Local.dat`:
//!
//! 1. [`point_to_account`] — the client about to start will find that account's `Local.dat`.
//! 2. The client starts and opens `Local.dat` exclusively; from then on it only uses that open
//!    handle for it (verified: it stays locked all session and in-game writes go through it).
//! 3. [`point_to_shared`] — anything later opened by path (graphics settings) and any client
//!    started outside Breakbar use the shared profile again.
//!
//! This mirrors Launchbuddy's approach (a `Local.dat` symlink swapped only during launch), but a
//! junction needs no admin rights or Developer Mode, and because GW2 re-creates `Local.dat` on
//! startup, a folder-level link also keeps that fresh file inside the account's profile.
//! Retargeting the junction while other clients run is safe: it changes the folder's reparse
//! point, not the files those clients hold open.

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bb_core::AccountId;

/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const GW2_FOLDER: &str = "Guild Wars 2";

#[derive(Debug, thiserror::Error)]
pub enum ProfileLinkError {
    #[error("the APPDATA environment variable is not set")]
    NoAppData,
    #[error("could not determine the profile folder: {0}")]
    Profile(#[from] bb_store::StoreError),
    #[error("could not prepare {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "both {real} and {shared} exist as ordinary folders; move one of them away so \
         Breakbar doesn't overwrite either"
    )]
    Conflict { real: PathBuf, shared: PathBuf },
    #[error("could not link Guild Wars 2's data folder: {0}")]
    Link(String),
}

/// The folder the account's client reads and writes as `%APPDATA%\Guild Wars 2`.
pub fn account_gw2_dir(account_id: AccountId) -> Result<PathBuf, ProfileLinkError> {
    Ok(bb_store::profile_dir(account_id)?.join(GW2_FOLDER))
}

/// Points the real `%APPDATA%\Guild Wars 2` at `account_id`'s own profile.
pub fn point_to_account(account_id: AccountId) -> Result<(), ProfileLinkError> {
    point_to(&account_gw2_dir(account_id)?)
}

/// Points the real `%APPDATA%\Guild Wars 2` back at the shared default profile.
pub fn point_to_shared() -> Result<(), ProfileLinkError> {
    point_to(&shared_gw2_dir()?)
}

/// Points `%APPDATA%\Guild Wars 2` back at the shared profile if it is a junction that points
/// somewhere else, which is what a launch that never finished (a crash, the process killed) leaves
/// behind. Returns whether it had to be repointed. Does nothing if the folder is no junction, so it
/// never migrates an installation on its own.
///
/// Call it only while no launch is in progress (see `launcher::recover_profile_link`).
pub fn restore_shared_if_stranded() -> Result<bool, ProfileLinkError> {
    if !is_reparse_point(&real_gw2_dir()?)? {
        return Ok(false);
    }
    let shared = shared_gw2_dir()?;
    if already_linked_to(&real_gw2_dir()?, &shared) {
        return Ok(false);
    }
    point_to(&shared)?;
    Ok(true)
}

fn point_to(target: &Path) -> Result<(), ProfileLinkError> {
    let real = real_gw2_dir()?;
    ensure_linked(&real)?;
    if already_linked_to(&real, target) {
        return Ok(());
    }
    ensure_dir(target)?;
    bb_win::junction::create(&real, target)
        .map_err(|error| ProfileLinkError::Link(error.to_string()))
}

/// Makes sure `real` is a junction, creating the shared profile if needed.
///
/// The first time Breakbar runs, `real` is still an ordinary folder holding the existing
/// installation's data. That folder is *moved* (never deleted or overwritten) to become the
/// shared profile, so clients started outside Breakbar keep working exactly as before.
fn ensure_linked(real: &Path) -> Result<(), ProfileLinkError> {
    let shared = shared_gw2_dir()?;

    if is_reparse_point(real)? {
        return ensure_dir(&shared);
    }

    if real.is_dir() {
        if shared.exists() {
            return Err(ProfileLinkError::Conflict {
                real: real.to_owned(),
                shared,
            });
        }
        if let Some(parent) = shared.parent() {
            ensure_dir(parent)?;
        }
        fs::rename(real, &shared).map_err(|source| ProfileLinkError::Io {
            path: real.to_owned(),
            source,
        })?;
    } else {
        ensure_dir(&shared)?;
    }

    // `real` was just moved away or never existed: recreate it as an empty directory so the
    // junction has something to attach its reparse point to.
    ensure_dir(real)?;
    bb_win::junction::create(real, &shared)
        .map_err(|error| ProfileLinkError::Link(error.to_string()))
}

fn real_gw2_dir() -> Result<PathBuf, ProfileLinkError> {
    let appdata = std::env::var_os("APPDATA").ok_or(ProfileLinkError::NoAppData)?;
    Ok(PathBuf::from(appdata).join(GW2_FOLDER))
}

fn shared_gw2_dir() -> Result<PathBuf, ProfileLinkError> {
    Ok(bb_store::shared_profile_dir()?.join(GW2_FOLDER))
}

fn ensure_dir(path: &Path) -> Result<(), ProfileLinkError> {
    fs::create_dir_all(path).map_err(|source| ProfileLinkError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Whether `path` currently has a reparse point (junction or symlink) set on it.
fn is_reparse_point(path: &Path) -> Result<bool, ProfileLinkError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ProfileLinkError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Whether `link` already resolves to `target`, without parsing the reparse point ourselves:
/// `canonicalize` follows it and returns the real path.
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

    fn real(root: &Path) -> PathBuf {
        root.join("Roaming").join(GW2_FOLDER)
    }

    fn profile(root: &Path, name: &str) -> PathBuf {
        root.join("Local")
            .join("Breakbar")
            .join("profiles")
            .join(name)
            .join(GW2_FOLDER)
    }

    #[test]
    fn fresh_install_links_to_an_empty_shared_profile() {
        with_isolated_appdata("fresh", |root| {
            point_to_shared().unwrap();

            assert!(is_reparse_point(&real(root)).unwrap());
            assert!(already_linked_to(&real(root), &profile(root, "shared")));
        });
    }

    #[test]
    fn existing_installation_becomes_the_shared_profile() {
        with_isolated_appdata("migrate", |root| {
            fs::create_dir_all(real(root)).unwrap();
            fs::write(real(root).join("Local.dat"), b"existing data").unwrap();

            point_to_shared().unwrap();

            let shared = profile(root, "shared");
            assert_eq!(
                fs::read(shared.join("Local.dat")).unwrap(),
                b"existing data"
            );
            assert!(is_reparse_point(&real(root)).unwrap());
            assert_eq!(
                fs::read(real(root).join("Local.dat")).unwrap(),
                b"existing data"
            );
        });
    }

    #[test]
    fn launch_window_switches_to_the_account_and_back() {
        with_isolated_appdata("switch", |root| {
            fs::create_dir_all(real(root)).unwrap();
            fs::write(real(root).join("Local.dat"), b"shared data").unwrap();

            point_to_account(AccountId(1)).unwrap();
            assert!(already_linked_to(&real(root), &profile(root, "1")));
            fs::write(real(root).join("Local.dat"), b"account one").unwrap();

            point_to_shared().unwrap();
            assert_eq!(
                fs::read(real(root).join("Local.dat")).unwrap(),
                b"shared data"
            );
            assert_eq!(
                fs::read(profile(root, "1").join("Local.dat")).unwrap(),
                b"account one"
            );
        });
    }

    #[test]
    fn junction_from_an_older_version_is_reused() {
        with_isolated_appdata("legacy", |root| {
            // Earlier builds left the real folder linked straight to an account profile.
            let account = profile(root, "2");
            fs::create_dir_all(&account).unwrap();
            fs::create_dir_all(real(root)).unwrap();
            bb_win::junction::create(&real(root), &account).unwrap();

            point_to_shared().unwrap();

            assert!(already_linked_to(&real(root), &profile(root, "shared")));
            assert!(account.is_dir(), "the account profile must be left alone");
        });
    }

    #[test]
    fn a_launch_that_never_finished_is_undone() {
        with_isolated_appdata("stranded", |root| {
            fs::create_dir_all(real(root)).unwrap();
            point_to_shared().unwrap();
            assert!(!restore_shared_if_stranded().unwrap());

            // Breakbar died between pointing at the account and pointing back.
            point_to_account(AccountId(1)).unwrap();
            assert!(already_linked_to(&real(root), &profile(root, "1")));

            assert!(restore_shared_if_stranded().unwrap());
            assert!(already_linked_to(&real(root), &profile(root, "shared")));
        });
    }

    #[test]
    fn an_ordinary_folder_is_left_alone_when_recovering() {
        with_isolated_appdata("stranded-plain", |root| {
            fs::create_dir_all(real(root)).unwrap();
            assert!(!restore_shared_if_stranded().unwrap());
            assert!(!is_reparse_point(&real(root)).unwrap());
        });
    }

    #[test]
    fn two_ordinary_folders_are_never_overwritten() {
        with_isolated_appdata("conflict", |root| {
            fs::create_dir_all(real(root)).unwrap();
            fs::create_dir_all(profile(root, "shared")).unwrap();

            assert!(matches!(
                point_to_shared(),
                Err(ProfileLinkError::Conflict { .. })
            ));
            assert!(!is_reparse_point(&real(root)).unwrap());
        });
    }
}
