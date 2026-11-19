//! The spawner a secret provider is handed: this machine's, and a program the Store installed found by its bare name, so a provider
//! that runs `bw` runs the copy the Store put there when the person has none of their own on the path.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    FileInfo, HarnessNotice, ProcessSpawner, SecretBytes, SpawnSpec, SpawnedProcess, SpawnerInfo,
};
use coqui_acp_harness::LocalSpawner;

use crate::store::runtimes::RUNTIME_PROGRAMS;

/// The program a bare `program` stands for: the copy the Store installed when it names one, else as it was written. A name with a
/// folder in it is the person's or the provider's choice and is never replaced.
fn resolved(program: &str) -> String {
    let bare = !program.contains(['/', '\\']);
    let stem = program.strip_suffix(".exe").unwrap_or(program);
    if bare
        && RUNTIME_PROGRAMS.contains(&stem)
        && let Some((path, _)) = crate::sources::managed_program(stem)
    {
        return path.to_string_lossy().into_owned();
    }
    program.to_owned()
}

struct Managed(LocalSpawner);

impl ProcessSpawner for Managed {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        self.0.describe()
    }

    fn spawn(&self, mut spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        spec.program = resolved(&spec.program);
        self.0.spawn(spec)
    }

    fn write_file(
        &self,
        path: &str,
        bytes: Secret<SecretBytes>,
        private: bool,
    ) -> Result<(), ProviderError> {
        self.0.write_file(path, bytes, private)
    }

    fn read_file(&self, path: &str, max_bytes: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        self.0.read_file(path, max_bytes)
    }

    fn file_info(&self, path: &str) -> Result<Option<FileInfo>, ProviderError> {
        self.0.file_info(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), ProviderError> {
        self.0.remove_file(path)
    }

    fn notice(&self, notice: HarnessNotice) -> Result<(), ProviderError> {
        self.0.notice(notice)
    }
}

/// A spawner for one call of a provider, which owns and destroys it.
pub(crate) fn spawner() -> Object<dyn ProcessSpawner> {
    Managed(LocalSpawner::new()).into_object()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_with_a_folder_or_one_the_store_does_not_manage_is_left_as_written() {
        for program in [r"C:\tools\bw.exe", "./bw", "git", "not-ours"] {
            // No copy is installed in a test's shelf for any of these, and a path is never replaced.
            assert_eq!(resolved(program), program, "{program}");
        }
    }
}
