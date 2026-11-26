//! The sections of Settings the providers of this host offer: each is a provider of the `settings_section` kind, the application's own
//! pages (`built_in`) registered the way an extension's library is, in a library the host carries (a bundled agent's) or one the
//! entries and extensions of canoa.cfg load. The application draws what a provider says of its section, and the section is gone with
//! its provider.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::settings_section::{SectionInfo, SettingsSection, Stored};
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Value, json};

use crate::config::Config;

mod built_in;
mod power;
#[cfg(test)]
pub(crate) mod standin;

/// The origin of the sections the application draws itself, which the host carries.
pub(crate) const BUILT_IN: &str = "built-in";

/// One section a registry offers.
struct Entry {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    info: SectionInfo,
}

impl Entry {
    /// Runs `body` with the provider's instance.
    fn with<T>(
        &self,
        body: impl FnOnce(
            &dyn SettingsSection,
        ) -> Result<T, coqui_acp_api::guatiao::library::ProviderError>,
    ) -> Result<T, String> {
        let offer = self
            .registry
            .offer::<dyn SettingsSection>(&self.key)
            .ok_or_else(|| format!("the provider {} is gone", self.key))?
            .map_err(|_| format!("{} is not a settings_section", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("{} needs settings", self.key))?;
        body(instance).map_err(|error| error.to_string())
    }
}

/// The sections this host offers, from every registry it was given.
pub(crate) struct SectionCatalog {
    entries: Vec<Entry>,
}

impl core::fmt::Debug for SectionCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

/// A section as the catalog lists it.
pub(crate) struct Section<'a> {
    entry: &'a Entry,
}

impl Section<'_> {
    pub(crate) fn info(&self) -> &SectionInfo {
        &self.entry.info
    }

    /// Whether this is one of the pages the application draws itself.
    pub(crate) fn is_built_in(&self) -> bool {
        self.entry.origin == BUILT_IN
    }

    /// The section as `settings/sections` answers.
    pub(crate) fn json(&self) -> Value {
        let described = |value: &Option<_>| match value {
            Some(value) => crate::forms::json_of(value).unwrap_or(Value::Null),
            None => Value::Null,
        };
        let info = &self.entry.info;
        json!({
            "id": info.id, "title": info.title, "help": info.help, "icon": info.icon,
            "group": info.group, "under": info.under, "keywords": info.keywords,
            "origin": self.entry.origin,
            "stored": match info.stored { Stored::Config => "config", Stored::Provider => "provider" },
            "schema": described(&info.schema), "form": described(&info.form),
            "collection": info.collection.as_ref().map(|collection| json!({
                "list": collection.list, "add": collection.add, "remove": collection.remove,
                "test": collection.test, "items": collection.items, "kinds": collection.kinds,
                "kind_member": collection.kind_member, "add_label": collection.add_label,
                "empty": collection.empty, "empty_help": collection.empty_help,
                "kind_label": collection.kind_label, "test_label": collection.test_label,
                "remove_note": collection.remove_note,
            })),
        })
    }

    /// What the provider holds, for a section stored with it.
    pub(crate) fn read(&self) -> Result<coqui_acp_api::guatiao::Value, String> {
        self.entry.with(|section| section.read())
    }

    /// Hands `values` to the provider, for a section stored with it.
    pub(crate) fn write(&self, values: coqui_acp_api::guatiao::Value) -> Result<(), String> {
        self.entry.with(|section| section.write(values))
    }
}

fn built_in() -> Result<Registry, String> {
    let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-settings-sections", built_in::library)
        .map_err(|error| format!("canoa-settings-sections: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-settings-sections was not accepted".to_owned());
    }
    Ok(registry)
}

impl SectionCatalog {
    /// The sections the application draws itself, then those `registries` offer, each registry as its origin names it. A section whose
    /// id an earlier one took, or that cannot be described, is left out and logged.
    pub(crate) fn from_registries(registries: Vec<(String, Registry)>) -> Self {
        let wrapped: Vec<(String, Arc<Registry>)> = registries
            .into_iter()
            .map(|(origin, registry)| (origin, Arc::new(registry)))
            .collect();
        Self::from_arcs(&wrapped)
    }

    /// As [`Self::from_registries`], sharing cached `Arc<Registry>` handles.
    pub(crate) fn from_arcs(registries: &[(String, Arc<Registry>)]) -> Self {
        let mut found: Vec<(String, Arc<Registry>)> = Vec::new();
        match built_in() {
            Ok(registry) => found.push((BUILT_IN.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the built-in settings sections are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut entries: Vec<Entry> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn SettingsSection>()
                .filter(|offer| !crate::disabled::off::<dyn SettingsSection>(offer.id()))
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn SettingsSection>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|section| section.describe().ok())
                else {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a settings section cannot be described"
                    );
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a settings section is not offered: {problem}"
                    );
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "the settings section {} is already offered",
                        info.id
                    );
                    continue;
                }
                entries.push(Entry {
                    origin: origin.clone(),
                    registry: Arc::clone(&registry),
                    key,
                    info,
                });
            }
        }
        // A registry offers its providers by name: the application's own sections keep the order it gives them, ahead of the others.
        entries.sort_by_key(|entry| {
            if entry.origin == BUILT_IN {
                built_in::ORDER
                    .iter()
                    .position(|id| *id == entry.info.id)
                    .unwrap_or(built_in::ORDER.len())
            } else {
                usize::MAX
            }
        });
        Self { entries }
    }

    /// The sections the application draws itself, the bundled libraries' and those of `config`'s entries and extensions, each library
    /// opened as the host opens it for `providers/list`: an extension whose pin no longer matches is left out.
    pub(crate) fn of(config: &Config) -> Self {
        let mut registries = crate::control::providers::loaded(config).to_vec();
        match power::registry() {
            Ok(registry) => registries.push(("bundled".to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the host's own settings groups are not offered: {why}"),
        }
        Self::from_arcs(&registries)
    }

    /// The sections offered, in the order they were found.
    pub(crate) fn list(&self) -> Vec<Section<'_>> {
        self.entries.iter().map(|entry| Section { entry }).collect()
    }

    /// The section called `id`.
    pub(crate) fn by_id(&self, id: &str) -> Option<Section<'_>> {
        self.entries
            .iter()
            .find(|entry| entry.info.id == id)
            .map(|entry| Section { entry })
    }
}
