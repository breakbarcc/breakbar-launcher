# Breakbar Launcher – Architecture

## Goals

1. **Performance** – instant start, near-zero idle CPU, minimal RAM, no artificial waits.
2. **UX** – one click per account, "launch all", no blocking dialogs.

## Decisions

| Topic | Decision |
|---|---|
| Language | Rust (stable, `x86_64-pc-windows-msvc`), Win32/NT via the `windows` crate |
| UI | Slint (royalty-free desktop license → "About Slint" attribution required) |
| Credentials | **Only per-account `Local.dat` files.** No email/password is stored by Breakbar. |
| Accounts | ArenaNet + Steam |
| Companion apps | Generic model, Blish HUD is the first preset |
| License | MIT |
| Privileges | `asInvoker` – no admin rights required |

## Multi-launch mechanism

```
launch(account):
  1. OpenMutexW("AN-Mutex-Window-Guild Wars 2")  → absent: start directly
  2. present: for each GW2 PID we launched (fallback: all Gw2-64.exe of the current user)
       NtQueryInformationProcess(ProcessHandleInformation)   // handles of that PID only
       filter ObjectType == Mutant, then NtQueryObject(name)  // never query pipes (may hang)
       DuplicateHandle(..., DUPLICATE_CLOSE_SOURCE)
  3. prepare profile (Local.dat) + environment block
  4. CreateProcessW(Gw2-64.exe, "-shareArchive -autologin -mumble Breakbar_<id> [...]")
```

The mutex is closed lazily – only when another client is about to start. No system-wide handle scan,
no sleep/retry loops.

## Login via Local.dat

```
%APPDATA%\Guild Wars 2              → NTFS junction, normally → profiles\shared\Guild Wars 2
%LOCALAPPDATA%\Breakbar\profiles\
    shared\Guild Wars 2\            the original installation's folder (moved here once)
    <account-id>\Guild Wars 2\Local.dat
    <account-id>\Temp\              TMP/TEMP of that account's client
```

**Spike S1 result (revised after real-world testing):**

- GW2 ignores a redirected `APPDATA` environment variable (it resolves the folder through the
  Windows known-folder API), so only a link on the real `%APPDATA%\Guild Wars 2` works.
- A running client opens `Local.dat` **exclusively** at startup (no read, write, rename or delete
  possible from outside) and keeps it open all session; in-game writes go through that handle.
  Copying, renaming or hard-linking per launch is therefore impossible — only a reparse point on the
  path works. A folder junction needs no admin rights or Developer Mode (a file symlink would).
- The junction must only point at an account **during its launch** (same as Launchbuddy's
  `Local.dat` symlink swap): point at the account → start the client → wait until its `Local.dat` has
  been locked for a few seconds → point back at `shared`. Launches are serialized on one background
  worker. Leaving the junction on an account breaks already running clients the next time they open
  something by path.
- A client started with `-shareArchive` opens `Local.dat` **read-only**: it cannot create a missing
  one ("data archive cannot be opened") and never writes anything back — including a remembered
  login (verified: its `Local.dat` kept its old write time after a session with "remember" ticked).
  Saving a login therefore needs a **"Set up login" launch** without `-shareArchive`, which requires
  that no other client runs (such a client locks `Gw2.dat` exclusively). Accounts without a
  `Local.dat` get this automatically on their first start; whether an existing `Local.dat` holds
  credentials can't be told from outside, so the action is also available on demand.
- Graphics settings are written by path during play and so live in `shared` for all accounts
  (as with Launchbuddy); per-account graphics settings are a possible later extension.

**After a game patch** `Local.dat` files of an older build must be refreshed (one normal launch per
account). Breakbar detects the build mismatch and guides the user instead of failing.

## Steam accounts

- Start `Gw2-64.exe` directly with `-provider Steam` and env `SteamAppId=1284210` (same as
  gw2launcher). Launchbuddy's `steam://rungameid/1284210` route was rejected: it goes through Steam's
  custom-arguments confirmation and then has to find the spawned process.
