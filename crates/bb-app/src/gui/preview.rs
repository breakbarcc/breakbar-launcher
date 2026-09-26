//! Renders the UI into image files without opening a window, to check the design:
//! `cargo test -p breakbar-launcher -- --ignored render_ui_previews`, output in `%TEMP%\breakbar-ui`.

use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{PhysicalSize, Rgb8Pixel};
use ui::Page::{Accounts, Editor, Settings, SetupAccount, SetupPath};

struct Offscreen(Rc<MinimalSoftwareWindow>);

/// One preview image.
struct Variant {
    name: &'static str,
    dark: bool,
    demo_rows: bool,
    page: ui::Page,
    size: (u32, u32),
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Writes a 24-bit BMP (bottom-up rows, padded to 4 bytes).
fn write_bmp(path: &Path, width: usize, height: usize, pixels: &[Rgb8Pixel]) {
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..height).rev() {
        for pixel in &pixels[y * width..(y + 1) * width] {
            out.extend_from_slice(&[pixel.b, pixel.g, pixel.r]);
        }
        out.resize(out.len() + row - width * 3, 0);
    }
    std::fs::write(path, out).unwrap();
}

#[test]
#[ignore = "writes image files for looking at the design"]
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn render_ui_previews() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    let dir = std::env::temp_dir().join("breakbar-ui");
    std::fs::create_dir_all(&dir).unwrap();
    // For the README screenshots: `BREAKBAR_PREVIEW_LANG=en` renders the source language, and
    // `BREAKBAR_PREVIEW_SCALE=2` renders at twice the resolution, and
    // `BREAKBAR_PREVIEW_NO_TOAST=1` leaves out the demo toast.
    let language = std::env::var("BREAKBAR_PREVIEW_LANG").ok();
    let scale: f32 = std::env::var("BREAKBAR_PREVIEW_SCALE")
        .ok()
        .and_then(|scale| scale.parse().ok())
        .unwrap_or(1.0);
    let physical = |logical: u32| (logical as f32 * scale).round() as u32;
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });

    let variant = |name, dark, demo_rows, page, size| Variant {
        name,
        dark,
        demo_rows,
        page,
        size,
    };
    let variants = [
        variant("accounts-dark", true, true, Accounts, (420, 600)),
        variant("accounts-light", false, true, Accounts, (420, 600)),
        variant("empty-dark", true, false, Accounts, (420, 520)),
        variant("empty-light", false, false, Accounts, (420, 520)),
        variant("editor-dark", true, false, Editor, (420, 780)),
        variant("editor-light", false, false, Editor, (420, 780)),
        variant("setup-path-dark", true, false, SetupPath, (420, 520)),
        variant(
            "setup-account-light",
            false,
            false,
            SetupAccount,
            (420, 520),
        ),
        variant("narrow-dark", true, true, Accounts, (320, 360)),
        variant("settings-dark", true, false, Settings, (420, 1400)),
        variant("settings-light", false, false, Settings, (420, 1400)),
        variant("settings-top-light", false, false, Settings, (420, 600)),
        variant("settings-top-dark", true, false, Settings, (420, 600)),
        variant("editor-steam-dark", true, false, Editor, (420, 780)),
        variant("login-refresh-dark", true, true, Accounts, (420, 600)),
        variant("login-refresh-light", false, true, Accounts, (420, 600)),
        variant("login-offer-dark", true, true, Accounts, (420, 520)),
        variant("login-offer-steam-light", false, true, Accounts, (420, 520)),
        variant("login-banner-dark", true, true, Accounts, (420, 520)),
        variant(
            "login-banner-steam-light",
            false,
            true,
            Accounts,
            (420, 520),
        ),
        variant("steam-link-dark", true, true, Accounts, (420, 520)),
        variant("steam-install-light", false, true, Accounts, (420, 520)),
    ];
    for Variant {
        name,
        dark,
        demo_rows: demo,
        page,
        size: (width, height),
    } in variants
    {
        window.set_size(PhysicalSize::new(physical(width), physical(height)));
        let ui = MainWindow::new().unwrap();
        if let Some(language) = &language {
            slint::select_bundled_translation(language).unwrap();
        }
        ui.global::<Theme>().set_choice(if dark {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        });
        ui.set_gw2_path(r"C:\Program Files\Guild Wars 2\Gw2-64.exe".into());
        ui.set_gw2_path_ok(true);
        ui.set_blish_path(r"D:\Tools\Blish HUD\Blish HUD.exe".into());
        ui.set_blish_path_ok(name != "settings-dark");
        ui.set_autostart(name == "settings-dark");
        ui.set_after_start(AfterStart::MinimizeToTray);
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        if name.starts_with("login-refresh") {
            ui.set_refresh_names("Main, 2nd, Steam Acc".into());
        }
        ui.set_overlay_enabled(true);
        ui.set_overlay_only_running(true);
        ui.set_overlay_locked(false);
        ui.set_overlay_opacity(58);
        ui.set_setup_detected_path(r"C:\Program Files\Guild Wars 2\Gw2-64.exe".into());
        if demo {
            // The failed row carries its reason as text, which must be in the preview language too.
            let mut rows = demo_rows();
            for row in &mut rows {
                if row.state == AccountState::Error {
                    row.detail = ui.global::<Messages>().invoke_start_failed();
                }
            }
            set_rows(&ui, rows);
            if std::env::var_os("BREAKBAR_PREVIEW_NO_TOAST").is_none() {
                let messages = ui.global::<Messages>();
                push_toast(
                    &ui,
                    ToastKind::Warning,
                    messages.invoke_steam_busy_title(),
                    messages.invoke_steam_busy("Steam Zweit".into(), "Steam".into()),
                );
            }
        }
        refresh(&ui);
        ui.set_editor(EditorData {
            id: 1,
            name: "Main".into(),
            steam: name == "editor-steam-dark",
            args: "-windowed -mapLoadinfo".into(),
            login: if name == "editor-steam-dark" {
                LoginState::Steam
            } else {
                LoginState::SetUp
            },
            active: false,
            companions: Rc::new(slint::VecModel::from(vec![CompanionToggle {
                id: 1,
                name: "Blish HUD".into(),
                per_client: true,
                after_game_start: true,
                enabled: true,
            }]))
            .into(),
        });
        ui.set_page(page);
        if name.starts_with("login-offer") {
            ui.set_login_offer_id(1);
            ui.set_login_offer_name("Main".into());
            ui.set_login_offer_steam(name.contains("steam"));
        }
        if name.starts_with("login-banner") {
            ui.set_login_setup_name("Main".into());
            ui.set_login_setup_steam(name.contains("steam"));
        }
        if name.starts_with("steam-") {
            ui.set_steam_setup_account("Steam Acc".into());
            ui.set_steam_setup_link(
                r"C:\Program Files (x86)\Steam\steamapps\common\Guild Wars 2".into(),
            );
            ui.set_steam_setup_target(r"C:\Games\Guild Wars\Guild Wars 2".into());
            ui.set_steam_setup_retry(name == "steam-install-light");
            ui.set_steam_setup(if name == "steam-link-dark" {
                SteamSetupStep::Link
            } else {
                SteamSetupStep::Install
            });
        }
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();

        let (w, h) = (physical(width) as usize, physical(height) as usize);
        let mut pixels = vec![Rgb8Pixel::default(); w * h];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w);
        });
        write_bmp(&dir.join(format!("{name}.bmp")), w, h, &pixels);
        ui.hide().unwrap();
    }

    // The overlay is a separate top-level component (its own window), so it's rendered the
    // same way but outside the `MainWindow` loop above.
    for (name, dark) in [("overlay-dark", true), ("overlay-light", false)] {
        let (width, height) = (260, 32);
        window.set_size(PhysicalSize::new(physical(width), physical(height)));
        let overlay = ui::OverlaySwitcher::new().unwrap();
        if let Some(language) = &language {
            slint::select_bundled_translation(language).unwrap();
        }
        overlay.global::<Theme>().set_choice(if dark {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        });
        // Shown as under the pointer; at rest it is dimmed (58 % by default).
        overlay.set_idle_opacity(1.0);
        overlay.set_entries(
            Rc::new(slint::VecModel::from(vec![
                ui::SwitcherEntry {
                    id: 1,
                    name: "Main".into(),
                    running: true,
                    active: true,
                    starting: false,
                },
                ui::SwitcherEntry {
                    id: 2,
                    name: "Raid Chrono".into(),
                    running: true,
                    active: false,
                    starting: false,
                },
                ui::SwitcherEntry {
                    id: 3,
                    name: "Farm Alt".into(),
                    running: true,
                    active: false,
                    starting: true,
                },
                ui::SwitcherEntry {
                    id: 4,
                    name: "Steam".into(),
                    running: false,
                    active: false,
                    starting: false,
                },
            ]))
            .into(),
        );
        overlay.show().unwrap();
        slint::platform::update_timers_and_animations();

        let (w, h) = (physical(width) as usize, physical(height) as usize);
        let mut pixels = vec![Rgb8Pixel::default(); w * h];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w);
        });
        write_bmp(&dir.join(format!("{name}.bmp")), w, h, &pixels);
        overlay.hide().unwrap();
    }
}

/// Rows in every state, for the UI previews.
fn demo_rows() -> Vec<AccountRow> {
    let row = |id: i32, name: &str, steam: bool, state: AccountState| AccountRow {
        id,
        name: name.into(),
        provider: if steam { "Steam" } else { "ArenaNet" }.into(),
        steam,
        state,
        ..AccountRow::default()
    };
    vec![
        AccountRow {
            has_companions: true,
            ..row(101, "Main", false, AccountState::Idle)
        },
        AccountRow {
            has_companions: true,
            ..row(102, "Raid Chrono", false, AccountState::Starting)
        },
        AccountRow {
            has_companions: true,
            addon_active: true,
            since: "14:02".into(),
            handle: 1,
            ..row(103, "Farm Alt", false, AccountState::Running)
        },
        row(104, "Steam", true, AccountState::NeedsLogin),
        AccountRow {
            selected: true,
            ..row(105, "WvW Guard", false, AccountState::Idle)
        },
        AccountRow {
            detail: "Start failed".into(),
            ..row(106, "Crafting", false, AccountState::Error)
        },
        row(107, "Steam Zweit", true, AccountState::Locked),
    ]
}
