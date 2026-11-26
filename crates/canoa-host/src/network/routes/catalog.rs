//! The routes a host offers: the providers of the `network_route` kind the host carries and those the libraries of canoa.cfg's entries
//! and extensions add, each with its schema and form.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::network_route::{NetworkRoute, RouteInfo};
use coqui_acp_api::guatiao::library::Registry;
use serde_json::Value;

use crate::config::Config;

/// The origin of the providers the host itself carries.
pub(crate) const BUILT_IN: &str = "built-in";

/// One provider the catalog offers.
struct Offered {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    id: String,
    info: RouteInfo,
    schema: Option<Value>,
    form: Option<Value>,
}

/// A provider as a page lists it.
#[derive(Debug, Clone)]
pub(crate) struct Listed {
    pub(crate) origin: String,
    pub(crate) id: String,
    pub(crate) info: RouteInfo,
    pub(crate) schema: Option<Value>,
    pub(crate) form: Option<Value>,
}

/// The providers of the `network_route` kind this host offers.
pub(crate) struct RouteCatalog {
    offered: Vec<Offered>,
}

impl core::fmt::Debug for RouteCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.offered.iter().map(|offered| &offered.id))
            .finish()
    }
}

fn to_guatiao(value: &Value) -> Result<coqui_acp_api::guatiao::Value, String> {
    guatiao_serde::adapt::to_value_in(value, coqui_acp_api::guatiao::Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())
}

fn instantiate(
    registry: &Registry,
    key: &str,
    settings: &Value,
) -> Result<Arc<dyn NetworkRoute>, String> {
    let offer = registry
        .offer::<dyn NetworkRoute>(key)
        .ok_or_else(|| format!("the provider {key} is gone"))?
        .map_err(|_| format!("{key} is not a network_route"))?;
    if offer.builds_instances() {
        let value = to_guatiao(settings)?;
        let built = offer
            .instantiate(&value)
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(built))
    } else if settings.as_object().is_none_or(serde_json::Map::is_empty) {
        offer
            .shared()
            .ok_or_else(|| format!("{key} needs settings"))
    } else {
        Err(format!("{key} takes no settings"))
    }
}

fn built_in_registry() -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-network-routes", super::library)
        .map_err(|error| format!("canoa-network-routes: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-network-routes was not accepted".to_owned());
    }
    Ok(registry)
}

impl RouteCatalog {
    /// The built-in providers, then those `registries` offer, each registry as its origin names it. A provider whose id an earlier one
    /// took, or that cannot be described, is left out and logged.
    pub(crate) fn from_arcs(registries: &[(String, Arc<Registry>)]) -> Self {
        let mut found: Vec<(String, Arc<Registry>)> = Vec::new();
        match built_in_registry() {
            Ok(registry) => found.push((BUILT_IN.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the built-in network routes are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut offered: Vec<Offered> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn NetworkRoute>()
                .filter(|offer| !crate::disabled::off::<dyn NetworkRoute>(offer.id()))
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn NetworkRoute>(&key) else {
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
                let described =
                    instantiate(&registry, &key, &Value::Object(serde_json::Map::new()))
                        .and_then(|route| route.describe().map_err(|error| error.to_string()));
                let info = match described {
                    Ok(info) => info,
                    Err(why) => {
                        tracing::warn!(
                            origin,
                            provider = id,
                            "a network route cannot be described: {why}"
                        );
                        continue;
                    }
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = id,
                        "a network route is not offered: {problem}"
                    );
                    continue;
                }
                if offered.iter().any(|each| each.id == id) {
                    tracing::warn!(
                        origin,
                        provider = id,
                        "the network route {id} is already offered"
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

    /// The built-in providers and those the libraries of `config`'s entries and extensions add.
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_arcs(&crate::control::providers::loaded(config))
    }

    /// The built-in providers alone.
    #[cfg(test)]
    pub(crate) fn built_in() -> Self {
        Self::from_arcs(&[])
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

    /// The route the provider `id` makes of `settings`.
    ///
    /// # Errors
    ///
    /// No such provider, or settings it does not take.
    pub(crate) fn instance(
        &self,
        id: &str,
        settings: &Value,
    ) -> Result<Arc<dyn NetworkRoute>, String> {
        let offered = self
            .offered
            .iter()
            .find(|offered| offered.id == id)
            .ok_or_else(|| format!("no network route provider called {id}"))?;
        instantiate(&offered.registry, &offered.key, settings)
    }
}
