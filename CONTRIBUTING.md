# Contributing

Thanks for your interest! By submitting a pull request you agree that your contribution is licensed
under the project's [MIT License](LICENSE).

## Working on the UI

The UI is written in [Slint](https://slint.dev) (`crates/bb-app/ui`). With the Slint extension for
VS Code:

- **Look at a component on its own:** open a `.slint` file and run "Slint: Show Preview". Open
  `ui/dev-preview.slint` for the account card in all of its states (idle, running, starting, login
  needed, error, locked, selected) with sample data; that file is only for the preview and not part
  of the app. The preview's toolbar switches between light and dark.
- **Change the running app:** build with `SLINT_LIVE_PREVIEW=1` and the feature `slint/live-preview`
  (`cargo run --features slint/live-preview`, into a separate `--target-dir` so the normal build is
  left alone); saving a `.slint` file then reloads the UI without a restart. Rust changes still need
  a rebuild.
- **Screenshots without a window:** see "Previews without a window" in
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Versioning

Breakbar follows [Semantic Versioning](https://semver.org/). The version lives in one place,
`[workspace.package] version` in the root `Cargo.toml` (every crate inherits it), and is shown in
the settings.

- A commit that changes what users see or do bumps the version and adds an entry to
  [CHANGELOG.md](CHANGELOG.md), in the same commit. Docs, tests, refactoring and CI commits do not
  bump it.
- While the version is `0.x`: a new feature or a breaking change (config, command line) raises the
  minor number (`0.2.0` to `0.3.0`), a bug fix raises the patch number (`0.2.0` to `0.2.1`). From
  `1.0.0` on breaking changes raise the major number, features the minor and fixes the patch number.
- The commit that bumps the version is tagged `vX.Y.Z` (`Cargo.lock` is committed with it).

Before opening a PR:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```
