//! Locating and validating the Guild Wars 2 client.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use bb_core::{pe, steam};
use bb_win::registry::{self, Root};

/// File name of the 64-bit game client.
pub const GW2_EXE: &str = "Gw2-64.exe";

/// `ProductName` in the client's version resource.
const PRODUCT_NAME: &str = "Guild Wars 2";

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Guild Wars 2";

#[derive(Debug, thiserror::Error)]
pub enum Gw2PathError {
    #[error("The selected file does not exist.")]
    NotFound,
    #[error("The selected file could not be read: {0}")]
    Unreadable(#[from] io::Error),
    #[error("The selected file is not a Windows program.")]
    NotExecutable,
    #[error("Only the 64-bit client ({GW2_EXE}) is supported.")]
    Not64Bit,
    #[error("The selected file is not the Guild Wars 2 client.")]
    NotGw2,
}

/// Checks that `path` points to the 64-bit Guild Wars 2 client.
pub fn validate(path: &Path) -> Result<(), Gw2PathError> {
    if !path.is_file() {
        return Err(Gw2PathError::NotFound);
    }

    let mut header = Vec::with_capacity(pe::HEADER_PROBE_LEN);
    File::open(path)?
        .take(pe::HEADER_PROBE_LEN as u64)
        .read_to_end(&mut header)?;
    match pe::machine(&header) {
        None => return Err(Gw2PathError::NotExecutable),
        Some(pe::MACHINE_AMD64) => {}
        Some(_) => return Err(Gw2PathError::Not64Bit),
    }

    let is_gw2 = match bb_win::version::product_name(path) {
        Some(product) => product == PRODUCT_NAME,
        None => path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(GW2_EXE)),
    };
    if is_gw2 {
        Ok(())
    } else {
        Err(Gw2PathError::NotGw2)
    }
}

/// Searches the usual install locations and returns the first valid client.
///
/// Order: ArenaNet installer registry entry, uninstall entry, Steam libraries, Program Files.
pub fn detect() -> Option<PathBuf> {
    candidates().into_iter().find(|path| validate(path).is_ok())
}

/// Marker file the Steam client writes into the game folder when it installs Guild Wars 2 (its
/// uninstall hook, which itself runs `Gw2-64.exe -provider Steam`).
const STEAM_INSTALL_SCRIPT: &str = "install_script.vdf";

/// Whether the client at `gw2_exe` can sign in through Steam.
///
/// Steam does not ship `steam_api64.dll` with the game (checked against a real installation), so
/// that cannot be used to tell the installations apart. The Steam client only leaves its
/// `install_script.vdf` in a folder it installed the game into, which includes an ArenaNet folder
/// that was linked into a Steam library with a directory junction.
pub fn supports_steam(gw2_exe: &Path) -> bool {
    gw2_exe.with_file_name(STEAM_INSTALL_SCRIPT).is_file()
}

/// The client to start a Steam account with: `preferred` if it supports Steam, otherwise a valid
/// Steam installation found in the Steam libraries.
pub fn steam_client(preferred: &Path) -> Option<PathBuf> {
    if supports_steam(preferred) {
        return Some(preferred.to_owned());
    }
    steam_candidates()
        .into_iter()
        .find(|path| supports_steam(path) && validate(path).is_ok())
}

fn candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for subkey in [
        r"SOFTWARE\ArenaNet\Guild Wars 2",
        r"SOFTWARE\WOW6432Node\ArenaNet\Guild Wars 2",
    ] {
        candidates.extend(registry::read_string(Root::LocalMachine, subkey, "Path").map(exe_path));
    }

    for root in [Root::LocalMachine, Root::CurrentUser] {
        candidates.extend(
            registry::read_string(root, UNINSTALL_KEY, "DisplayIcon")
                .map(|icon| exe_path(strip_icon_index(&icon).to_owned())),
        );
    }

    candidates.extend(steam_candidates());

    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            candidates.push(Path::new(&dir).join("Guild Wars 2").join(GW2_EXE));
        }
    }

    candidates
}

/// The Steam client's folder, if Steam is installed.
pub fn steam_dir() -> Option<PathBuf> {
    registry::read_string(Root::CurrentUser, r"Software\Valve\Steam", "SteamPath")
        .map(PathBuf::from)
}

fn steam_candidates() -> Vec<PathBuf> {
    let Some(steam_dir) = steam_dir() else {
        return Vec::new();
    };

    let mut libraries = std::fs::read_to_string(steam_dir.join(r"steamapps\libraryfolders.vdf"))
        .map(|vdf| steam::library_folders(&vdf))
        .unwrap_or_default();
    if libraries.is_empty() {
        libraries.push(steam_dir);
    }

    libraries
        .into_iter()
        .map(|library| library.join(steam::GW2_INSTALL_DIR).join(GW2_EXE))
        .collect()
}

/// Registry entries may point to the install directory or to the executable itself.
fn exe_path(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    if path.is_dir() {
        path.join(GW2_EXE)
    } else {
        path
    }
}

/// Turns an icon reference like `"C:\Games\Gw2-64.exe",0` into a plain path.
fn strip_icon_index(icon: &str) -> &str {
    let icon = icon.trim();
    let icon = match icon.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => path,
        _ => icon,
    };
    icon.trim().trim_matches('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_icon_index_and_quotes() {
        assert_eq!(
            strip_icon_index(r#""C:\Games\Gw2-64.exe",0"#),
            r"C:\Games\Gw2-64.exe"
        );
        assert_eq!(
            strip_icon_index(r"C:\Games\Gw2-64.exe"),
            r"C:\Games\Gw2-64.exe"
        );
        assert_eq!(
            strip_icon_index(r"C:\Games,Stuff\Gw2-64.exe"),
            r"C:\Games,Stuff\Gw2-64.exe"
        );
    }

    /// Diagnostic aid: `cargo test -p breakbar-launcher -- --ignored --nocapture print_candidates`
    #[test]
    #[ignore = "depends on the local installation"]
    fn print_candidates() {
        for candidate in candidates() {
            println!("{} -> {:?}", candidate.display(), validate(&candidate));
        }
    }

    #[test]
    fn steam_support_needs_the_steam_install_script_next_to_the_client() {
        let dir = std::env::temp_dir().join(format!("breakbar-steam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(GW2_EXE);
        assert!(!supports_steam(&exe));

        std::fs::write(dir.join(STEAM_INSTALL_SCRIPT), b"").unwrap();
        assert!(supports_steam(&exe));
        assert_eq!(steam_client(&exe), Some(exe.clone()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_is_rejected() {
        assert!(matches!(
            validate(Path::new(r"C:\does\not\exist\Gw2-64.exe")),
            Err(Gw2PathError::NotFound)
        ));
    }

    #[test]
    fn non_executable_is_rejected() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(matches!(
            validate(&manifest),
            Err(Gw2PathError::NotExecutable)
        ));
    }

    #[test]
    fn other_program_is_rejected() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let notepad = Path::new(&windir).join("System32").join("notepad.exe");
        assert!(matches!(validate(&notepad), Err(Gw2PathError::NotGw2)));
    }
}
