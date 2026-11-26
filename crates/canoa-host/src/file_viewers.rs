//! The viewers the application's Files surface can show a file with, as the providers of this host offer them: each is a provider of
//! the `file_viewer` kind, which the libraries of canoa.cfg's entries and extensions add. Nothing here names a viewer or a type of
//! file: the application draws what a provider answers, and the host puts a policy on a page before it is given out.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::file_viewer::{
    FileViewer, MAX_RENDER_BYTES, Shows, ViewRequest, ViewerInfo, sealed_page,
};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use serde_json::{Value, json};

use crate::config::Config;

#[cfg(test)]
pub(crate) mod standin;

/// What a viewer drew: the A2UI messages of a surface, or the sealed document of a page.
#[derive(Debug, PartialEq)]
pub(crate) enum Drawn {
    /// A JSON array of A2UI messages.
    Surface(Value),
    /// One HTML document under the policy of [`canoa::file_viewer::PAGE_POLICY`].
    Page(String),
}

/// One viewer a registry offers.
struct Entry {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    info: ViewerInfo,
}

impl Entry {
    fn with<T>(
        &self,
        body: impl FnOnce(&dyn FileViewer) -> Result<T, ProviderError>,
    ) -> Result<T, String> {
        let offer = self
            .registry
            .offer::<dyn FileViewer>(&self.key)
            .ok_or_else(|| format!("the provider {} is gone", self.key))?
            .map_err(|_| format!("{} is not a file_viewer", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("{} needs settings", self.key))?;
        body(instance).map_err(|error| error.to_string())
    }
}

/// The viewers this host offers, from every registry it was given.
pub(crate) struct ViewerCatalog {
    entries: Vec<Entry>,
}

impl core::fmt::Debug for ViewerCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

impl ViewerCatalog {
    /// The viewers `registries` offer, each registry as its origin names it. A viewer whose id an earlier one took, or that cannot
    /// be described, is left out and logged.
    #[cfg(test)]
    pub(crate) fn from_registries(registries: Vec<(String, Registry)>) -> Self {
        let wrapped: Vec<(String, Arc<Registry>)> = registries
            .into_iter()
            .map(|(origin, registry)| (origin, Arc::new(registry)))
            .collect();
        Self::from_arcs(&wrapped)
    }

    /// As [`Self::from_registries`], sharing cached `Arc<Registry>` handles.
    pub(crate) fn from_arcs(registries: &[(String, Arc<Registry>)]) -> Self {
        let mut entries: Vec<Entry> = Vec::new();
        for (origin, registry) in registries {
            let keys: Vec<String> = registry
                .offers::<dyn FileViewer>()
                .filter(|offer| !crate::disabled::off::<dyn FileViewer>(offer.id()))
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn FileViewer>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|viewer| viewer.describe().ok())
                else {
                    tracing::warn!(origin, provider = key, "a file viewer cannot be described");
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a file viewer is not offered: {problem}"
                    );
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "the file viewer {} is already offered",
                        info.id
                    );
                    continue;
                }
                entries.push(Entry {
                    origin: origin.clone(),
                    registry: Arc::clone(registry),
                    key,
                    info,
                });
            }
        }
        Self { entries }
    }

    /// The viewers the libraries of `config`'s entries and extensions add.
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_arcs(&crate::control::providers::loaded(config))
    }

    /// The viewers as `file_viewers/list` answers: each with its `id`, `title`, `help`, `mime_types`, `extensions`, `priority`,
    /// `shows` (`surface` or `page`), `opens_sections` and `origin` (the name of the entry or extension that loads it).
    pub(crate) fn list(&self) -> Value {
        let viewers: Vec<Value> = self
            .entries
            .iter()
            .map(|entry| {
                let info = &entry.info;
                json!({
                    "id": info.id, "title": info.title, "help": info.help,
                    "mime_types": info.mime_types, "extensions": info.extensions,
                    "priority": info.priority, "opens_sections": info.opens_sections,
                    "shows": match info.shows { Shows::Surface => "surface", Shows::Page => "page" },
                    "origin": entry.origin,
                })
            })
            .collect();
        json!({ "viewers": viewers })
    }

    /// Whether the catalog has a viewer `id`.
    pub(crate) fn has(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.info.id == id)
    }

    /// What the viewer `id` draws of `request`; an error in words for a viewer that is none, whose provider refuses, or whose answer
    /// is too long or is not what its description says (a surface that is no list of messages).
    pub(crate) fn render(&self, id: &str, request: ViewRequest) -> Result<Drawn, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.info.id == id)
            .ok_or_else(|| format!("this host has no file viewer {id}"))?;
        let answer = entry.with(|viewer| viewer.render(request))?;
        if answer.len() > MAX_RENDER_BYTES {
            return Err(format!(
                "the viewer {id} answered {} bytes, more than the {MAX_RENDER_BYTES} a host takes",
                answer.len()
            ));
        }
        match entry.info.shows {
            Shows::Surface => match serde_json::from_str::<Value>(&answer) {
                Ok(messages @ Value::Array(_)) => Ok(Drawn::Surface(messages)),
                Ok(_) => Err(format!(
                    "the viewer {id} shows a surface and did not answer a list of A2UI messages"
                )),
                Err(error) => Err(format!(
                    "the viewer {id} shows a surface and its answer is not JSON: {error}"
                )),
            },
            Shows::Page => Ok(Drawn::Page(sealed_page(&answer))),
        }
    }
}
