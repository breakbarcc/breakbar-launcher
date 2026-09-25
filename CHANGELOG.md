# Changelog

All notable changes to Breakbar Launcher. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), the version numbers follow
[Semantic Versioning](https://semver.org/) (see [CONTRIBUTING.md](CONTRIBUTING.md#versioning)).

Everything before 0.2.0 is in the git history and in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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
