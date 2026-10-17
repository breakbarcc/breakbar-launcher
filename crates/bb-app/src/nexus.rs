//! Installing and updating the Nexus addon loader (<https://github.com/RaidcoreGG/Nexus>).
//!
//! Nexus is a single `d3d11.dll` that lives next to `Gw2-64.exe`. Breakbar downloads it from the
//! project's GitHub releases, checks it against the digest GitHub reports, and swaps it in. A
//! different `d3d11.dll` that is already there (`ArcDPS` ships under the same name) is never
//! overwritten: Nexus's own instructions are to rename it to `d3d11_chainload.dll`, which Nexus
//! then loads, and that rename only happens when the caller says the user confirmed it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Host and path of GitHub's release API for Nexus, unauthenticated.
const API_HOST: &str = "api.github.com";
const API_PATH: &str = "/repos/RaidcoreGG/Nexus/releases/latest";
/// Download URLs are only followed when they point into Nexus's own release downloads.
const DOWNLOAD_PREFIX: &str = "https://github.com/RaidcoreGG/Nexus/releases/download/";
const DOWNLOAD_HOST: &str = "github.com";
/// The asset to install.
const ASSET_NAME: &str = "d3d11.dll";
/// Name of the loader inside the game folder.
pub const DLL_NAME: &str = "d3d11.dll";
/// Name Nexus expects a pre-existing `d3d11.dll` to be renamed to.
pub const CHAINLOAD_NAME: &str = "d3d11_chainload.dll";
/// Left behind in the game folder only while a download is in flight.
const PART_NAME: &str = "d3d11.dll.breakbar-part";
/// Text that is part of every Nexus build and of no other `d3d11.dll` a user is likely to have.
const MARKER: &[u8] = b"RaidcoreGG";

const API_TIMEOUT: Duration = Duration::from_secs(8);
/// Applies to each stage of the download, not the whole transfer.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// A Nexus release that can be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// Path on [`DOWNLOAD_HOST`] of the `d3d11.dll` asset.
    download_path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 from GitHub's `digest` field, if the release has one.
    pub sha256: Option<String>,
}

/// What is currently in the game folder under the name `d3d11.dll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    No,
    Nexus,
    /// Some other proxy DLL, for example `ArcDPS`.
    Foreign,
}

/// Where the Nexus installation stands, for the settings page and the update banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Foreign,
    Installed { update_available: bool },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Guild Wars 2 is running")]
    ClientRunning,
    #[error("another d3d11.dll is in the game folder")]
    ForeignDll,
    #[error("{CHAINLOAD_NAME} already exists")]
    ChainloadExists,
    #[error("the installed d3d11.dll is not Nexus")]
    NotNexus,
    #[error("the download did not match the published checksum")]
    Mismatch,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Asks GitHub for the latest Nexus release. `None` on any failure: a failed background check is
/// just skipped.
#[must_use]
pub fn fetch_latest() -> Option<Release> {
    let body = bb_win::http::get(API_HOST, API_PATH, API_TIMEOUT).ok()?;
    parse_release(&body)
}

/// Pulls the fields we need out of GitHub's release JSON by hand, like `gw2_build` does: the
/// payload is large but only four values matter, which doesn't justify a JSON dependency.
fn parse_release(body: &str) -> Option<Release> {
    let tag = string_field(body, "tag_name")?;
    let marker = format!("\"name\":\"{ASSET_NAME}\"");
    let after = body.split_once(marker.as_str())?.1;
    // The asset's own fields follow its name; stop before the next asset begins.
    let asset = after.split("\"name\":\"").next()?;
    let url = string_field(asset, "browser_download_url")?;
    let download_path = url
        .strip_prefix(DOWNLOAD_PREFIX)
        .map(|rest| format!("/RaidcoreGG/Nexus/releases/download/{rest}"))?;
    let size = number_field(asset, "size")?;
    let sha256 = string_field(asset, "digest")
        .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_ascii_lowercase));
    Some(Release {
        tag,
        download_path,
        size,
        sha256,
    })
}

/// The value of a `"key":"value"` pair. Escapes are not handled; none of the values we read
/// contain any.
fn string_field(json: &str, key: &str) -> Option<String> {
    let after = json.split_once(format!("\"{key}\"").as_str())?.1;
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let value = after.strip_prefix('"')?;
    Some(value.split('"').next()?.to_owned())
}

/// The value of a `"key":123` pair.
fn number_field(json: &str, key: &str) -> Option<u64> {
    let after = json.split_once(format!("\"{key}\"").as_str())?.1;
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The game folder (where `Gw2-64.exe` is) for a configured game path.
#[must_use]
pub fn game_dir(gw2_path: &Path) -> Option<&Path> {
    gw2_path.parent()
}

/// Whether `bytes` look like a Nexus build.
fn is_nexus(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
}

/// Looks at the `d3d11.dll` in `game_dir`.
///
/// # Errors
///
/// Returns the I/O error if the file exists but can't be read.
pub fn inspect(game_dir: &Path) -> io::Result<Installed> {
    match fs::read(game_dir.join(DLL_NAME)) {
        Ok(bytes) if is_nexus(&bytes) => Ok(Installed::Nexus),
        Ok(_) => Ok(Installed::Foreign),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Installed::No),
        Err(error) => Err(error),
    }
}

