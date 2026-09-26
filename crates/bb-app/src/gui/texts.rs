//! Translated texts for starts without a window.

use super::{ComponentHandle, LaunchError, MainWindow, Messages, launch_failure};

/// Translated texts for starts without a window (desktop shortcuts, command line). Uses a window
/// instance that is never shown, because that is where Slint keeps the translated texts.
pub struct Texts(MainWindow);

impl std::fmt::Debug for Texts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Texts")
    }
}

impl Texts {
    pub fn new() -> Result<Self, slint::PlatformError> {
        MainWindow::new().map(Self)
    }

    pub fn start_failed(&self) -> String {
        self.0.global::<Messages>().invoke_start_failed().into()
    }

    pub fn launch_failure(&self, error: &LaunchError, name: &str) -> String {
        let (kind, detail) = launch_failure(error);
        self.0
            .global::<Messages>()
            .invoke_launch_failure(kind, name.into(), detail)
            .into()
    }

    pub fn companion_failed(&self, app: &str, error: &std::io::Error) -> String {
        let title = self
            .0
            .global::<Messages>()
            .invoke_companion_failed_title(app.into());
        format!("{title}: {error}")
    }

    pub fn unknown_account(&self, name: &str) -> String {
        self.0
            .global::<Messages>()
            .invoke_unknown_account(name.into())
            .into()
    }

    pub fn settings_load_failed(&self, detail: &str) -> String {
        self.0
            .global::<Messages>()
            .invoke_settings_load_failed(detail.into())
            .into()
    }
}
