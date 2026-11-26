//! The content registry: the providers of the `content_source` kind (`file`, `https`, `data` and `git` ship in the host; a library
//! adds more) and of the `catalog_generator` kind, and the one table of URI schemes they serve.
//!
//! A URI is read through the provider that serves its scheme. A string with no scheme is a path on this computer. Two providers
//! claiming a scheme never both load: the first stays, the second is listed as refused with the reason, and the library's other
//! providers load. `[host.disabled]` takes a provider and all its schemes away, by the provider's id.
//!
//! A catalog generator is handed a [`HostReach`](reading::HostReach): the registry, limited to the entry's `source` and the
//! places the generator declares, so a generator never opens a connection of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and processes are std's"
)]

mod command;
mod data;
mod file;
mod git;
mod https;
mod reading;

use std::path::Path;
use std::sync::Arc;

use canoa::content::{
    CatalogGeneratorProvider, ContentHost, ContentInfo, ContentSourceProvider, GENERATOR_KIND,
    GenerateRequest, GeneratorInfo, ListRequest, Listing, OpenRequest, Refusal, RefusalKind,
    Resolved, SOURCE_KIND, Stat, Target, scheme_of,
};
use coqui_acp_api::ProcessSpawner as _;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_harness::LocalSpawner;

use crate::store::sources::{Fetching, Reach};

/// The most a document read through the registry may hold unless the caller names less: what `[host.content] document_bytes`
/// defaults to.
pub(crate) const DOCUMENT_BYTES: u64 = 8 << 20;

/// Where a provider's code lives.
enum Backend<K: ?Sized> {
    /// Written in the host.
    Native(Arc<K>),
    /// Offered by a library's registry.
    Loaded { registry: Arc<Registry>, id: String },
}

impl<K: ?Sized> Clone for Backend<K> {
    fn clone(&self) -> Self {
        match self {
            Self::Native(provider) => Self::Native(Arc::clone(provider)),
            Self::Loaded { registry, id } => Self::Loaded {
                registry: Arc::clone(registry),
                id: id.clone(),
            },
        }
    }
}

/// One content source provider this host offers.
#[derive(Clone)]
struct SourceHeld {
    backend: Backend<dyn ContentSourceProvider>,
    /// The library's extension that brought it, none for one built in.
    extension: Option<String>,
    info: ContentInfo,
}

/// One catalog generator this host offers.
#[derive(Clone)]
struct GeneratorHeld {
    backend: Backend<dyn CatalogGeneratorProvider>,
    extension: Option<String>,
    info: GeneratorInfo,
}

/// A provider a library offered that this host does not run, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    /// `content_source` or `catalog_generator`.
    pub(crate) kind: &'static str,
    /// The provider's id.
    pub(crate) id: String,
    /// The extension that brought it.
    pub(crate) extension: Option<String>,
    /// Why it is not offered.
    pub(crate) why: String,
}

/// One scheme of the table, as the Content page and `content/schemes` list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SchemeRow {
    pub(crate) scheme: String,
    pub(crate) grammar: String,
    pub(crate) title: String,
    pub(crate) provider: String,
    pub(crate) extension: Option<String>,
    pub(crate) lists: bool,
    pub(crate) resolves: bool,
    /// Whether `[host.disabled] content_source` takes its provider away.
    pub(crate) on: bool,
}

/// What the built-in providers are made from.
#[derive(Clone, Debug, Default)]
struct Built {
    fetching: Fetching,
    /// The folder fetched repositories are kept in; none leaves the `git+` schemes refusing.
    git_root: Option<std::path::PathBuf>,
}

/// Every content source and catalog generator this host offers.
#[derive(Clone)]
pub(crate) struct Content {
    sources: Vec<SourceHeld>,
    generators: Vec<GeneratorHeld>,
    refused: Vec<Refused>,
    built: Built,
    /// The most a document read without a limit of its own may hold.
    document_bytes: Option<u64>,
}

impl Default for Content {
    fn default() -> Self {
        Self::built_in(Fetching::default(), None)
    }
}