/// Combines what is installed with the latest release (if it could be fetched) into a [`Status`].
///
/// Whether an installed Nexus is current is decided by comparing it with the release's published
/// SHA-256, so it works the same for an installation Breakbar made, an older one and one the user
/// copied in by hand. A release without a digest, or no release at all (offline), never reports
/// an update.
#[must_use]
pub fn status(game_dir: &Path, latest: Option<&Release>) -> Status {
    match inspect(game_dir) {
        Ok(Installed::No) | Err(_) => Status::NotInstalled,
        Ok(Installed::Foreign) => Status::Foreign,
        Ok(Installed::Nexus) => {
            let update_available = latest
                .and_then(|release| release.sha256.as_deref())
                .is_some_and(|expected| {
                    bb_win::hash::sha256_file_hex(&game_dir.join(DLL_NAME))
                        .is_ok_and(|actual| actual != expected)
                });
            Status::Installed { update_available }
        }
    }
}

/// Downloads `release` and installs it into the folder of `gw2_path`.
///
/// `rename_foreign` is the user's confirmation to move an existing, non-Nexus `d3d11.dll` to
/// [`CHAINLOAD_NAME`]; without it such a file makes this fail with [`Error::ForeignDll`] before
/// anything is downloaded.
///
/// # Errors
///
/// See [`Error`]. Nothing in the game folder is changed unless the download verified.
pub fn install(gw2_path: &Path, release: &Release, rename_foreign: bool) -> Result<(), Error> {
    let dir = game_dir(gw2_path).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    if !crate::launcher::running_clients(gw2_path).is_empty() {
        return Err(Error::ClientRunning);
    }
    if inspect(dir)? == Installed::Foreign && !rename_foreign {
        return Err(Error::ForeignDll);
    }

    let part = dir.join(PART_NAME);
    let result = download(release, &part).and_then(|()| apply(dir, &part, rename_foreign));
    let _ = fs::remove_file(&part);
    result
}

fn download(release: &Release, part: &Path) -> Result<(), Error> {
    let written = bb_win::http::download(
        DOWNLOAD_HOST,
        &release.download_path,
        part,
        DOWNLOAD_TIMEOUT,
    )?;
    if written != release.size {
        return Err(Error::Mismatch);
    }
    match &release.sha256 {
        Some(expected) if bb_win::hash::sha256_file_hex(part)? != *expected => Err(Error::Mismatch),
        _ => Ok(()),
    }
}

/// Moves the verified download `part` into place as `d3d11.dll`, first moving a foreign
/// `d3d11.dll` out of the way if `rename_foreign` allows it. Undoes that move if the final step
/// fails, so a failed install leaves the folder as it was.
fn apply(dir: &Path, part: &Path, rename_foreign: bool) -> Result<(), Error> {
    let dll = dir.join(DLL_NAME);
    let chainload = dir.join(CHAINLOAD_NAME);

    let moved_foreign = match inspect(dir)? {
        Installed::Foreign => {
            if !rename_foreign {
                return Err(Error::ForeignDll);
            }
            if chainload.exists() {
                return Err(Error::ChainloadExists);
            }
            fs::rename(&dll, &chainload)?;
            true
        }
        Installed::Nexus | Installed::No => false,
    };

    if let Err(error) = fs::rename(part, &dll) {
        if moved_foreign {
            let _ = fs::rename(&chainload, &dll);
        }
        return Err(error.into());
    }
    Ok(())
}

