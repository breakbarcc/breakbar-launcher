//! Conversions between the settings in the config and the UI's enums, and applying them.

use super::{AfterStart, FpsLimit, LanguageChoice, ThemeChoice};

pub(super) fn to_ui_after_start(value: bb_store::AfterStart) -> AfterStart {
    match value {
        bb_store::AfterStart::KeepOpen => AfterStart::KeepOpen,
        bb_store::AfterStart::MinimizeToTray => AfterStart::MinimizeToTray,
        bb_store::AfterStart::Close => AfterStart::Close,
    }
}

/// Selects the UI language. Slint picks the system language by itself when the first window is
/// created; this makes the choice explicit (and lets "System" be chosen again later).
pub(super) fn apply_language(choice: bb_store::LanguageChoice) {
    let tag = match choice {
        bb_store::LanguageChoice::English => "en",
        bb_store::LanguageChoice::German => "de",
        bb_store::LanguageChoice::System => match sys_locale::get_locale() {
            Some(locale) if locale.to_ascii_lowercase().starts_with("de") => "de",
            _ => "en",
        },
    };
    // Can only fail for a language that is not bundled, and both of these are.
    let _ = slint::select_bundled_translation(tag);
}

pub(super) fn to_ui_language(value: bb_store::LanguageChoice) -> LanguageChoice {
    match value {
        bb_store::LanguageChoice::System => LanguageChoice::System,
        bb_store::LanguageChoice::English => LanguageChoice::English,
        bb_store::LanguageChoice::German => LanguageChoice::German,
    }
}

pub(super) fn from_ui_language(value: LanguageChoice) -> bb_store::LanguageChoice {
    match value {
        LanguageChoice::System => bb_store::LanguageChoice::System,
        LanguageChoice::English => bb_store::LanguageChoice::English,
        LanguageChoice::German => bb_store::LanguageChoice::German,
    }
}

pub(super) fn to_ui_theme(value: bb_store::ThemeChoice) -> ThemeChoice {
    match value {
        bb_store::ThemeChoice::System => ThemeChoice::System,
        bb_store::ThemeChoice::Light => ThemeChoice::Light,
        bb_store::ThemeChoice::Dark => ThemeChoice::Dark,
    }
}

pub(super) fn from_ui_theme(value: ThemeChoice) -> bb_store::ThemeChoice {
    match value {
        ThemeChoice::System => bb_store::ThemeChoice::System,
        ThemeChoice::Light => bb_store::ThemeChoice::Light,
        ThemeChoice::Dark => bb_store::ThemeChoice::Dark,
    }
}

pub(super) fn to_ui_fps_limit(value: bb_store::FpsLimit) -> FpsLimit {
    match value {
        bb_store::FpsLimit::Fps60 => FpsLimit::Fps60,
        bb_store::FpsLimit::Fps30 => FpsLimit::Fps30,
        bb_store::FpsLimit::Unlimited => FpsLimit::Unlimited,
    }
}

pub(super) fn from_ui_fps_limit(value: FpsLimit) -> bb_store::FpsLimit {
    match value {
        FpsLimit::Fps60 => bb_store::FpsLimit::Fps60,
        FpsLimit::Fps30 => bb_store::FpsLimit::Fps30,
        FpsLimit::Unlimited => bb_store::FpsLimit::Unlimited,
    }
}

pub(super) fn from_ui_after_start(value: AfterStart) -> bb_store::AfterStart {
    match value {
        AfterStart::KeepOpen => bb_store::AfterStart::KeepOpen,
        AfterStart::MinimizeToTray => bb_store::AfterStart::MinimizeToTray,
        AfterStart::Close => bb_store::AfterStart::Close,
    }
}
