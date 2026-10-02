//! Copying one account's graphics/sound settings into others.
//!
//! Guild Wars 2 stores them in `GFXSettings.*.xml` under `%APPDATA%\Guild Wars 2`, the same folder
//! `profile_link` junctions per account (see `launcher::LaunchMode::Configure` for how a file like
//! that gets into an account's own profile in the first place). `Local.dat` is never touched here:
//! it holds the account's login, not its settings.

use std::fs;
use std::path::PathBuf;

use bb_core::AccountId;

use crate::{launcher, profile_link};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("account is currently running")]
    Running(AccountId),
    #[error("could not locate the profile folder: {0}")]
    Profile(#[from] profile_link::ProfileLinkError),
    #[error("could not copy {file}: {source}")]
    Copy {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Copies every `GFXSettings*.xml` file found in `source`'s profile into each of `targets`'.
/// Returns how many target accounts actually received a file: `0` if `source` has no settings of
/// its own yet (only the shared profile does, until it has gone through `Configure` mode once).
/// Refuses, without copying anything, if `source` or any of `targets` is currently running.
///
/// # Errors
///
/// Returns [`SyncError`] if an account is running, or a file can't be read, created or copied.
pub fn sync_settings(source: AccountId, targets: &[AccountId]) -> Result<usize, SyncError> {
    if launcher::account_running(source) {
        return Err(SyncError::Running(source));
    }
    for &target in targets {
        if launcher::account_running(target) {
            return Err(SyncError::Running(target));
        }
    }

    let source_dir = profile_link::account_gw2_dir(source)?;
    let files = gfx_settings_files(&source_dir);
    if files.is_empty() {
        return Ok(0);
    }

    for &target in targets {
        let target_dir = profile_link::account_gw2_dir(target)?;
        fs::create_dir_all(&target_dir).map_err(|source| SyncError::Copy {
            file: target_dir.clone(),
            source,
        })?;
        for file in &files {
            let name = file.file_name().expect("listed from read_dir");
            profile_link::copy_settings_file(file, &target_dir.join(name), target).map_err(
                |source| SyncError::Copy {
                    file: file.clone(),
                    source,
                },
            )?;
        }
    }
    Ok(targets.len())
}

/// The `GFXSettings*.xml` files directly inside `dir`, if any.
fn gfx_settings_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| profile_link::is_gfx_settings_file(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_isolated_appdata;

    #[test]
    fn copies_every_gfx_settings_file_to_every_target() {
        with_isolated_appdata("sync-settings", |_root| {
            let source_dir = profile_link::account_gw2_dir(AccountId(1)).unwrap();
            fs::create_dir_all(&source_dir).unwrap();
            fs::write(source_dir.join("GFXSettings.Gw2-64.exe.xml"), b"video").unwrap();
            fs::write(source_dir.join("GFXSettings.Gw2.exe.xml"), b"video-32").unwrap();
            fs::write(source_dir.join("Local.dat"), b"login").unwrap();

            let synced = sync_settings(AccountId(1), &[AccountId(2), AccountId(3)]).unwrap();

            assert_eq!(synced, 2);
            for id in [AccountId(2), AccountId(3)] {
                let dir = profile_link::account_gw2_dir(id).unwrap();
                assert_eq!(
                    fs::read(dir.join("GFXSettings.Gw2-64.exe.xml")).unwrap(),
                    b"video"
                );
                assert_eq!(
                    fs::read(dir.join("GFXSettings.Gw2.exe.xml")).unwrap(),
                    b"video-32"
                );
                assert!(!dir.join("Local.dat").exists(), "login must not be copied");
            }
        });
    }

    #[test]
    fn nothing_to_sync_yet_is_not_an_error() {
        with_isolated_appdata("sync-settings-empty", |_root| {
            let synced = sync_settings(AccountId(1), &[AccountId(2)]).unwrap();
            assert_eq!(synced, 0);
        });
    }
}
