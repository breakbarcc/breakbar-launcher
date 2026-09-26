# Changelog

All notable changes to Breakbar Launcher. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), the version numbers follow
[Semantic Versioning](https://semver.org/) (see [CONTRIBUTING.md](CONTRIBUTING.md#versioning)).

Everything before 0.2.0 is in the git history and in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## [0.9.1] - 2026-09-26

### Changed

- The window no longer waits for the disk: settings are written by a background thread (changes
  that come in meanwhile are merged, and a failed write still shows a message), and deleting an
  account removes its profile folder in the background. A deleted account's folder that could not be
  removed before Breakbar ended is removed at the next start.
- A row of the account list is updated on its own instead of rebuilding the whole list on every
  change, which avoids flicker and keeps hover states.
- The instance switcher stops checking the account list and the foreground window while it is
  hidden. An idle Breakbar in the tray now wakes up only for the login check every 5 seconds.

### Fixed

- The temp folders of the accounts (`%LOCALAPPDATA%\Breakbar\profiles\<id>\Temp`), which only grew
  before, are cleaned at start: entries that were not touched for 14 days are removed (accounts that
  are running are skipped).

## [0.9.0] - 2026-09-26

### Added

- A log file, `%LOCALAPPDATA%\Breakbar\breakbar.log`, for problems that had no place to show up:
  failed starts (also from shortcuts), a launch lock that couldn't be taken, and the profile folder
  being pointed back after an unfinished start. It keeps at most about 512 KB (the current file and
  one `.old`) and never holds logins.

### Changed

- Internal clean-up only, nothing you can see: the window code is split into modules, the code
  passes Clippy's stricter `pedantic` checks, and outdated comments were brought up to date.

## [0.8.1] - 2026-09-26

### Fixed

- Setting up Steam accounts no longer runs `cmd` to create the link to the installation, so a game
  folder with `&` or `%` in its path can't be misread as a command.
- If Breakbar crashed or was killed while starting an account, `%APPDATA%\Guild Wars 2` stayed
  pointed at that account, and a game started outside Breakbar would have used its login. Breakbar
  now points it back at the shared profile the next time it starts.
- Closing Guild Wars 2's single-instance mutex no longer gives up when one of the game's other
  mutexes has a name that can't be read; it keeps looking for the right one.
- Internal: the buffers read from Windows' handle tables are properly aligned.

## [0.8.0] - 2026-09-25

### Added

- Game update detection: an account whose `Local.dat` is older than the installed game (its
  `Gw2.dat`) shows "Login required", and its start is a setup launch, which refreshes the login
  (a start with `-shareArchive` can't). A banner above the list, "The game was updated", names the
  affected accounts, and "Set up one after another" refreshes them one by one: the first starts at
  once, each further one is offered after the previous finished. The check runs locally every
  5 seconds, no network.

## [0.7.1] - 2026-09-25

### Fixed

- Fine tuning of the account row: the status icon sits a little lower next to its label, and the
  companion icon is 20 px and level with the action button.

## [0.7.0] - 2026-09-25

### Added

- The official "Made with Slint" badge in the About section (replacing the text placeholder), in a
  variant for each theme; a click opens slint.dev.

## [0.6.5] - 2026-09-25

### Fixed

- The status icon in the account rows sat about a pixel above the letters of its label (the text
  line has room for descenders below the letters); it is moved down to be level with them.

## [0.6.4] - 2026-09-25

### Fixed

- In the account list the grip, the checkbox, the companion icon and the status icon (ring, dot,
  key, ...) sat at the top of their row instead of being centered; the status icon is now level with
  the text behind it.

### Changed

- The companion apps are marked with the Blish HUD icon (dimmed, with an accent dot while they are
  running) instead of the generic layers icon.

## [0.6.3] - 2026-09-25

### Fixed

- The instance switcher was still see-through at 100 % opacity: its background was translucent by
  itself, on top of the opacity setting. The background is now opaque, so the setting alone decides
  (100 % is solid, the default of 58 % looks as designed).

## [0.6.2] - 2026-09-25

### Changed

- The instance switcher's position is only written to `config.toml` once the drag is over (the
  position stays the same for two updates in a row) instead of on every change while it is dragged.

## [0.6.1] - 2026-09-25

### Fixed

- The instance switcher could be unreachable after a restart: its position was saved while it was
  hidden, as the parked position (-32000, -32000) that Windows reports for a hidden window. Such a
  position is no longer saved, and a saved position that is not on any monitor is ignored.

## [0.6.0] - 2026-09-25

### Added

- Settings for the instance switcher: show it or not, only while a client is running, lock its
  position and its opacity at rest (30 to 100 percent, fully opaque under the pointer).

## [0.5.0] - 2026-09-25

### Added

- A language setting (System, English, Deutsch) in the settings. System follows the Windows display
  language, as before; the choice is applied at once and stored in `config.toml`.

## [0.4.0] - 2026-09-25

### Changed

- The application is called "Breakbar Launcher" everywhere: the program is now
  `breakbar-launcher.exe` (was `breakbar.exe`) and the crate `breakbar-launcher`, the file properties
  and the command line help use the full name. "breakbar" stays the name of the organization.
  Autostart entries and desktop shortcuts that point to `breakbar.exe` have to be created again.

## [0.3.0] - 2026-09-25

### Added

- Version information in `breakbar.exe` (file properties, task manager): file and product version,
  description, company and copyright.

## [0.2.0] - 2026-09-25

### Added

- The version number, shown in the new "About" section of the settings (with website and license
  links, the "Made with Slint" note and the trademark notice).
