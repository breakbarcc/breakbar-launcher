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

## Releasing

A release is published by pushing to the `release` branch; the workflow
[`.github/workflows/release.yml`](.github/workflows/release.yml) does the rest:

1. Make sure the version in the root `Cargo.toml` is the new one and that [CHANGELOG.md](CHANGELOG.md)
   has a `## [x.y.z]` section for it (see [Versioning](#versioning)).
2. Push it: `git push origin main:release` (or merge `main` into `release` and push that).
3. The workflow reads the version from the crate, stops if it is already released or missing from
   the changelog, runs the tests, builds `breakbar-launcher.exe` in release mode and creates the
   GitHub release `vx.y.z` for that commit. The release notes are the changelog section.

The release contains three files with fixed names, so that
`https://github.com/breakbarcc/breakbar-launcher/releases/latest/download/<file>` always points to the
newest one:

- `breakbar-launcher.exe`, the program (what the download button of the website links to)
- `breakbar-launcher.exe.sha256`, its SHA-256 checksum
- `latest.json`, the update manifest: `version`, `url`, `sha256`, `signed`, `released`, `notes`

### Repository settings

The rules that protect the branches and the version tags are kept as ruleset files in
[`.github/rulesets`](.github/rulesets), so that they can be applied again or reviewed:

- `release-branch.json`: `release` cannot be deleted or force-pushed, and only repository admins can
  push to it (publishing a release is a push to this branch, so it is limited to them).
- `main-branch.json`: `main` cannot be deleted or force-pushed and needs the `build` job of the CI
  workflow to pass; repository admins can bypass it.
- `release-tags.json`: version tags (`v*`) can be neither moved nor deleted, by anyone. Creating them
  stays possible, which the release workflow needs.

Apply them under Settings, Rules, Rulesets, "New ruleset", "Import a ruleset", or with the GitHub CLI
(`gh auth login` first):

```bash
gh api --method POST repos/breakbarcc/breakbar-launcher/rulesets --input .github/rulesets/release-branch.json
gh api --method POST repos/breakbarcc/breakbar-launcher/rulesets --input .github/rulesets/main-branch.json
gh api --method POST repos/breakbarcc/breakbar-launcher/rulesets --input .github/rulesets/release-tags.json
```

To let another person publish releases, give them the admin role or add their role to the
`bypass_actors` of `release-branch.json`. Not covered by rulesets, set in Settings: a `release`
environment (deployment branch `release` only) that holds the SignPath secrets, and the default
workflow permission "Read repository contents" under Actions, General.

### Code signing (SignPath)

Releases are unsigned until the project is approved by the
[SignPath Foundation](https://signpath.org/terms.html) (free for open source projects; it wants a
public repository with an OSI license, a released version and a download page that describes the
program). Once it is approved:

1. Create the repository variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG` and
   `SIGNPATH_SIGNING_POLICY_SLUG` and the secret `SIGNPATH_API_TOKEN` (Settings, Secrets and
   variables, Actions).
2. Set the variable `SIGNPATH_ENABLED` to `true`.

The workflow then sends the built executable to SignPath, waits for the signed one and publishes
that instead; `signed` in `latest.json` becomes `true`. If signing fails, no release is published
(it never falls back to an unsigned one). The signing job has not been run yet: check its inputs
against SignPath's documentation for the GitHub connector when setting it up.

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
