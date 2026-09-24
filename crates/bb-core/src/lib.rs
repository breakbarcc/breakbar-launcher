//! Breakbar domain model.
//!
//! This crate is platform independent and free of I/O so it can be unit tested in isolation.

pub mod account;
pub mod companion;
pub mod launch;

pub use account::{Account, AccountId, Provider};
pub use companion::{ArgContext, CompanionApp, CompanionId, Scope, Trigger};
pub use launch::{LaunchOptions, game_command_line};
