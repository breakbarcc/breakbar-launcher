//! Local wall-clock time.

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Current local time as `(hour, minute)`.
pub fn local_hour_minute() -> (u16, u16) {
    // SAFETY: no preconditions.
    let now = unsafe { GetLocalTime() };
    (now.wHour, now.wMinute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_and_minute_are_in_range() {
        let (hour, minute) = local_hour_minute();
        assert!(hour < 24 && minute < 60);
    }
}
