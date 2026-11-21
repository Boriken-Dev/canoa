//! The secret providers this host offers: each is a provider of the `secret_provider` kind, from the library the host carries
//! (the form and the system store) and from every library canoa.cfg's entries and extensions load, a script's included. Nothing
//! here names a provider the host does not find in a registry.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::secret_provider::{ProviderInfo, SecretProvider};
use coqui_acp_api::guatiao::Alloc;
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Map, Value, json};

use super::builtin;
use crate::config::Config;

/// The origin of the providers the host carries.
pub(crate) const BUILT_IN: &str = "built-in";

type Library = fn(
    coqui_acp_api::guatiao::library::Host,
) -> Option<coqui_acp_api::guatiao::library::kind::LibraryParts>;

/// One provider a registry offers.
struct Offered {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    id: String,
    info: ProviderInfo,
    schema: Option<Value>,
    form: Option<Value>,
}

/// A provider as the catalog lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Listed {
    /// Where it comes from: `built-in`, or the entry or extension that loads it.
    pub(crate) origin: String,
    /// The id entries name it by.
    pub(crate) id: String,
    pub(crate) info: ProviderInfo,
    /// The schema of the settings an entry of it takes, and the form that says how to show them.
    pub(crate) schema: Option<Value>,
    pub(crate) form: Option<Value>,
}

impl Listed {
    /// As `secrets/providers` answers it.
    pub(crate) fn json(&self) -> Value {
        json!({
            "id": self.id, "title": self.info.title, "description": self.info.description,
            "origin": self.origin, "can_store": self.info.can_store, "can_list": self.info.can_list,
            "needs_person": self.info.needs_person, "schema": self.schema, "form": self.form,
        })
    }
}

/// The providers a host offers, from every registry it was given.
pub(crate) struct Catalog {
    offered: Vec<Offered>,
}

impl core::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.offered.iter().map(|offered| &offered.id))
            .finish()
    }
}

fn to_guatiao(value: &Value) -> Result<coqui_acp_api::guatiao::Value, String> {
    guatiao_serde::adapt::to_value_in(value, Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())
}

fn registry_of(name: &str, library: Library) -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local(name, library)
        .map_err(|error| format!("{name}: {error}"))?;
    if loading.loaded().is_none() {
        return Err(format!("{name} was not accepted"));
    }
    Ok(registry)
}

fn instantiate(
    registry: &Registry,
    key: &str,
    config: &Map<String, Value>,
) -> Result<Arc<dyn SecretProvider>, String> {
    let offer = registry
        .offer::<dyn SecretProvider>(key)
        .ok_or_else(|| format!("the provider {key} is gone"))?
        .map_err(|_| format!("{key} is not a secret_provider"))?;
    if offer.builds_instances() {
        let value = to_guatiao(&Value::Object(config.clone()))?;
        let built = offer
            .instantiate(&value)
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(built))
    } else if config.is_empty() {
        offer
            .shared()
            .ok_or_else(|| format!("{key} needs settings"))
    } else {
        Err(format!("{key} takes no settings"))
    }
}

impl Catalog {
    /// The built-in providers alone.
    pub(crate) fn built_in() -> Self {
        Self::from_registries(Vec::new())
    }

