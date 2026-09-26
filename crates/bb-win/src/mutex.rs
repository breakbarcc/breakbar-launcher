//! Named mutex queries.

use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateMutexW, INFINITE, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::core::{HSTRING, Result};

/// Name of the mutex Guild Wars 2 holds to enforce a single running instance.
pub const GW2_MUTEX_NAME: &str = "AN-Mutex-Window-Guild Wars 2";

/// Returns whether a named mutex exists in the current session namespace.
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
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
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn gw2_mutex_exists() -> Result<bool> {
    mutex_exists(GW2_MUTEX_NAME)
}

/// Closes the single-instance mutex handle held by the Guild Wars 2 process `pid`, so a second
/// client can start. `pid` must be a client Breakbar itself launched (see [`crate::nt`] for why
/// and how).
///
/// Returns `Ok(true)` if the mutex was found and closed, `Ok(false)` if `pid` didn't have one
/// open (nothing to do — for example, it hasn't created it yet, or already lost it).
///
/// # Errors
///
/// Returns the Windows error if the underlying call fails.
pub fn kill_gw2_mutex(pid: u32) -> Result<bool> {
    crate::nt::close_named_mutex_in_process(pid, GW2_MUTEX_NAME)
}

/// Ownership of a named mutex, released on drop. Tied to the thread that acquired it (a mutex
/// can only be released by its owning thread), so it is neither `Send` nor `Sync`.
#[derive(Debug)]
pub struct OwnedMutex(HANDLE);

impl OwnedMutex {
    /// Creates or opens the named mutex and waits until this thread owns it. A mutex left behind
    /// by a process that died while holding it is taken over.
    ///
    /// # Errors
    ///
    /// Returns the Windows error if the underlying call fails.
    pub fn acquire(name: &str) -> Result<Self> {
        let name = HSTRING::from(name);
        // SAFETY: `name` is a valid wide string for the call; the handle is owned by `Self`.
        let handle = unsafe { CreateMutexW(None, false, &name) }?;
        // SAFETY: `handle` is a valid mutex handle.
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Ok(Self(handle))
        } else {
            let error = windows::core::Error::from_thread();
            // SAFETY: `handle` is owned here and not used afterwards.
            let _ = unsafe { CloseHandle(handle) };
            Err(error)
        }
    }
}

impl Drop for OwnedMutex {
    fn drop(&mut self) {
        // SAFETY: this thread owns the mutex (see `acquire`) and the handle.
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_mutex_is_reported_as_absent() {
        let name = format!("Breakbar-Test-Missing-{}", std::process::id());
        assert!(!mutex_exists(&name).unwrap());
    }

    #[test]
    fn owned_mutex_is_exclusive_and_released_on_drop() {
        let name = format!("Breakbar-Test-Owned-{}", std::process::id());
        let first = OwnedMutex::acquire(&name).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn({
            let name = name.clone();
            move || {
                let _second = OwnedMutex::acquire(&name).unwrap();
                sender.send(()).unwrap();
            }
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
        drop(first);
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        waiter.join().unwrap();
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
