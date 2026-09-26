//! Local wall-clock time.

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Current local time as `(hour, minute)`.
#[must_use]
pub fn local_hour_minute() -> (u16, u16) {
    // SAFETY: no preconditions.
    let now = unsafe { GetLocalTime() };
    (now.wHour, now.wMinute)
}

/// Current local date and time as `YYYY-MM-DD HH:MM:SS`.
#[must_use]
pub fn local_timestamp() -> String {
    // SAFETY: no preconditions.
    let now = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_has_the_documented_shape() {
        let stamp = local_timestamp();
        assert_eq!(stamp.len(), 19);
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[13..14], ":");
    }

    #[test]
    fn hour_and_minute_are_in_range() {
        let (hour, minute) = local_hour_minute();
        assert!(hour < 24 && minute < 60);
    }
}
