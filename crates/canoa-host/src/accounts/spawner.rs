//! The spawner a provider is handed for an account: this machine's, with its file methods confined to the account's
//! directory, and a record of the files the provider placed there so the host can take them away when the last
//! connection on the account ends.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a set shared between threads is std's"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    FileInfo, HarnessNotice, ProcessSpawner, SecretBytes, SpawnSpec, SpawnedProcess, SpawnerInfo,
};
use coqui_acp_harness::LocalSpawner;

/// This machine's spawner with files served in `dir`.
pub(super) fn local(dir: &str) -> LocalSpawner {
    LocalSpawner::new().files(dir)
}

/// Removes the files in `placed` from the directory `state`, and the note of them beside it.
pub(super) fn remove_these(state: &Path, placed: &BTreeSet<String>) {
    let files = local(&state.to_string_lossy());
    for path in placed {
        let _ = files.remove_file(path);
    }
    if let Some(own) = state.parent() {
        let _ = fs::remove_file(own.join("placed"));
    }
}

/// Removes what the note `note` says was placed in the `state` directory of the account directory `own`: what a crash left.
pub(super) fn remove_placed(own: &Path, note: &Path) {
    let placed: BTreeSet<String> = fs::read_to_string(note)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    remove_these(&own.join("state"), &placed);
    let _ = fs::remove_file(note);
}

/// [`LocalSpawner`], remembering every path `write_file` was given, in memory and in a note beside the account's
/// directory, so that a crash does not leave a credential file nobody knows to remove.
pub(super) struct Recording {
    inner: LocalSpawner,
    written: Arc<Mutex<BTreeSet<String>>>,
    note: Option<PathBuf>,
}

impl Recording {
    /// A spawner serving files in `dir`, when the account has one; without it the file methods are refused as this
    /// machine's spawner refuses them.
    pub(super) fn new(
        dir: Option<&str>,
        written: Arc<Mutex<BTreeSet<String>>>,
        notices: impl Fn(HarnessNotice) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: dir.map_or_else(LocalSpawner::new, local).notices(notices),
            written,
            note: dir.and_then(|dir| Path::new(dir).parent().map(|own| own.join("placed"))),
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
        // Noted before it is written, so that a crash between the two leaves a note and no file, not the other way.
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if !path.contains('\n')
            && written.insert(path.to_owned())
            && let Some(note) = &self.note
        {
            let lines: Vec<&str> = written.iter().map(String::as_str).collect();
            let _ = super::private::write_file(note, lines.join("\n").as_bytes());
        }
        drop(written);
        self.inner.write_file(path, bytes, private)
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

    fn notice(&self, notice: HarnessNotice) -> Result<(), ProviderError> {
        self.inner.notice(notice)
    }
}
