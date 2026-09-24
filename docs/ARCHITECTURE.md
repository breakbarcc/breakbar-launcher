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

Every account owns a profile directory:

```
%LOCALAPPDATA%\Breakbar\profiles\<account-id>\
    Guild Wars 2\Local.dat
    Guild Wars 2\GFXSettings.Gw2-64.exe.xml   (later)
```

**First launch of an account:** the client starts with an empty profile, the user logs in once with
"remember email/password", GW2 writes `Local.dat` into the profile. All later launches use `-autologin`.

How the profile reaches the client is decided by **Spike S1**:

- **A (preferred):** redirect `APPDATA` (and `TMP`) in the child's environment block – no race, no admin,
  all accounts can launch concurrently.
- **B (fallback):** symlink `%APPDATA%\Guild Wars 2\Local.dat` → profile file before each launch
  (needs Developer Mode/admin, launches must be serialized until the client has opened the file).

**After a game patch** `Local.dat` files of an older build must be refreshed (one normal launch per
account). Breakbar detects the build mismatch and guides the user instead of failing.

## Steam accounts

- Start `Gw2-64.exe` directly with `-provider Steam` and env `SteamAppId=1284210` (Steam client running).
- **To verify (Spike S2):** Steam authenticates the account logged into the Steam client, so presumably
  only **one Steam-linked account** can run at a time (plus any number of ArenaNet accounts).

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

- **PerClient** (Blish HUD): one instance per GW2 client, terminated when that client exits.
  Preset args: `--pid {pid} --mumble {mumble}`.
- **Shared**: one instance for all clients, reference-counted, terminated when the last client exits.
- Each GW2 client gets a unique MumbleLink name (`Breakbar_<id>`) so overlays never read another client's data.

## Process monitoring

- All child handles are waited on by thread-pool waits (`RegisterWaitForSingleObject`) → 0 % CPU when idle.
- Window appearance per PID via `SetWinEventHook(EVENT_OBJECT_SHOW)` – no polling.
- Error dialogs (`ArenaNet_Dialog_Class`, e.g. "needs to be patched before using -shareArchive") are
  detected and surfaced inline.

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
| 3.4 | Spike S1 → account management with per-account profile/Local.dat |
| 3.5 | Steam accounts (Spike S2) |
| 3.6 | Companion apps, Blish HUD preset |
| 4 | UX: status per account, launch-all queue, tray, hotkeys, dark/light, shortcuts/CLI |
| 5 | Patch detection + Local.dat refresh, window layout per account, priority/affinity, GFX per account |
| 6 | Code signing (SignPath/Azure Trusted Signing), releases, winget |
