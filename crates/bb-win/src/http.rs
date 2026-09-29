//! One blocking HTTPS GET, via `WinHTTP`.
//!
//! Just enough for a single small request against a known host: no connection reuse, no request
//! body, no redirect handling beyond what `WinHTTP` does on its own. Not a general HTTP client.

use std::ffi::c_void;
use std::io;
use std::time::Duration;

use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR};

/// Identifies Breakbar to the server, the way a browser's `User-Agent` would.
const USER_AGENT: &str = concat!("breakbar-launcher/", env!("CARGO_PKG_VERSION"));

/// A `WinHTTP` handle (session, connection or request), closed once when it goes out of scope so an
/// early return on error can't leak it.
struct Handle(*mut c_void);

impl Handle {
    fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was returned by one of the WinHttp*Open/Connect/OpenRequest calls
            // below and is closed at most once, here, regardless of which call produced it.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Performs one blocking HTTPS GET to `https://{host}{path}` and returns the response body as
/// text. `timeout` bounds every individual stage (resolve, connect, send, receive), so a stalled
/// connection can't hold up the caller much longer than it.
///
/// # Errors
///
/// Returns the OS error if the request can't be made or the response isn't valid UTF-8 text.
pub fn get(host: &str, path: &str, timeout: Duration) -> Result<String, io::Error> {
    // SAFETY: all string arguments are valid wide strings for the duration of the call; the
    // returned handle is wrapped in `Handle` right away so every exit path below closes it.
    let session = Handle(unsafe {
        WinHttpOpen(
            &HSTRING::from(USER_AGENT),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    });
    if session.is_null() {
        return Err(io::Error::last_os_error());
    }
    set_timeouts(&session, timeout)?;

    // SAFETY: `session.0` is a valid, still-open session handle.
    let connect = Handle(unsafe {
        WinHttpConnect(
            session.0,
            &HSTRING::from(host),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        )
    });
    if connect.is_null() {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `connect.0` is a valid, still-open connection handle; a null accept-types list means
    // "none", which WinHTTP accepts.
    let request = Handle(unsafe {
        WinHttpOpenRequest(
            connect.0,
            &HSTRING::from("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    });
    if request.is_null() {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `request.0` is a valid, still-open request handle; no request body or extra headers
    // are sent.
    unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }.map_err(io::Error::other)?;
    // SAFETY: `request.0` has an outstanding send from the call just above.
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.map_err(io::Error::other)?;

    read_body(&request)
}

fn set_timeouts(handle: &Handle, timeout: Duration) -> Result<(), io::Error> {
    let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `handle.0` is a valid, still-open handle.
    unsafe { WinHttpSetTimeouts(handle.0, millis, millis, millis, millis) }
        .map_err(io::Error::other)
}

fn read_body(request: &Handle) -> Result<String, io::Error> {
    let mut body = Vec::new();
    loop {
        let mut available = 0u32;
        // SAFETY: `request.0` is a valid, still-open request handle that has received a response;
        // `available` receives the number of bytes ready to read.
        unsafe { WinHttpQueryDataAvailable(request.0, &raw mut available) }
            .map_err(io::Error::other)?;
        if available == 0 {
            break;
        }

        let mut chunk = vec![0u8; available as usize];
        let mut read = 0u32;
        // SAFETY: `chunk` holds at least `available` bytes, the length passed as the read size.
        unsafe {
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                available,
                &raw mut read,
            )
        }
        .map_err(io::Error::other)?;
        chunk.truncate(read as usize);
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetches_a_known_page() {
        let body =
            get("example.com", "/", Duration::from_secs(10)).expect("request should succeed");
        assert!(body.contains("Example Domain"));
    }

    #[test]
    fn a_missing_host_is_an_error() {
        let result = get(
            "this-host-does-not-exist.invalid",
            "/",
            Duration::from_secs(10),
        );
        assert!(result.is_err());
    }
}
