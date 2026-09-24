//! Shared test-only helpers.

use std::fs;
use std::path::Path;

/// Runs `body` with `APPDATA` and `LOCALAPPDATA` pointed at fresh, disposable directories under
/// `%TEMP%`, so tests that touch profile paths never come near the real ones. `APPDATA`/
/// `LOCALAPPDATA` are process-global, so this is serialized crate-wide via `LOCK` — every test
/// module that needs isolation calls this same function rather than rolling its own mutex, or
/// parallel test threads could still race on the same two environment variables.
pub(crate) fn with_isolated_appdata(test_name: &str, body: impl FnOnce(&Path)) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap();

    let root = std::env::temp_dir().join(format!(
        "breakbar-test-appdata-{}-{test_name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Roaming")).unwrap();
    fs::create_dir_all(root.join("Local")).unwrap();

    let real_appdata = std::env::var_os("APPDATA");
    let real_localappdata = std::env::var_os("LOCALAPPDATA");
    // SAFETY: guarded by `LOCK` above; no other thread in this process reads/writes these two
    // variables while the guard is held.
    unsafe {
        std::env::set_var("APPDATA", root.join("Roaming"));
        std::env::set_var("LOCALAPPDATA", root.join("Local"));
    }

    body(&root);

    // SAFETY: see above.
    unsafe {
        match real_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        match real_localappdata {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
    }
    let _ = fs::remove_dir_all(&root);
}
