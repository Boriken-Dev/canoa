//! The `catalog_source` providers this host offers: the two that ship with it (`read` and `command`, scripts written with the Canoa
//! SDK) and those the extensions of canoa.cfg add, all found the same way, through a Guatiao registry.
//!
//! `read` names a place the Store type reads itself (a git source, an address or a file), so the host's own reader for the type does
//! the reading and the provider only describes the form. `command` runs a program the person named and takes what it prints. Both
//! run through the spawner each `read` is handed: the person's own reach, with no other way to start a program or open a file.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and processes are std's"
)]

use std::sync::{Arc, Mutex, PoisonError};

use canoa::store::source::{CatalogSourceProvider, KIND, ReadRequest, SourceInfo};
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Secret, Status};
use coqui_acp_api::{
    FileInfo, HarnessNotice, ProcessSpawner, SecretBytes, SpawnSpec, SpawnedProcess, SpawnerInfo,
};
use coqui_acp_harness::LocalSpawner;

use crate::scripts::Bundle;

/// The sources that ship in the host, with the SDK modules they import.
static BUNDLE: Bundle = Bundle {
    name: "catalog-sources",
    libraries: &["catalog_sources"],
    files: &[
        (
            "catalog_sources/sources.js",
            include_str!("../../extensions/catalog_sources/sources.js"),
        ),
        (
            "catalog_sources/package.json",
            include_str!("../../extensions/catalog_sources/package.json"),
        ),
        (
            "catalog_sources/canoa_catalog_sources.guatiao",
            include_str!("../../extensions/catalog_sources/canoa_catalog_sources.guatiao"),
        ),
        (
            "catalog_sources/sdk/source.js",
            include_str!("../../../../bindings/js/canoa-sdk/src/source.js"),
        ),
        (
            "catalog_sources/sdk/kinds.js",
            include_str!("../../../../bindings/js/canoa-sdk/src/kinds.js"),
        ),
        (
            "catalog_sources/sdk/model.js",
            include_str!("../../../../bindings/js/canoa-sdk/src/model.js"),
        ),
    ],
};

/// The spawner a source's `read` is handed: this machine's, with the person's own reach, so a source runs a program as the person
/// and reads a file anywhere their account can. It writes and removes nothing.
struct Reach(LocalSpawner);

fn refused(status: Status, why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(status, why.as_ref())
}

impl ProcessSpawner for Reach {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        self.0.describe()
    }

    fn spawn(&self, spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        self.0.spawn(spec)
    }

    fn write_file(
        &self,
        _path: &str,
        _bytes: Secret<SecretBytes>,
        _private: bool,
    ) -> Result<(), ProviderError> {
        Err(refused(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a catalog source keeps no file",
        ))
    }

    fn read_file(&self, path: &str, max_bytes: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        let path = std::path::Path::new(path);
        if !path.is_absolute() {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                "a file is read by its absolute path",
            ));
        }
        let meta = std::fs::metadata(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => refused(Status::GUATIAO_ERR_NOT_FOUND, "no such file"),
            _ => refused(Status::GUATIAO_ERR_INTERNAL, error.to_string()),
        })?;
        if !meta.is_file() {
            return Err(refused(Status::GUATIAO_ERR_NOT_FOUND, "not a file"));
        }
        if meta.len() > max_bytes {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                format!("the file is longer than {max_bytes} bytes"),
            ));
        }
        std::fs::read(path)
            .map(|bytes| Secret::new(SecretBytes::new(bytes)))
            .map_err(|error| refused(Status::GUATIAO_ERR_INTERNAL, error.to_string()))
    }

    fn file_info(&self, path: &str) -> Result<Option<FileInfo>, ProviderError> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => {
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| {
                        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
                    });
                Ok(Some(FileInfo::new(meta.len(), modified_ms)))
            }
            _ => Ok(None),
        }
    }

    fn remove_file(&self, _path: &str) -> Result<(), ProviderError> {
        Err(refused(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a catalog source removes no file",
        ))
    }

    fn notice(&self, notice: HarnessNotice) -> Result<(), ProviderError> {
        self.0.notice(notice)
    }
}