    /// The built-in providers, then those `registries` offer, each registry as its origin names it. A provider whose id an earlier
    /// one took, or that cannot be described, is left out and logged.
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
        match registry_of("canoa-secret-providers", builtin::library as Library) {
            Ok(registry) => found.push((BUILT_IN.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the built-in secret providers are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut offered: Vec<Offered> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn SecretProvider>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn SecretProvider>(&key) else {
                    continue;
                };
                let id = offer.id().to_owned();
                let schema = offer.config_schema().and_then(|schema| {
                    serde_json::to_value(guatiao_serde::Serializable::from(schema)).ok()
                });
                let form = offer
                    .meta()
                    .and_then(|meta| meta.get("form"))
                    .and_then(|form| {
                        serde_json::to_value(guatiao_serde::Serializable::from(form)).ok()
                    });
                let described = instantiate(&registry, &key, &Map::new())
                    .and_then(|provider| provider.describe().map_err(|error| error.to_string()));
                let info = match described {
                    Ok(info) => info,
                    Err(why) => {
                        tracing::warn!(
                            origin,
                            provider = id,
                            "a secret provider cannot be described: {why}"
                        );
                        continue;
                    }
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = id,
                        "a secret provider is not offered: {problem}"
                    );
                    continue;
                }
                if offered.iter().any(|each| each.id == id) {
                    tracing::warn!(
                        origin,
                        provider = id,
                        "the secret provider {id} is already offered"
                    );
                    continue;
                }
                offered.push(Offered {
                    origin: origin.clone(),
                    registry: Arc::clone(&registry),
                    key,
                    id,
                    info,
                    schema,
                    form,
                });
            }
        }
        Self { offered }
    }

    /// The built-in providers, the script providers that ship in the host (kept in `home`, when the host has one) and those the
    /// libraries of `config`'s entries and extensions add.
    pub(crate) fn of(config: &Config, home: Option<&std::path::Path>) -> Self {
        let mut found = Self::bundled_scripts(home);
        found.extend(crate::control::providers::loaded(config).iter().cloned());
        Self::from_arcs(&found)
    }

    /// The script providers that ship in the host, as built-in ones.
    fn bundled_scripts(home: Option<&std::path::Path>) -> Vec<(String, Arc<Registry>)> {
        let Some(home) = home else {
            return Vec::new();
        };
        match super::bundled::registry(home) {
            Ok(registry) => vec![(BUILT_IN.to_owned(), Arc::new(registry))],
            Err(why) => {
                tracing::warn!(
                    "the script secret providers that ship in the host are not offered: {why}"
                );
                Vec::new()
            }
        }
    }

    /// The built-in providers and the script providers that ship in the host, kept in `home`.
    #[cfg(test)]
    pub(crate) fn built_in_in(home: &std::path::Path) -> Self {
        Self::from_arcs(&Self::bundled_scripts(Some(home)))
    }

    /// The providers offered, in the order they were found.
    pub(crate) fn list(&self) -> Vec<Listed> {
        self.offered
            .iter()
            .map(|offered| Listed {
                origin: offered.origin.clone(),
                id: offered.id.clone(),
                info: offered.info.clone(),
                schema: offered.schema.clone(),
                form: offered.form.clone(),
            })
            .collect()
    }

    /// The settings of provider `id` that may be another secret: the fields its schema marks `x-from-secret`.
    pub(crate) fn secret_sourced_settings(&self, id: &str) -> Vec<String> {
        self.offered
            .iter()
            .find(|offered| offered.id == id)
            .and_then(|offered| offered.schema.as_ref())
            .and_then(|schema| schema.get("properties"))
            .and_then(Value::as_object)
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(_, field)| field.get("x-from-secret") == Some(&Value::Bool(true)))
                    .map(|(key, _)| key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What provider `id` says of itself.
    pub(crate) fn info(&self, id: &str) -> Option<&ProviderInfo> {
        self.offered
            .iter()
            .find(|offered| offered.id == id)
            .map(|offered| &offered.info)
    }

    /// An instance of provider `id` from `config`, its settings as an entry holds them with any other secret they name resolved.
    pub(crate) fn instance(
        &self,
        id: &str,
        config: &Map<String, Value>,
    ) -> Result<Arc<dyn SecretProvider>, String> {
        let offered = self
            .offered
            .iter()
            .find(|offered| offered.id == id)
            .ok_or_else(|| format!("this host has no secret provider {id}"))?;
        instantiate(&offered.registry, &offered.key, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_offers_the_form_and_the_system_store_through_the_kind() {
        let catalog = Catalog::built_in();
        let listed = catalog.list();
        let ids: Vec<&str> = listed.iter().map(|each| each.id.as_str()).collect();
        assert_eq!(ids, ["file", "form", "system"]);
        assert!(listed.iter().all(|each| each.origin == BUILT_IN));
        let system = &listed[2];
        assert!(system.info.can_store && system.info.can_list && !system.info.needs_person);
        assert!(listed[1].info.needs_person);
        assert!(listed[0].info.can_store && !listed[0].info.can_list);
        // The system provider's settings are a schema with a form, which an application draws without knowing the provider.
        assert!(
            system
                .schema
                .as_ref()
                .is_some_and(|schema| schema["properties"]["service"].is_object())
        );
        assert!(system.form.is_some());
        assert!(catalog.info("ghost").is_none());
    }

    #[test]
    fn an_instance_takes_the_settings_its_schema_declares_and_no_other() {
        let catalog = Catalog::built_in();
        assert!(catalog.instance("system", &Map::new()).is_ok());
        let mut surprise = Map::new();
        surprise.insert("surprise".to_owned(), json!(1));
        assert!(catalog.instance("system", &surprise).is_err());
        assert!(catalog.instance("form", &surprise).is_err());
        assert!(catalog.instance("ghost", &Map::new()).is_err());
    }
}
