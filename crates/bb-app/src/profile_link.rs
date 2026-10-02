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
//!    started outside Breakbar use the shared profile again. GW2 reads those settings by path too,
//!    at some point during its own startup that isn't tied to taking `Local.dat`, so depending on
//!    timing it may do so before or after this step: [`seed_shared_with_account_settings`] is
//!    called right before it to make sure either way sees this account's own settings.
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

/// Copies `account_id`'s own `GFXSettings*.xml` files into the shared profile, overwriting
/// whatever is there. A no-op if the account has none of its own yet (nothing was ever captured
/// for it, e.g. via Configure mode) — the shared profile is simply left as it is.
///
/// Call this right before [`point_to_shared`] at the end of a launch: GW2 re-reads its
/// graphics/audio settings *by path* at some point during startup that isn't tied to when it takes
/// `Local.dat` (see the module doc), so depending on exact timing it may do so before or after the
/// junction has been pointed back at the shared profile. Seeding the shared profile with this
/// account's own settings first means it sees the same, correct values either way, instead of
/// silently falling back to whatever the shared profile happened to hold (typically some other
/// account's settings, from whichever one played last).
///
/// # Errors
///
/// Returns [`ProfileLinkError`] if `%LOCALAPPDATA%` is not set or a file can't be read or written.
pub fn seed_shared_with_account_settings(account_id: AccountId) -> Result<(), ProfileLinkError> {
    let source = account_gw2_dir(account_id)?;
    let Ok(entries) = fs::read_dir(&source) else {
        return Ok(());
    };
    let files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_gfx_settings_file(path))
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    let target = shared_gw2_dir()?;
    ensure_dir(&target)?;
    for file in files {
        let name = file.file_name().expect("listed from read_dir");
        copy_settings_file(&file, &target.join(name), account_id)
            .map_err(|source| ProfileLinkError::Io { path: file, source })?;
    }
    Ok(())
}

/// Copies a `GFXSettings*.xml` file to `to`, rewriting the launch command line it records
/// (`EXECCMD`) so its `-mumble Breakbar_<id>` names `account_id`. The client wrote that line
/// for the account the file came from, and it was seen to start with default settings when the
/// line named another account than the one starting.
pub(crate) fn copy_settings_file(
    from: &Path,
    to: &Path,
    account_id: AccountId,
) -> std::io::Result<()> {
    match fs::read_to_string(from) {
        Ok(text) => fs::write(to, with_mumble_name(&text, account_id)),
        Err(_) => fs::copy(from, to).map(drop),
    }
}

fn with_mumble_name(text: &str, account_id: AccountId) -> String {
    const MARKER: &str = "-mumble Breakbar_";
    let Some(start) = text.find(MARKER).map(|at| at + MARKER.len()) else {
        return text.to_owned();
    };
    let digits = text[start..].bytes().take_while(u8::is_ascii_digit).count();
    format!(
        "{}{}{}",
        &text[..start],
        account_id.0,
        &text[start + digits..]
    )
}

/// Whether `path` is one of GW2's `GFXSettings.<exe name>.xml` files.
pub(crate) fn is_gfx_settings_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("GFXSettings"))
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
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

/// Undoes what Breakbar ever did to `%APPDATA%\Guild Wars 2`: removes the junction and, if the
/// shared profile still holds data (the common case), moves it back into the real folder, exactly
/// restoring the layout Guild Wars 2 had before Breakbar's first start ([`ensure_linked`] in
/// reverse). Per-account profiles are untouched; only the shared one is ever moved in or out of the
/// real folder. Does nothing if the real folder was never linked.
///
/// Call this only while no launch is in progress and no client runs (same requirement as
/// [`restore_shared_if_stranded`]).
///
/// # Errors
///
/// Returns [`ProfileLinkError`] if `%APPDATA%` is not set or the folder can't be removed or moved.
pub fn unlink() -> Result<(), ProfileLinkError> {
    let real = real_gw2_dir()?;
    if !is_reparse_point(&real)? {
        return Ok(());
    }
    fs::remove_dir(&real).map_err(|source| ProfileLinkError::Io {
        path: real.clone(),
        source,
    })?;
    let shared = shared_gw2_dir()?;
    if shared.exists() {
        fs::rename(&shared, &real).map_err(|source| ProfileLinkError::Io {
            path: shared,
            source,
        })?;
    }
    Ok(())
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
    fn seeding_copies_the_account_gfx_settings_into_the_shared_profile() {
        with_isolated_appdata("seed", |root| {
            fs::create_dir_all(profile(root, "1")).unwrap();
            fs::write(
                profile(root, "1").join("GFXSettings.Gw2-64.exe.xml"),
                b"account settings",
            )
            .unwrap();
            fs::create_dir_all(profile(root, "shared")).unwrap();
            fs::write(
                profile(root, "shared").join("GFXSettings.Gw2-64.exe.xml"),
                b"stale shared settings",
            )
            .unwrap();

            seed_shared_with_account_settings(AccountId(1)).unwrap();

            assert_eq!(
                fs::read(profile(root, "shared").join("GFXSettings.Gw2-64.exe.xml")).unwrap(),
                b"account settings"
            );
        });
    }

    #[test]
    fn copies_name_the_target_account_in_the_recorded_command_line() {
        let xml = r#"<EXECCMD Value="Gw2-64.exe -shareArchive -mumble Breakbar_1 -fps:60"/>"#;
        assert_eq!(
            with_mumble_name(xml, AccountId(12)),
            r#"<EXECCMD Value="Gw2-64.exe -shareArchive -mumble Breakbar_12 -fps:60"/>"#
        );
        assert_eq!(with_mumble_name("<a/>", AccountId(2)), "<a/>");
    }

    #[test]
    fn seeding_is_a_no_op_without_an_account_settings_file() {
        with_isolated_appdata("seed-empty", |root| {
            fs::create_dir_all(profile(root, "shared")).unwrap();
            fs::write(
                profile(root, "shared").join("GFXSettings.Gw2-64.exe.xml"),
                b"shared settings",
            )
            .unwrap();

            seed_shared_with_account_settings(AccountId(1)).unwrap();

            assert_eq!(
                fs::read(profile(root, "shared").join("GFXSettings.Gw2-64.exe.xml")).unwrap(),
                b"shared settings"
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
    fn unlink_restores_the_pre_breakbar_layout() {
        with_isolated_appdata("unlink", |root| {
            fs::create_dir_all(real(root)).unwrap();
            fs::write(real(root).join("Local.dat"), b"existing data").unwrap();
            point_to_shared().unwrap();
            assert!(is_reparse_point(&real(root)).unwrap());

            unlink().unwrap();

            assert!(!is_reparse_point(&real(root)).unwrap());
            assert_eq!(
                fs::read(real(root).join("Local.dat")).unwrap(),
                b"existing data"
            );
            assert!(!profile(root, "shared").exists());
        });
    }

    #[test]
    fn unlink_does_nothing_if_never_linked() {
        with_isolated_appdata("unlink-fresh", |root| {
            assert!(unlink().is_ok());
            assert!(!real(root).exists());
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
