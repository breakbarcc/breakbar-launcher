//! Making an ArenaNet installation usable for Steam accounts.
//!
//! Steam only signs in a Guild Wars 2 that it installed itself (its own `Gw2-64.exe` build, and
//! `install_script.vdf` next to it), but the Steam version is a 1:1 copy of the ArenaNet one. So a
//! directory junction from Steam's library folder to the existing installation, followed by
//! "Install" in Steam, makes Steam adopt the files and download only a few megabytes.

use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use bb_core::steam::GW2_INSTALL_DIR;

use crate::game;

/// `CREATE_NO_WINDOW`: no console flashing up for the `mklink` helper.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A directory junction from Steam's library to the existing installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Where Steam expects the game: `<Steam>\steamapps\common\Guild Wars 2`.
    pub link: PathBuf,
    /// The installation the link points to.
    pub target: PathBuf,
}

/// What is missing before Steam accounts can be started.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// No link yet: it can be created.
    CreateLink(Link),
    /// The link exists, but Steam has not installed (adopted) the game yet.
    AwaitInstall(Link),
    /// Something else already occupies the folder Steam would install into.
    Blocked(PathBuf),
    /// Steam is not installed (or its folder is not where the registry says).
    NoSteam,
}

/// `None` if a Steam-capable installation is ready already; otherwise what is left to do.
pub fn plan(gw2_exe: &Path) -> Option<Plan> {
    if game::steam_client(gw2_exe).is_some() {
        return None;
    }
    let Some(steam_dir) = game::steam_dir() else {
        return Some(Plan::NoSteam);
    };
    Some(plan_in(&steam_dir, gw2_exe))
}

fn plan_in(steam_dir: &Path, gw2_exe: &Path) -> Plan {
    if !steam_dir.join("steamapps").is_dir() {
        return Plan::NoSteam;
    }
    let Some(target) = gw2_exe.parent() else {
        return Plan::NoSteam;
    };
    let link = Link {
        link: steam_dir.join(GW2_INSTALL_DIR),
        target: target.to_owned(),
    };
    if std::fs::symlink_metadata(&link.link).is_err() {
        return Plan::CreateLink(link);
    }
    if points_to(&link) {
        Plan::AwaitInstall(link)
    } else {
        Plan::Blocked(link.link)
    }
}

/// Whether `link.link` resolves to the same folder as `link.target`.
fn points_to(link: &Link) -> bool {
    match (
        std::fs::canonicalize(&link.link),
        std::fs::canonicalize(&link.target),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Creates the junction (no administrator rights needed). Uses the shell's `mklink /J`.
pub fn create_link(link: &Link) -> io::Result<()> {
    if let Some(parent) = link.link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&link.link)
        .arg(&link.target)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "mklink exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if points_to(link) {
        Ok(())
    } else {
        Err(io::Error::other(
            "the link does not point to the installation",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("breakbar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plans_follow_the_state_of_the_steam_folder() {
        let root = temp("steam-plan");
        let steam = root.join("Steam");
        let gw2 = root.join("Games").join("Guild Wars 2");
        std::fs::create_dir_all(&gw2).unwrap();
        let exe = gw2.join(game::GW2_EXE);

        assert_eq!(plan_in(&steam, &exe), Plan::NoSteam);

        std::fs::create_dir_all(steam.join(r"steamapps\common")).unwrap();
        let Plan::CreateLink(link) = plan_in(&steam, &exe) else {
            panic!("expected a link to create");
        };
        assert_eq!(link.target, gw2);

        create_link(&link).unwrap();
        assert_eq!(plan_in(&steam, &exe), Plan::AwaitInstall(link.clone()));

        // A real folder (or a link to somewhere else) in Steam's place is never touched.
        let other = root.join("Other");
        std::fs::create_dir_all(&other).unwrap();
        let other_exe = other.join(game::GW2_EXE);
        assert_eq!(
            plan_in(&steam, &other_exe),
            Plan::Blocked(link.link.clone())
        );

        // Removes only the junction, not the installation behind it.
        std::fs::remove_dir(&link.link).unwrap();
        assert!(gw2.is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