- `Gw2-64.exe` loads `steam_api64.dll` only at runtime (not a static import), and that library ships
  only with the **Steam installation** of the game. Steam accounts therefore use the configured client
  if it has the DLL next to it, otherwise a Steam installation found in the Steam libraries;
  otherwise the launch is refused with an explanation.
- The Steam client must be running (checked before launch); it signs the game in with its currently
  signed-in Steam user, so **only one Steam account runs at a time** (enforced in the UI), next to
  any number of ArenaNet accounts.
- Steam accounts need no remembered login, only their own `Local.dat` (created by the automatic
  first start without `-shareArchive`).
- **Not yet verified against a real Steam-linked account** (none available during development).

## Companion apps

```rust
struct CompanionApp {
    name: String,
    exe: PathBuf,
    args: String,          // placeholders: {pid}, {mumble}, {account}
    scope: Scope,          // PerClient | Shared
    start_when: Trigger,   // ProcessStarted | WindowShown
    close_with_game: bool,
}
```

- **PerClient** (Blish HUD): one instance per GW2 client, closed when that client exits.
  Preset args: `--pid {pid} --mumble {mumble}`.
- **Shared**: one instance for all clients, reference-counted, closed when the last client exits.
- Each GW2 client gets a unique MumbleLink name (`Breakbar_<id>`) so overlays never read another client's data.
- Placeholders are substituted per argument after splitting the template, so a value with spaces
  (e.g. `{account}`) stays one argument. Companions run with their own folder as working directory.
- Everything runs on the client's monitor thread: start `ProcessStarted` apps → wait for the game
  window (only if an app needs it) → start `WindowShown` apps → wait for exit → close them.
- Closing is graceful: wait 3 s for the companion to exit on its own (Blish HUD follows its client
  out), then `WM_CLOSE` to its top-level windows (like `taskkill` without `/f`, lets it save its
  settings), terminate only after another 15 s. Apps with `close_with_game = false` are left running.
- A companion that fails to start is reported in the UI; the client and other companions keep running.

**Blish HUD specifics** (from its source, `ApplicationSettings.cs` / `Program.cs`):

