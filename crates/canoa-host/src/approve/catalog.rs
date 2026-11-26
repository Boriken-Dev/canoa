//! The approvers this host offers: each is a provider of the `approver` kind, from the library the host carries (the person's rules)
//! and from every library canoa.cfg's entries and extensions load. Nothing here names a provider the host does not find in a
//! registry.

use alloc::sync::Arc;

use canoa::approver::Approver;
use coqui_acp_api::guatiao::Alloc;
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Map, Value};

struct Offered {
    registry: Arc<Registry>,
    key: String,
    id: String,
}

/// The `approver` providers a host offers, from every registry it was given.
#[derive(Default)]
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

impl Catalog {
    /// The built-in providers, then those `registries` offer. A provider whose id an earlier one took is left out and logged.
    pub(crate) fn of(registries: &[Arc<Registry>]) -> Self {
        let mut found: Vec<Arc<Registry>> = Vec::new();
        match super::policy::registry() {
            Ok(registry) => found.push(Arc::new(registry)),
            Err(why) => tracing::warn!("the built-in approvers are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut offered: Vec<Offered> = Vec::new();
        for registry in found {
            let keys: Vec<(String, String)> = registry
                .offers::<dyn Approver>()
                .filter(|offer| !crate::disabled::off::<dyn Approver>(offer.id()))
                .filter_map(|offer| Some((offer.key()?.to_owned(), offer.id().to_owned())))
                .collect();
            for (key, id) in keys {
                if offered.iter().any(|each| each.id == id) {
                    tracing::warn!(provider = id, "the approver {id} is already offered");
                    continue;
                }
                offered.push(Offered {
                    registry: Arc::clone(&registry),
                    key,
                    id,
                });
            }
        }
        Self { offered }
    }

    /// An instance of provider `id` with `settings`.
    pub(crate) fn instance(
        &self,
        id: &str,
        settings: &Map<String, Value>,
    ) -> Result<Arc<dyn Approver>, String> {
        let offered = self
            .offered
            .iter()
            .find(|offered| offered.id == id)
            .ok_or_else(|| format!("no approver provider is called {id}"))?;
        let offer = offered
            .registry
            .offer::<dyn Approver>(&offered.key)
            .ok_or_else(|| format!("the provider {id} is gone"))?
            .map_err(|_| format!("{id} is not an approver"))?;
        if offer.builds_instances() {
            let value =
                guatiao_serde::adapt::to_value_in(&Value::Object(settings.clone()), Alloc::rust())
                    .map_err(|_| "the settings cannot be read".to_owned())?;
            let built = offer
                .instantiate(&value)
                .map_err(|error| error.to_string())?;
            Ok(Arc::new(built))
        } else if settings.is_empty() {
            offer.shared().ok_or_else(|| format!("{id} needs settings"))
        } else {
            Err(format!("{id} takes no settings"))
        }
    }
}
