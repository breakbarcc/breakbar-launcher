//! COM initialization for the calling thread.

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::core::Result;

/// Initializes a single-threaded COM apartment for the current thread for the guard's lifetime.
pub(crate) struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    pub(crate) fn enter() -> Result<Self> {
        // SAFETY: balanced by CoUninitialize in Drop when initialization succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr == RPC_E_CHANGED_MODE {
            // The thread already runs a multithreaded apartment; the dialog still works there.
            return Ok(Self { initialized: false });
        }
        hr.ok()?;
        Ok(Self { initialized: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: matches the successful CoInitializeEx in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}