- `--pid` / `-P` attaches to a process, `--mumble` / `-m` sets the MumbleLink name; `--settings` /
  `-s` would allow per-account settings via `{account}` (not used by the preset: all instances share
  Blish HUD's settings, as when started by hand).
- Its single-instance mutex is `<guid>:<mumble name>`, so one instance per client works as long as
  every client has its own MumbleLink name.
- It ignores the launcher/patcher window (class `ArenaNet`) and waits for the game window
  (`ArenaNet_Gr_Window_Class` for DX11, `ArenaNet_Dx_Window_Class` for DX9). Breakbar uses the same
  classes for `WindowShown`, so Blish HUD only starts once the account is past the launcher.
- With `--pid` it exits by itself when its client exits (it only restarts into the tray when
  started without `--pid`/`--startgw2`). Every instance therefore loads its modules anew, and only
  once the character is in game (verified in its logs; same as when started by hand).
- **Shared settings are last-writer-wins:** `SettingsService` reads `settings.json` once at start
  and writes its whole in-memory copy back on every change (4 s debounce) and on exit. With two
  instances running, a setting changed in one is overwritten when the other one saves or exits.
  Verified in its source; not fixable from outside. Options: accept and document it, or give each
  account its own `--settings` folder (decision pending).
- UI (until the design lands): Blish HUD's path is picked by file dialog; a per-account checkbox
  adds or removes it. There is no install location to auto-detect (it ships as a zip).

## Process monitoring

- All child handles are waited on by thread-pool waits (`RegisterWaitForSingleObject`) → 0 % CPU when idle.
- Window appearance per PID: currently `EnumWindows` every 250 ms, only while a client with
  `WindowShown` companions has not shown its game window yet (microseconds per check).
  `SetWinEventHook(EVENT_OBJECT_SHOW)` would avoid even that, but needs a message loop per waiting
  thread; revisit if measurements show a need.
- Error dialogs (`ArenaNet_Dialog_Class`, e.g. "needs to be patched before using -shareArchive") are
  detected and surfaced inline.

## User interface

- **Design source:** the design hand-off (spec `DESIGN.md`, screens for both themes, tokens, icons,
  app icon), exported from the design canvas in German. It is not part of the repository; keep it
  locally in `design/` (git-ignored). It is the reference for all UI work; sections 1 and 8 of
  `DESIGN.md` are the ground rules. Two of its screens are in `docs/images/` for the README.
- **Tokens:** `crates/bb-app/ui/theme.slint` is `design/tokens/theme.slint` (comments in English).
  Every color and size comes from it; `Theme.dark` follows the Windows app theme until a theme is
  chosen in the settings. The native title bar is kept (the spec allows it) and colored to match
  via DWM (`bb_win::window::set_frame`), also when the theme changes.
- **Icons:** `ui/icons.slint` is generated from `design/icons/ui/*.svg` into path commands drawn by
  Slint's `Path` (crisp at any scale, colored per use, no image decoding). Lucide, ISC license
  (`ui/LICENSE-lucide`).
- **App icon:** `assets/icon/breakbar.ico` (the pixel-tuned 16–256 px set from the design) is
  embedded as the exe icon; `breakbar-64.png` is the window icon.
- **Translations:** all user-visible text goes through Slint's `@tr` — texts the Rust side shows
  (toasts, error details) are functions of the `Messages` global in `ui/messages.slint`. English
  is the source language; German lives in `lang/de/LC_MESSAGES/breakbar.po` and is bundled at
  compile time; the language follows Windows. After changing texts:
  `slint-tr-extractor -j -o lang/de/LC_MESSAGES/breakbar.po ui/*.slint` (from `crates/bb-app`),
  then translate the new entries.
- **Previews without a window:** `cargo test -p breakbar -- --ignored render_ui_previews` renders
  the main states in both themes with the software renderer into `%TEMP%\breakbar-ui\*.bmp` —
  no window opens, so it is safe to run while playing.

## Workspace layout

```
crates/
  bb-win/    unsafe Win32/NT wrappers (RAII handles, mutex kill, CreateProcess, win events)
  bb-core/   domain: Account, Profile, LaunchPlan, CompanionApp, ProcessMonitor
  bb-store/  config.toml (serde), atomic writes
  bb-app/    binary `breakbar`: main.rs, Slint UI (ui/), CLI (`breakbar --launch "Main,Alt1"`)
    assets/  app.manifest (asInvoker, PerMonitorV2, longPathAware, UTF-8), resource script
```

Slint runs with the winit backend and the **software renderer** (no GPU context → lowest RAM,
smallest binary). Switch to femtovg/skia only if measurements show a need.

Config: `%APPDATA%\Breakbar\config.toml` (atomic write via temp file + `ReplaceFileW`).

## Roadmap

| Phase | Content |
|---|---|
| 1 | Repository, license, docs, CI |
| 2 | Cargo workspace skeleton, manifest, empty Slint window, CLI parsing |
| 3.1 | GW2 path selection + validation (file dialog, registry/Steam auto-detect) |
| 3.2 | Single launch + process monitoring |
| 3.3 | Multi-launch (mutex kill + `-shareArchive`), benchmarks with 2–5 clients |
| 3.4 | ✅ Per-account `Local.dat` via a junction swapped only during launch, serialized launches, one-time setup launch |
| 3.5 | ✅ Steam accounts: direct start with `-provider Steam` + `SteamAppId`, Steam install auto-selected, one Steam account at a time (not yet tested with a real Steam account) |
| 3.6 | ✅ Companion apps (per-client / shared, start on process or game window, graceful close), Blish HUD preset (not yet tested with a real Blish HUD) |
| 4.1 | ✅ Design: new UI in both themes (tokens, components, account rows with all states, selection, toasts, empty state, narrow layout), app icon, English + German |
| 4.1b | Account management: edit page, duplicate, delete, desktop shortcut, profile folder, drag & drop order, companion editor, first-start wizard |
| 4.2 | Settings page: paths |
| 4.3 | Start with Windows |
| 4.4 | Close behavior and tray |
| 4.5 | Instance switcher overlay |
| 4.6 | Login set-up flow |
| 5 | Patch detection + Local.dat refresh, window layout per account, priority/affinity, GFX per account |
| 6 | Code signing (SignPath/Azure Trusted Signing), releases, winget |