/// One provider this host offers.
#[derive(Clone)]
struct Held {
    registry: Arc<Registry>,
    /// The extension that brought it, none for a built-in one.
    extension: Option<String>,
    info: SourceInfo,
}

/// Every `catalog_source` provider this host offers: the built-in ones first, then each extension's in the order loaded.
#[derive(Clone, Default)]
pub(crate) struct Sources {
    held: Vec<Held>,
}

impl core::fmt::Debug for Sources {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.held.iter().map(|each| &each.info.id))
            .finish()
    }
}

impl Sources {
    /// The providers that ship with this host: scripts it carries, found like any extension's. The registry is made once for each
    /// place the bundled files are kept.
    pub(crate) fn built_in() -> Self {
        static MADE: Mutex<Vec<(std::path::PathBuf, Sources)>> = Mutex::new(Vec::new());
        let home = crate::scripts::root();
        let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, sources)) = made.iter().find(|(place, _)| *place == home) {
            return sources.clone();
        }
        let sources = {
            let mut sources = Self::default();
            match BUNDLE.registry(&home) {
                Ok(registry) => {
                    if let Err(why) = sources.add(None, Arc::new(registry)) {
                        tracing::warn!("the built-in catalog sources were refused: {why}");
                    }
                }
                Err(why) => tracing::warn!("the built-in catalog sources are not offered: {why}"),
            }
            sources
        };
        made.push((home, sources.clone()));
        sources
    }

    /// Takes every `catalog_source` provider of `registry`, brought by the extension `extension` or built in. Nothing is taken
    /// when one is refused, or has an id another already has.
    pub(crate) fn add(
        &mut self,
        extension: Option<&str>,
        registry: Arc<Registry>,
    ) -> Result<(), String> {
        let mut found = Vec::new();
        for provider in registry.providers(KIND) {
            let id = provider.id().to_owned();
            let offer = registry
                .offer::<dyn CatalogSourceProvider>(&id)
                .ok_or_else(|| format!("no provider {id}"))?
                .map_err(|_| format!("{id} is not a catalog_source"))?;
            let info = offer
                .default_instance()
                .ok_or_else(|| format!("{id} needs settings"))?
                .describe()
                .map_err(|error| error.to_string())?;
            if info.id != id {
                return Err(format!("{id} describes itself as {}", info.id));
            }
            if let Some(problem) = info.problem() {
                return Err(problem);
            }
            if self
                .held
                .iter()
                .map(|each| &each.info)
                .chain(found.iter().map(|each: &Held| &each.info))
                .any(|taken| taken.id == info.id)
            {
                return Err(format!("the source {id:?} is already offered"));
            }
            found.push(Held {
                registry: Arc::clone(&registry),
                extension: extension.map(str::to_owned),
                info,
            });
        }
        self.held.extend(found);
        Ok(())
    }

    /// Each provider and the extension that brought it.
    pub(crate) fn list(&self) -> impl Iterator<Item = (&SourceInfo, Option<&str>)> {
        self.held
            .iter()
            .map(|each| (&each.info, each.extension.as_deref()))
    }

    /// What the provider `id` describes, if this host offers it.
    #[cfg(test)]
    pub(crate) fn info(&self, id: &str) -> Option<&SourceInfo> {
        self.held
            .iter()
            .map(|each| &each.info)
            .find(|info| info.id == id)
    }

    /// The catalog the provider `id` answers with for `request`, or why it gave none.
    pub(crate) fn read(&self, id: &str, request: ReadRequest) -> Result<String, String> {
        let held = self
            .held
            .iter()
            .find(|each| each.info.id == id)
            .ok_or_else(|| format!("this host has no catalog source {id:?}"))?;
        if !held.info.serves(&request.kind) {
            return Err(format!(
                "the source {id:?} does not serve the type {:?}",
                request.kind
            ));
        }
        held.registry
            .offer::<dyn CatalogSourceProvider>(id)
            .ok_or_else(|| format!("no provider {id}"))?
            .map_err(|_| format!("{id} is not a catalog_source"))?
            .default_instance()
            .ok_or_else(|| format!("{id} needs settings"))?
            .read(request, Reach(LocalSpawner::new()).into_object())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
pub(crate) mod tests;
