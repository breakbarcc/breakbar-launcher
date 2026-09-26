//! Game accounts.

use serde::{Deserialize, Serialize};

use crate::companion::CompanionId;

/// Stable identifier of an account. Also used to name its profile directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub u32);

/// How the account authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Regular `ArenaNet` account, logged in via its own `Local.dat`.
    #[default]
    ArenaNet,
    /// Steam-linked account, authenticated by the running Steam client.
    Steam,
}

impl Provider {
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::ArenaNet => "ArenaNet",
            Provider::Steam => "Steam",
        }
    }
}

/// A Guild Wars 2 account managed by Breakbar.
///
/// Credentials are never stored here: the login lives exclusively in the account's `Local.dat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    #[serde(default)]
    pub provider: Provider,
    /// Additional command line arguments, appended verbatim.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub extra_args: String,
    /// Companion apps started together with this account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub companions: Vec<CompanionId>,
}

impl Account {
    pub fn new(id: AccountId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            provider: Provider::default(),
            extra_args: String::new(),
            companions: Vec::new(),
        }
    }

    /// Unique `MumbleLink` shared memory name, so overlays never read another client's data.
    #[must_use]
    pub fn mumble_link_name(&self) -> String {
        format!("Breakbar_{}", self.id.0)
    }
}
