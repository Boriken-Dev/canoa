//! What another kind keeps its secrets with: a handle on one entry of `[secrets.providers]`, by name, through whichever provider the
//! person chose to hold them. The accounts' credentials and the session services' passwords are kept this way, so a credential is a named
//! secret of the same providers an agent's secret comes from and the Secrets section lists it, instead of resting in a store of its own.
//!
//! A handle always asks the provider and never the cache: a credential that rotates is read as it is. Its words on failure name what
//! failed and never a value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the handle is the host's"
)]

use std::sync::Arc;

use super::vault::Vault;
use crate::accounts::store::{SecretStore, StoreError};

/// The prefix of the name a record of an account is kept under, so the Secrets section tells it from a secret an agent asked for.
pub(crate) const PREFIX: &str = "accounts/";

/// A record by its id, kept as the named secret `accounts/<id>` in the entry `entry` of `vault`.
#[derive(Clone)]
pub(crate) struct HandleStore {
    vault: Arc<Vault>,
    entry: String,
}

impl core::fmt::Debug for HandleStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandleStore")
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl HandleStore {
    /// The handle on `entry` of `vault`.
    pub(crate) fn new(vault: Arc<Vault>, entry: impl Into<String>) -> Self {
        Self {
            vault,
            entry: entry.into(),
        }
    }

    fn name(id: &str) -> String {
        format!("{PREFIX}{id}")
    }
}

fn refused(why: String) -> StoreError {
    StoreError(why)
}

impl SecretStore for HandleStore {
    fn describe(&self) -> &'static str {
        "a secret provider's store"
    }

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.vault
            .get_in(&self.entry, &Self::name(id))
            .map(|value| value.map(|text| text.as_bytes().to_vec()))
            .map_err(refused)
    }

    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let text =
            std::str::from_utf8(bytes).map_err(|_| StoreError("a record is not text".into()))?;
        self.vault
            .put_in(&self.entry, &Self::name(id), text)
            .map_err(refused)
    }

    fn remove(&self, id: &str) -> Result<(), StoreError> {
        self.vault
            .forget_in(&self.entry, &Self::name(id))
            .map_err(refused)
    }
}