/// Removes Nexus from the folder of `gw2_path` and gives a `d3d11_chainload.dll` its old name
/// back. Nexus's `addons` folder and settings are left alone.
///
/// # Errors
///
/// [`Error::NotNexus`] if the `d3d11.dll` there isn't Nexus; see [`Error`] for the rest.
pub fn uninstall(gw2_path: &Path) -> Result<(), Error> {
    let dir = game_dir(gw2_path).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    if !crate::launcher::running_clients(gw2_path).is_empty() {
        return Err(Error::ClientRunning);
    }
    match inspect(dir)? {
        Installed::Nexus => {}
        Installed::No => return Ok(()),
        Installed::Foreign => return Err(Error::NotNexus),
    }
    let dll: PathBuf = dir.join(DLL_NAME);
    fs::remove_file(&dll)?;
    let chainload = dir.join(CHAINLOAD_NAME);
    if chainload.exists() {
        fs::rename(&chainload, &dll)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE_JSON: &str = r#"{"url":"x","tag_name":"v2026.2.17.1210","name":"Release","assets":[{"url":"u","id":1,"name":"d3d11.dll","label":null,"uploader":{"login":"a"},"content_type":"application/x-msdownload","state":"uploaded","size":7618048,"digest":"sha256:8B123807FC2CCC090EB13286D5DF2628BA78F7B9980886C1AB3881E919CA2992","download_count":3,"browser_download_url":"https://github.com/RaidcoreGG/Nexus/releases/download/v2026.2.17.1210/d3d11.dll"},{"url":"u2","id":2,"name":"d3d11.pdb","size":33681408,"digest":"sha256:00","browser_download_url":"https://github.com/RaidcoreGG/Nexus/releases/download/v2026.2.17.1210/d3d11.pdb"}]}"#;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("breakbar-nexus-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn nexus_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 64];
        bytes.extend_from_slice(b"https://github.com/RaidcoreGG/Nexus");
        bytes
    }

    #[test]
    fn parses_the_release() {
        let release = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(release.tag, "v2026.2.17.1210");
        assert_eq!(release.size, 7_618_048);
        assert_eq!(
            release.download_path,
            "/RaidcoreGG/Nexus/releases/download/v2026.2.17.1210/d3d11.dll"
        );
        assert_eq!(
            release.sha256.as_deref(),
            Some("8b123807fc2ccc090eb13286d5df2628ba78f7b9980886c1ab3881e919ca2992")
        );
    }

    #[test]
    fn rejects_a_download_url_outside_the_nexus_releases() {
        let json = RELEASE_JSON.replace("github.com/RaidcoreGG/Nexus", "evil.example/x");
        assert_eq!(parse_release(&json), None);
    }

    #[test]
    fn a_release_without_a_digest_still_parses() {
        let json = RELEASE_JSON.replace("\"digest\":\"sha256:8B1", "\"dig\":\"sha256:8B1");
        assert_eq!(parse_release(&json).unwrap().sha256, None);
    }

    #[test]
    fn tells_nexus_from_another_proxy_dll() {
        assert!(is_nexus(&nexus_bytes()));
        assert!(!is_nexus(b"arcdps d3d11 proxy"));
    }

    #[test]
    fn installs_next_to_a_foreign_dll_by_chainloading_it() {
        let dir = temp_dir("chainload");
        fs::write(dir.join(DLL_NAME), b"arcdps").unwrap();
        let part = dir.join(PART_NAME);
        fs::write(&part, nexus_bytes()).unwrap();

        assert!(matches!(apply(&dir, &part, false), Err(Error::ForeignDll)));
        assert_eq!(fs::read(dir.join(DLL_NAME)).unwrap(), b"arcdps");

        apply(&dir, &part, true).unwrap();
        assert_eq!(inspect(&dir).unwrap(), Installed::Nexus);
        assert_eq!(fs::read(dir.join(CHAINLOAD_NAME)).unwrap(), b"arcdps");
        assert!(!part.exists());
    }

    #[test]
    fn never_overwrites_an_existing_chainload_file() {
        let dir = temp_dir("chainload-exists");
        fs::write(dir.join(DLL_NAME), b"arcdps").unwrap();
        fs::write(dir.join(CHAINLOAD_NAME), b"other").unwrap();
        let part = dir.join(PART_NAME);
        fs::write(&part, nexus_bytes()).unwrap();

        assert!(matches!(
            apply(&dir, &part, true),
            Err(Error::ChainloadExists)
        ));
        assert_eq!(fs::read(dir.join(DLL_NAME)).unwrap(), b"arcdps");
        assert_eq!(fs::read(dir.join(CHAINLOAD_NAME)).unwrap(), b"other");
    }

    #[test]
    fn updating_replaces_the_old_nexus() {
        let dir = temp_dir("update");
        fs::write(dir.join(DLL_NAME), nexus_bytes()).unwrap();
        let part = dir.join(PART_NAME);
        let mut newer = nexus_bytes();
        newer.push(1);
        fs::write(&part, &newer).unwrap();

        apply(&dir, &part, false).unwrap();
        assert_eq!(fs::read(dir.join(DLL_NAME)).unwrap(), newer);
        assert!(!dir.join(CHAINLOAD_NAME).exists());
    }

    /// Hits the real GitHub; run with `--ignored`.
    #[test]
    #[ignore = "needs network access"]
    fn downloads_and_verifies_the_latest_release() {
        let release = fetch_latest().expect("latest release");
        let dir = temp_dir("network");
        let part = dir.join(PART_NAME);
        download(&release, &part).expect("download should verify");
        assert!(is_nexus(&fs::read(&part).unwrap()));
    }

    #[test]
    fn status_reflects_the_folder_and_the_release() {
        let dir = temp_dir("status");
        let release = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(status(&dir, Some(&release)), Status::NotInstalled);

        fs::write(dir.join(DLL_NAME), b"arcdps").unwrap();
        assert_eq!(status(&dir, Some(&release)), Status::Foreign);

        fs::write(dir.join(DLL_NAME), nexus_bytes()).unwrap();
        assert_eq!(
            status(&dir, Some(&release)),
            Status::Installed {
                update_available: true
            }
        );
        assert_eq!(
            status(&dir, None),
            Status::Installed {
                update_available: false
            }
        );

        let current = Release {
            sha256: bb_win::hash::sha256_file_hex(&dir.join(DLL_NAME)).ok(),
            ..release
        };
        assert_eq!(
            status(&dir, Some(&current)),
            Status::Installed {
                update_available: false
            }
        );
    }
}