impl core::fmt::Debug for Content {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Content")
            .field(
                "sources",
                &self
                    .sources
                    .iter()
                    .map(|each| &each.info.id)
                    .collect::<Vec<_>>(),
            )
            .field(
                "generators",
                &self
                    .generators
                    .iter()
                    .map(|each| &each.info.id)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

fn failed(status: Status, why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(status, why.as_ref())
}

/// A provider's own failure, worded as a refusal of the place it was asked about.
fn refusal_of(id: &str, error: &ProviderError) -> Refusal {
    let kind = if error.status == Status::GUATIAO_ERR_NOT_FOUND {
        RefusalKind::NotFound
    } else {
        RefusalKind::BadReply
    };
    Refusal::new(kind, format!("{id}: {}", error.message()))
}

fn too_large(max: u64, uri: &str) -> Refusal {
    Refusal::new(
        RefusalKind::TooLarge,
        format!("{} holds more than {max} bytes", shown(uri)),
    )
}

/// `uri` as it is shown in a refusal: no userinfo, no query token.
pub(crate) fn shown(uri: &str) -> String {
    let bare = uri.split(['?', '#']).next().unwrap_or(uri);
    let Some((scheme, rest)) = bare.split_once("://") else {
        return bare.to_owned();
    };
    let (authority, tail) = rest
        .split_once('/')
        .map_or((rest, None), |(authority, tail)| (authority, Some(tail)));
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    match tail {
        Some(tail) => format!("{scheme}://{host}/{tail}"),
        None => format!("{scheme}://{host}"),
    }
}

impl SourceHeld {
    /// Runs `call` on the provider.
    fn with<R>(&self, call: impl FnOnce(&dyn ContentSourceProvider) -> R) -> Result<R, String> {
        match &self.backend {
            Backend::Native(provider) => Ok(call(provider.as_ref())),
            Backend::Loaded { registry, id } => {
                let offer = registry
                    .offer::<dyn ContentSourceProvider>(id)
                    .ok_or_else(|| format!("no provider {id}"))?
                    .map_err(|_| format!("{id} is not a content_source"))?;
                let provider = offer
                    .default_instance()
                    .ok_or_else(|| format!("{id} needs settings"))?;
                Ok(call(provider))
            }
        }
    }

    fn off(&self) -> bool {
        crate::disabled::is_off(SOURCE_KIND, &self.info.id)
    }
}

impl GeneratorHeld {
    fn with<R>(&self, call: impl FnOnce(&dyn CatalogGeneratorProvider) -> R) -> Result<R, String> {
        match &self.backend {
            Backend::Native(provider) => Ok(call(provider.as_ref())),
            Backend::Loaded { registry, id } => {
                let offer = registry
                    .offer::<dyn CatalogGeneratorProvider>(id)
                    .ok_or_else(|| format!("no provider {id}"))?
                    .map_err(|_| format!("{id} is not a catalog_generator"))?;
                let provider = offer
                    .default_instance()
                    .ok_or_else(|| format!("{id} needs settings"))?;
                Ok(call(provider))
            }
        }
    }

    fn off(&self) -> bool {
        crate::disabled::is_off(GENERATOR_KIND, &self.info.id)
    }
}

impl Content {
    /// The providers that ship with the host, reading over `fetching` and keeping fetched repositories in `git_root`.
    pub(crate) fn built_in(fetching: Fetching, git_root: Option<&Path>) -> Self {
        let mut content = Self {
            sources: Vec::new(),
            generators: Vec::new(),
            refused: Vec::new(),
            built: Built::default(),
            document_bytes: None,
        };
        content.build(Built {
            fetching,
            git_root: git_root.map(Path::to_path_buf),
        });
        content
    }

    /// Makes the built-in providers again from `built`, keeping what libraries added.
    fn build(&mut self, built: Built) {
        self.sources.retain(|each| each.extension.is_some());
        self.generators.retain(|each| each.extension.is_some());
        let mut sources = vec![
            native_source(file::FileSource),
            native_source(https::HttpsSource::new(built.fetching.clone())),
            native_source(data::DataSource),
            native_source(git::GitSource::new(built.git_root.as_deref())),
        ];
        sources.append(&mut self.sources);
        self.sources = sources;
        match command::CommandGenerator::held() {
            Ok(held) => self.generators.insert(0, held),
            Err(why) => tracing::warn!("the built-in command generator is not offered: {why}"),
        }
        self.built = built;
    }

    /// These providers reading over `fetching`.
    pub(crate) fn fetching(mut self, fetching: Fetching) -> Self {
        let built = Built {
            fetching,
            ..self.built.clone()
        };
        self.build(built);
        self
    }

    /// These providers keeping fetched repositories in `folder`.
    pub(crate) fn keeping_in(mut self, folder: Option<&Path>) -> Self {
        let built = Built {
            git_root: folder.map(Path::to_path_buf),
            ..self.built.clone()
        };
        self.build(built);
        self
    }

    /// These providers reading documents of at most `bytes` when the caller names no limit.
    pub(crate) const fn document_bytes(mut self, bytes: Option<u64>) -> Self {
        self.document_bytes = bytes;
        self
    }

    /// The most a document read without a limit of its own may hold.
    pub(crate) fn most_bytes(&self) -> u64 {
        self.document_bytes.unwrap_or(DOCUMENT_BYTES)
    }

    /// Takes every `content_source` and `catalog_generator` provider of `registry`, brought by `extension` or built in. A provider
    /// that is refused (a scheme or an id another already has, a description that is no description) is listed with the reason and
    /// the library's other providers still load.
    pub(crate) fn add(&mut self, extension: Option<&str>, registry: &Arc<Registry>) {
        for provider in registry.providers(SOURCE_KIND) {
            let id = provider.id().to_owned();
            if let Err(why) = self.add_source(extension, registry, &id) {
                self.refuse(SOURCE_KIND, &id, extension, why);
            }
        }
        for provider in registry.providers(GENERATOR_KIND) {
            let id = provider.id().to_owned();
            if let Err(why) = self.add_generator(extension, registry, &id) {
                self.refuse(GENERATOR_KIND, &id, extension, why);
            }
        }
    }

    fn refuse(&mut self, kind: &'static str, id: &str, extension: Option<&str>, why: String) {
        tracing::warn!("the {kind} {id:?} is not offered: {why}");
        self.refused
            .retain(|each| !(each.kind == kind && each.id == id));
        self.refused.push(Refused {
            kind,
            id: id.to_owned(),
            extension: extension.map(str::to_owned),
            why,
        });
    }

    fn add_source(
        &mut self,
        extension: Option<&str>,
        registry: &Arc<Registry>,
        id: &str,
    ) -> Result<(), String> {
        let offer = registry
            .offer::<dyn ContentSourceProvider>(id)
            .ok_or_else(|| format!("no provider {id}"))?
            .map_err(|_| format!("{id} is not a content_source"))?;
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
        if self.sources.iter().any(|each| each.info.id == info.id) {
            return Err(format!("the content source {id:?} is already offered"));
        }
        if let Some((scheme, held)) = info.schemes.iter().find_map(|scheme| {
            self.sources
                .iter()
                .find(|each| {
                    each.info
                        .schemes
                        .iter()
                        .any(|taken| taken.name == scheme.name)
                })
                .map(|held| (&scheme.name, held))
        }) {
            return Err(format!("{scheme} is already offered by {}", held.info.id));
        }
        self.sources.push(SourceHeld {
            backend: Backend::Loaded {
                registry: Arc::clone(registry),
                id: id.to_owned(),
            },
            extension: extension.map(str::to_owned),
            info,
        });
        Ok(())
    }

    fn add_generator(
        &mut self,
        extension: Option<&str>,
        registry: &Arc<Registry>,
        id: &str,
    ) -> Result<(), String> {
        let offer = registry
            .offer::<dyn CatalogGeneratorProvider>(id)
            .ok_or_else(|| format!("no provider {id}"))?
            .map_err(|_| format!("{id} is not a catalog_generator"))?;
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
        if self.generators.iter().any(|each| each.info.id == info.id) {
            return Err(format!("the generator {id:?} is already offered"));
        }
        self.generators.push(GeneratorHeld {
            backend: Backend::Loaded {
                registry: Arc::clone(registry),
                id: id.to_owned(),
            },
            extension: extension.map(str::to_owned),
            info,
        });
        Ok(())
    }

    /// The providers a library offered that are not run, with the reasons.
    pub(crate) fn refused(&self) -> &[Refused] {
        &self.refused
    }

    /// Each scheme the providers serve, the built-in ones first.
    pub(crate) fn schemes(&self) -> Vec<SchemeRow> {
        self.sources
            .iter()
            .flat_map(|held| {
                held.info.schemes.iter().map(move |scheme| SchemeRow {
                    scheme: scheme.name.clone(),
                    grammar: scheme.grammar.clone(),
                    title: scheme.title.clone(),
                    provider: held.info.id.clone(),
                    extension: held.extension.clone(),
                    lists: scheme.lists,
                    resolves: scheme.resolves,
                    on: !held.off(),
                })
            })
            .collect()
    }

    /// Each generator and the extension that brought it.
    pub(crate) fn generators(&self) -> impl Iterator<Item = (&GeneratorInfo, Option<&str>)> {
        self.generators
            .iter()
            .filter(|each| !each.off())
            .map(|each| (&each.info, each.extension.as_deref()))
    }

    /// The provider that serves the scheme of `uri`.
    fn route(&self, uri: &str) -> Result<&SourceHeld, Refusal> {
        let scheme = scheme_of(uri).unwrap_or_else(|| "file".to_owned());
        if canoa::content::holds_password(uri) {
            return Err(Refusal::new(
                RefusalKind::RefusedPlace,
                format!(
                    "{} holds a password: a secret goes in the entry's credential, not in the address",
                    shown(uri)
                ),
            ));
        }
        let serving = |held: &&SourceHeld| held.info.schemes.iter().any(|each| each.name == scheme);
        let Some(held) = self.sources.iter().find(serving) else {
            let offered: Vec<&str> = self
                .sources
                .iter()
                .flat_map(|held| held.info.schemes.iter().map(|each| each.name.as_str()))
                .collect();
            return Err(Refusal::new(
                RefusalKind::RefusedPlace,
                format!(
                    "no provider offers {scheme}:, which {} names; the schemes offered are {}",
                    shown(uri),
                    offered.join(", ")
                ),
            ));
        };
        if held.off() {
            return Err(Refusal::new(
                RefusalKind::RefusedPlace,
                format!(
                    "{scheme}: is served by {}, which [host.disabled] turns off",
                    held.info.id
                ),
            ));
        }
        Ok(held)
    }

    fn ask<T>(
        held: &SourceHeld,
        call: impl FnOnce(&dyn ContentSourceProvider) -> Result<T, ProviderError>,
    ) -> Result<T, Refusal> {
        held.with(call)
            .map_err(|why| Refusal::new(RefusalKind::BadReply, format!("{}: {why}", held.info.id)))?
            .map_err(|error| refusal_of(&held.info.id, &error))
    }

    fn target(uri: &str, person: bool) -> Target {
        if person {
            Target::own(uri)
        } else {
            Target::named(uri)
        }
    }

    /// What is at `uri`.
    ///
    /// # Errors
    ///
    /// Why nothing could be said: no provider for the scheme, or the place refused.
    pub(crate) fn stat(&self, uri: &str, person: bool) -> Result<Stat, Refusal> {
        let held = self.route(uri)?;
        Self::ask(held, |provider| provider.stat(Self::target(uri, person)))?.into_result()
    }

    /// The first page of what is under `uri`, at most `max_entries`.
    ///
    /// # Errors
    ///
    /// As [`stat`](Self::stat).
    pub(crate) fn list(
        &self,
        uri: &str,
        person: bool,
        max_entries: u32,
    ) -> Result<Listing, Refusal> {
        let held = self.route(uri)?;
        let mut listing = Self::ask(held, |provider| {
            provider.list(ListRequest::of(Self::target(uri, person), max_entries))
        })?
        .into_result()?;
        // A name that is not one path component is not an entry.
        listing
            .entries
            .retain(|entry| entry.path.split('/').all(canoa::content::is_component));
        Ok(listing)
    }

    /// The bytes of the file at `uri`, at most `max_bytes`.
    ///
    /// # Errors
    ///
    /// As [`stat`](Self::stat), or the file is longer than `max_bytes`.
    pub(crate) fn open_bytes(
        &self,
        uri: &str,
        person: bool,
        max_bytes: u64,
    ) -> Result<Vec<u8>, Refusal> {
        let held = self.route(uri)?;
        let opened = Self::ask(held, |provider| {
            provider.open(OpenRequest::of(Self::target(uri, person), max_bytes))
        })?
        .into_result()?;
        if let Some(inline) = opened.delivery.inline {
            return if inline.0.len() as u64 > max_bytes {
                Err(too_large(max_bytes, uri))
            } else {
                Ok(inline.0)
            };
        }
        if let Some(locator) = opened.delivery.locator {
            let allowed = held.info.endpoints.clone();
            return https::get_bytes(&self.built.fetching, &locator.url, allowed, max_bytes);
        }
        Err(Refusal::new(
            RefusalKind::BadReply,
            format!("{} gave no content for {}", held.info.id, shown(uri)),
        ))
    }

    /// The text of the file at `uri`, at most `max_bytes`.
    ///
    /// # Errors
    ///
    /// As [`open_bytes`](Self::open_bytes), or the file is not UTF-8 text.
    pub(crate) fn open_text(
        &self,
        uri: &str,
        person: bool,
        max_bytes: u64,
    ) -> Result<String, Refusal> {
        String::from_utf8(self.open_bytes(uri, person, max_bytes)?)
            .map_err(|_| Refusal::new(RefusalKind::BadReply, format!("{} is not text", shown(uri))))
    }

    /// `uri` with its revision fixed.
    ///
    /// # Errors
    ///
    /// As [`stat`](Self::stat), or the scheme has no revisions.
    pub(crate) fn resolve(&self, uri: &str, person: bool) -> Result<Resolved, Refusal> {
        let held = self.route(uri)?;
        Self::ask(held, |provider| provider.resolve(Self::target(uri, person)))?.into_result()
    }

    /// The catalog the generator `id` makes for `request`, reading through the registry only `source`'s place and what the
    /// generator declares.
    ///
    /// # Errors
    ///
    /// Why the generator gave no catalog.
    pub(crate) fn generate(&self, id: &str, request: GenerateRequest) -> Result<String, String> {
        let held = self
            .generators
            .iter()
            .filter(|each| !each.off())
            .find(|each| each.info.id == id)
            .ok_or_else(|| {
                let offered: Vec<&str> = self
                    .generators()
                    .map(|(info, _)| info.id.as_str())
                    .collect();
                format!(
                    "this host has no catalog generator {id:?}; it offers {}",
                    offered.join(", ")
                )
            })?;
        if !held.info.serves(&request.kind) {
            return Err(format!(
                "the generator {id:?} does not serve the type {:?}",
                request.kind
            ));
        }
        match (held.info.source, request.source.as_deref()) {
            (canoa::content::SourceUse::Required, None) => {
                return Err(format!("the generator {id:?} reads a source: name it"));
            }
            (canoa::content::SourceUse::Absent, Some(_)) => {
                return Err(format!("the generator {id:?} reads no source"));
            }
            _ => {}
        }
        let mut places: Vec<reading::Place> = held
            .info
            .endpoints
            .iter()
            .map(|each| reading::Place::declared(each))
            .collect();
        if let Some(source) = &request.source {
            places.push(reading::Place::written(source));
        }
        let host = ContentHost::into_object(reading::HostReach::new(self.clone(), places));
        held.with(|generator| {
            generator.generate(request, Reach(LocalSpawner::new()).into_object(), host)
        })?
        .map_err(|error| error.to_string())
    }
}

fn native_source<P: ContentSourceProvider + 'static>(provider: P) -> SourceHeld {
    let info = provider
        .describe()
        .unwrap_or_else(|_| ContentInfo::new("native", "A built-in source"));
    SourceHeld {
        backend: Backend::Native(Arc::new(provider)),
        extension: None,
        info,
    }
}

#[cfg(test)]
#[path = "content/tests.rs"]
pub(crate) mod tests;
