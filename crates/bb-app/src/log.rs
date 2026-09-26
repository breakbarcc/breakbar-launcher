//! A small log file for problems that have nowhere else to go.
//!
//! Breakbar is a GUI program without a console, so `eprintln!` reaches nobody. What it can't tell
//! the user in the moment (a launch lock that couldn't be taken, a failed start from a shortcut) is
//! written to `%LOCALAPPDATA%\Breakbar\breakbar.log` instead, so that there is something to look at
//! when a report comes in. Nothing secret is ever logged: no logins, no file contents.

use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The log is moved to `breakbar.log.old` (replacing an older one) once it grows past this.
const MAX_BYTES: u64 = 256 * 1024;

/// Appends `message` with the time and the process id. Best-effort: a log that can't be written
/// must never get in the way of what the program is doing.
pub fn write(message: impl Display) {
    let Some(path) = log_path() else {
        return;
    };
    let line = format!(
        "{} [{}] {message}\n",
        bb_win::time::local_timestamp(),
        std::process::id()
    );
    let _ = append(&path, &line, MAX_BYTES);
}

fn log_path() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local_app_data)
            .join("Breakbar")
            .join("breakbar.log"),
    )
}

fn append(path: &Path, line: &str, max_bytes: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() > max_bytes) {
        // Another Breakbar process may rotate at the same moment; whoever loses just carries on.
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    // One `write_all` of a whole line, in append mode: lines of two processes don't interleave.
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_isolated_appdata;

    #[test]
    fn lines_are_appended_with_time_and_process_id() {
        with_isolated_appdata("log-append", |_| {
            write("first");
            write(format_args!("second {}", 2));

            let text = fs::read_to_string(log_path().unwrap()).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 2);
            assert!(lines[0].ends_with(&format!("[{}] first", std::process::id())));
            assert!(lines[1].ends_with("second 2"));
            // "2026-09-26 14:02:11 [..." starts with the date.
            assert_eq!(lines[0].as_bytes()[4], b'-');
        });
    }

    #[test]
    fn a_full_log_is_moved_aside() {
        with_isolated_appdata("log-rotate", |_| {
            let path = log_path().unwrap();
            append(&path, "old line\n", 5).unwrap();
            append(&path, "second old line\n", 5).unwrap();
            append(&path, "new line\n", 5).unwrap();

            assert_eq!(fs::read_to_string(&path).unwrap(), "new line\n");
            assert_eq!(
                fs::read_to_string(path.with_extension("log.old")).unwrap(),
                "second old line\n"
            );
        });
    }
}
