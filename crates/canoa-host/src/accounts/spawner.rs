//! The spawner a provider is handed for an account: this machine's, with its file methods confined to the account's
//! directory, and a record of the files the provider placed there so the host can take them away when the last
//! connection on the account ends.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a set shared between threads is std's"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    FileInfo, ProcessSpawner, SecretBytes, SpawnSpec, SpawnedProcess, SpawnerInfo,
};
use coqui_acp_harness::LocalSpawner;

/// This machine's spawner with files served in `dir`.
pub(super) fn local(dir: &str) -> LocalSpawner {
    LocalSpawner::new().files(dir)
}

/// [`LocalSpawner`], remembering every path `write_file` was given.
pub(super) struct Recording {
    inner: LocalSpawner,
    written: Arc<Mutex<BTreeSet<String>>>,
}

impl Recording {
    /// A spawner serving files in `dir`, when the account has one; without it the file methods are refused as this
    /// machine's spawner refuses them.
    pub(super) fn new(dir: Option<&str>, written: Arc<Mutex<BTreeSet<String>>>) -> Self {
        Self {
            inner: dir.map_or_else(LocalSpawner::new, local),
            written,
        }
    }
}

impl ProcessSpawner for Recording {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        self.inner.describe()
    }

    fn spawn(&self, spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        self.inner.spawn(spec)
    }

    fn write_file(
        &self,
        path: &str,
        bytes: Secret<SecretBytes>,
        private: bool,
    ) -> Result<(), ProviderError> {
        self.inner.write_file(path, bytes, private)?;
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned());
        Ok(())
    }

    fn read_file(&self, path: &str, max_bytes: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        self.inner.read_file(path, max_bytes)
    }

    fn file_info(&self, path: &str) -> Result<Option<FileInfo>, ProviderError> {
        self.inner.file_info(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), ProviderError> {
        self.inner.remove_file(path)
    }
}
