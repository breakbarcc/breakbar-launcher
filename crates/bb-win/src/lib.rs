//! Thin, safe wrappers around the Win32/NT APIs used by Breakbar.
//!
//! All `unsafe` code of the project lives in this crate so that the rest of the
//! workspace can stay safe Rust.

pub mod autostart;
mod com;
pub mod console;
pub mod dialog;
pub mod junction;
pub mod mutex;
mod nt;
pub mod process;
pub mod registry;
pub mod shortcut;
pub mod time;
pub mod version;
pub mod window;
