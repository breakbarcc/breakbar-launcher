//! Checking `ArenaNet`'s public build API for a Guild Wars 2 update Breakbar hasn't downloaded yet.
//!
//! `game::login_outdated` already detects a patch that the local client has *already* installed
//! (comparing `Gw2.dat`'s write time against each account's `Local.dat`). What that can't see is a
//! patch that only exists on `ArenaNet`'s servers so far: the local client has to download it first,
//! which needs a normal (non-`-shareArchive`) start of the game itself, not something Breakbar can
//! detect from local files alone. This module fills that gap with one small network request.

use std::time::Duration;

/// Host and path of `ArenaNet`'s public, unauthenticated build endpoint. Response body is the plain
/// JSON `{"id": 152159}`.
const API_HOST: &str = "api.guildwars2.com";
const API_PATH: &str = "/v2/build";
/// Never lets a flaky connection hold up the poll thread for long; a timeout just means this tick
/// found out nothing, like any other failure here.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Asks `ArenaNet`'s servers for the current build id. `None` on any failure (network, timeout,
/// unexpected response) — there's nothing more useful to do with one failed background check than
/// to skip it and try again on the next tick.
#[must_use]
pub fn fetch_current_build() -> Option<u64> {
    let body = bb_win::http::get(API_HOST, API_PATH, TIMEOUT).ok()?;
    parse_build_id(&body)
}

/// Pulls the `id` field out of a `{"id": 152159}` response by hand: the payload has exactly one
/// field, so pulling in a JSON crate for it is not worth the dependency.
fn parse_build_id(body: &str) -> Option<u64> {
    let after_key = body.split("\"id\"").nth(1)?;
    let after_colon = after_key.split_once(':')?.1;
    let digits: String = after_colon
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// What an update check found: whether to show the banner, and the baseline to remember for next
/// time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    pub update_available: bool,
    /// `(server_build, local_gw2dat_mtime)` to hand to `bb_store::set_known_server_build`.
    pub new_baseline: (u64, u64),
}

/// Decides whether a new update is available, from the local `Gw2.dat` write time
/// (`game::game_build`), the build id `ArenaNet`'s servers currently report, and the last stored
/// baseline (`bb_store::known_server_build`), if any.
///
/// `ArenaNet`'s build id has no local counterpart Breakbar can read (the client's file version
/// resource doesn't carry it), so the baseline instead remembers the local `Gw2.dat` write time
/// the server build was captured against. Whenever that write time has since moved on, the local
/// client was patched in the meantime — by Breakbar or otherwise, even while Breakbar wasn't
/// running — so the old baseline no longer says anything about the current install: `check`
/// re-baselines against the freshly fetched server build instead of reporting a stale mismatch. No
/// baseline yet (first run) is treated the same way, so there's no false positive on first use.
#[must_use]
pub fn check(local_mtime: u64, server_build: u64, baseline: Option<(u64, u64)>) -> Check {
    match baseline {
        Some((known_build, known_mtime)) if known_mtime >= local_mtime => Check {
            update_available: server_build != known_build,
            new_baseline: (known_build, known_mtime),
        },
        _ => Check {
            update_available: false,
            new_baseline: (server_build, local_mtime),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_id_field_out_of_the_response() {
        assert_eq!(parse_build_id(r#"{"id":152159}"#), Some(152_159));
        assert_eq!(parse_build_id(r#"{"id": 152159}"#), Some(152_159));
        assert_eq!(parse_build_id(r#"{ "id" : 152159 }"#), Some(152_159));
        assert_eq!(parse_build_id(""), None);
        assert_eq!(parse_build_id(r#"{"error":"not found"}"#), None);
    }

    #[test]
    fn first_check_ever_stores_a_baseline_without_a_banner() {
        let result = check(1_000_000, 152_159, None);
        assert!(!result.update_available);
        assert_eq!(result.new_baseline, (152_159, 1_000_000));
    }

    #[test]
    fn matching_build_shows_no_banner() {
        let result = check(1_000_000, 152_159, Some((152_159, 1_000_000)));
        assert!(!result.update_available);
        assert_eq!(result.new_baseline, (152_159, 1_000_000));
    }

    #[test]
    fn a_newer_server_build_shows_the_banner_and_keeps_the_old_baseline() {
        let result = check(1_000_000, 152_200, Some((152_159, 1_000_000)));
        assert!(result.update_available);
        assert_eq!(
            result.new_baseline,
            (152_159, 1_000_000),
            "must not adopt the newer build before the local client actually has it"
        );
    }

    #[test]
    fn a_local_patch_since_the_baseline_re_baselines_and_hides_the_banner() {
        // The local Gw2.dat moved on since the baseline was captured (patched outside Breakbar,
        // or while Breakbar wasn't running): trust the server value as current again.
        let result = check(2_000_000, 152_200, Some((152_159, 1_000_000)));
        assert!(!result.update_available);
        assert_eq!(result.new_baseline, (152_200, 2_000_000));
    }
}
