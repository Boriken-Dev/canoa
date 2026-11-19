//! The sections of Settings the providers of this host offer: each is a provider of the `settings_section` kind, in a library the host
//! carries (a bundled agent's) or one the entries and extensions of canoa.cfg load. Nothing here names a section: the application
//! draws what a provider says of its section, and the section is gone with its provider.
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

#[cfg(test)]
pub(crate) mod standin;

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
        })
    }

    /// What the provider holds, for a section stored with it.
    pub(crate) fn read(&self) -> Result<String, String> {
        self.entry.with(|section| section.read())
    }

    /// Hands `values` to the provider, for a section stored with it.
    pub(crate) fn write(&self, values: String) -> Result<(), String> {
        self.entry.with(|section| section.write(values))
    }
}

impl SectionCatalog {
    /// The sections `registries` offer, each registry as its origin names it. A section whose id an earlier one took, or that cannot
    /// be described, is left out and logged.
    pub(crate) fn from_registries(registries: Vec<(String, Registry)>) -> Self {
        let mut entries: Vec<Entry> = Vec::new();
        for (origin, registry) in registries {
            let registry = Arc::new(registry);
            let keys: Vec<String> = registry
                .offers::<dyn SettingsSection>()
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
        Self { entries }
    }

    /// The sections the bundled libraries and those of `config`'s entries and extensions offer, each library opened as the host opens
    /// it for `providers/list`: an extension whose pin no longer matches is left out.
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_registries(crate::control::providers::loaded(config))
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
