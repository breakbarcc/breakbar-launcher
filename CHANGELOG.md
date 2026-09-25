# Changelog

All notable changes to Breakbar Launcher. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), the version numbers follow
[Semantic Versioning](https://semver.org/) (see [CONTRIBUTING.md](CONTRIBUTING.md#versioning)).

Everything before 0.2.0 is in the git history and in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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
