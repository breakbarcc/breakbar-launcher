//! Loading and saving Breakbar's configuration (`%APPDATA%\Breakbar\config.toml`).
//!
//! The config never contains credentials; logins live exclusively in per-account `Local.dat` files.

mod profile;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bb_core::{Account, CompanionApp};
use serde::{Deserialize, Serialize};

pub use profile::{
    delete_profile, ensure_profile_dir, is_set_up, local_dat_path, profile_dir, shared_profile_dir,
};

const CURRENT_VERSION: u32 = 1;

/// What happens to the main window right after starting an account (not on setup or a manual
/// stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfterStart {
    /// Leaves the window as it is.
    KeepOpen,
    /// Hides it to the tray icon, like the close button does.
    #[default]
    MinimizeToTray,
    /// Exits Breakbar entirely.
    Close,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the APPDATA environment variable is not set")]
    NoAppData,
    #[error("the LOCALAPPDATA environment variable is not set")]
    NoLocalAppData,
    #[error("I/O error on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid config file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// The persisted application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    /// Path to `Gw2-64.exe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gw2_path: Option<PathBuf>,
    #[serde(default, rename = "account", skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<Account>,
    #[serde(default, rename = "companion", skip_serializing_if = "Vec::is_empty")]
    pub companions: Vec<CompanionApp>,
    #[serde(default)]
    pub after_start: AfterStart,
    /// Last dragged-to position of the instance-switcher overlay. `None` before it's ever been
    /// moved, so it starts at a fixed default position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_position: Option<OverlayPosition>,
}

/// A saved screen position, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayPosition {
    pub x: i32,
    pub y: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            gw2_path: None,
            accounts: Vec::new(),
            companions: Vec::new(),
            after_start: AfterStart::default(),
            overlay_position: None,
        }
    }
}

/// Default location of the config file: `%APPDATA%\Breakbar\config.toml`.
pub fn default_config_path() -> Result<PathBuf, StoreError> {
    let app_data = std::env::var_os("APPDATA").ok_or(StoreError::NoAppData)?;
    Ok(PathBuf::from(app_data).join("Breakbar").join("config.toml"))
}

impl Config {
    /// Loads the config from `path`. A missing file yields the default config.
    pub fn load(path: &Path) -> Result<Self, StoreError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(StoreError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|source| StoreError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    /// Saves the config atomically: write a temp file, flush it to disk, then replace the target.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let text = toml::to_string_pretty(self)?;
        let io_err = |source| StoreError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let mut file = fs::File::create(&tmp).map_err(io_err)?;
        file.write_all(text.as_bytes()).map_err(io_err)?;
        file.sync_all().map_err(io_err)?;
        drop(file);
        fs::rename(&tmp, path).map_err(io_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_core::{AccountId, CompanionId, Provider};

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("breakbar-test-{}-{name}", std::process::id()))
            .join("config.toml")
    }

    #[test]
    fn missing_file_yields_default() {
        let config = Config::load(&temp_path("missing")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn round_trip() {
        let path = temp_path("roundtrip");
        let mut steam = Account::new(AccountId(2), "Alt");
        steam.provider = Provider::Steam;
        let mut main = Account::new(AccountId(1), "Main");
        main.companions.push(CompanionId(1));
        let config = Config {
            gw2_path: Some(r"C:\Games\Guild Wars 2\Gw2-64.exe".into()),
            accounts: vec![main, steam],
            companions: vec![CompanionApp::blish_hud(
                CompanionId(1),
                r"C:\Blish HUD\Blish HUD.exe",
            )],
            ..Config::default()
        };

        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
