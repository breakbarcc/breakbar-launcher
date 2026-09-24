# Breakbar Launcher

A fast, lightweight multi-launcher for **Guild Wars 2** on Windows.

- Launch multiple GW2 clients side by side
- Per-account login via separate `Local.dat` files – no passwords stored anywhere else
- ArenaNet and Steam accounts
- Companion apps per account (Blish HUD first), started and stopped together with the game
- Native Rust binary, event-driven, near-zero idle CPU

> **Status:** early development – see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## How multi-launching works

GW2 holds a named mutex (`AN-Mutex-Window-Guild Wars 2`) to prevent a second instance and locks
`Gw2.dat`. Breakbar closes that mutex handle inside the already running client and starts new
clients with `-shareArchive`, which opens `Gw2.dat` read-only. The game executable is never modified.

## Building

Requirements: Windows 10/11 x64, [Rust](https://rustup.rs) (MSVC toolchain), Visual Studio Build Tools (C++ workload).

```bash
cargo build --release
```

## License

[MIT](LICENSE)

Guild Wars 2 is a trademark of ArenaNet, LLC. This project is not affiliated with ArenaNet or NCSOFT.
