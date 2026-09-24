//! Named mutex queries.

use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND};
use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::core::{HSTRING, Result};

/// Name of the mutex Guild Wars 2 holds to enforce a single running instance.
pub const GW2_MUTEX_NAME: &str = "AN-Mutex-Window-Guild Wars 2";

/// Returns whether a named mutex exists in the current session namespace.
pub fn mutex_exists(name: &str) -> Result<bool> {
    let name = HSTRING::from(name);
    // SAFETY: `name` is a valid, NUL-terminated wide string that outlives the call.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) } {
        Ok(handle) => {
            // SAFETY: `handle` was just returned by OpenMutexW and is owned by us.
            unsafe { CloseHandle(handle)? };
            Ok(true)
        }
        Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(false),
        Err(error) => Err(error),
    }
}

/// Returns whether a Guild Wars 2 client currently holds its single-instance mutex.
pub fn gw2_mutex_exists() -> Result<bool> {
    mutex_exists(GW2_MUTEX_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::CreateMutexW;

    #[test]
    fn missing_mutex_is_reported_as_absent() {
        let name = format!("Breakbar-Test-Missing-{}", std::process::id());
        assert!(!mutex_exists(&name).unwrap());
    }

    #[test]
    fn existing_mutex_is_detected() {
        let name = format!("Breakbar-Test-Existing-{}", std::process::id());
        let wide = HSTRING::from(name.as_str());
        // SAFETY: `wide` is a valid wide string; the returned handle is closed below.
        let handle = unsafe { CreateMutexW(None, false, &wide) }.unwrap();
        assert!(mutex_exists(&name).unwrap());
        // SAFETY: `handle` is owned by this test.
        unsafe { CloseHandle(handle) }.unwrap();
        assert!(!mutex_exists(&name).unwrap());
    }
}
