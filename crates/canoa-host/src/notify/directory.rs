//! The notifiers a host can tell: the providers of the `notifier` kind its libraries offer, the entries of `[notifications.notifiers]`
//! that choose one and give it settings, and where each event goes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::notifier::{Delivery, Notice, Notifier, NotifierInfo, SendRequest};
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Map, Value};

use super::events;
use crate::config::{Config, NotificationsConfig, NotifierConfig};

/// The origin of the providers the host itself carries.
pub(crate) const BUILT_IN: &str = "built-in";

/// The key under which the host tells a built-in provider which host it serves.
pub(crate) const HOST_KEY: &str = "_host";

/// Answers the secret named `notifiers/NAME/MEMBER`, or `None` when none is kept.
pub(crate) type Keeping = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// One provider of the `notifier` kind.
pub(crate) struct Offered {
    /// `built-in`, or the name of the entry or extension that loads it.
    pub(crate) origin: String,
    registry: Arc<Registry>,
    key: String,
    /// What it says of itself.
    pub(crate) info: NotifierInfo,
}

impl Offered {
    /// Delivers `notice` with `settings`, which is JSON text.
    pub(crate) fn send(&self, settings: String, notice: Notice) -> Result<Delivery, String> {
        let offer = self
            .registry
            .offer::<dyn Notifier>(&self.key)
            .ok_or_else(|| format!("the provider {} is gone", self.key))?
            .map_err(|_| format!("{} is not a notifier", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("{} needs settings", self.key))?;
        instance
            .send(SendRequest::new(settings, notice))
            .map_err(|error| canoa::notifier::NotifierError::from(error).to_string())
    }

    /// The schema of its settings as JSON, or null.
    pub(crate) fn schema(&self) -> Value {
        self.info
            .schema
            .as_ref()
            .and_then(|schema| crate::forms::json_of(schema).ok())
            .unwrap_or(Value::Null)
    }

    /// The form of its settings as JSON, or null.
    pub(crate) fn form(&self) -> Value {
        self.info
            .form
            .as_ref()
            .and_then(|form| crate::forms::json_of(form).ok())
            .unwrap_or(Value::Null)
    }
}

/// A notifier chosen by name, ready to be sent to.
pub(crate) struct Resolved<'a> {
    pub(crate) offered: &'a Offered,
    /// The settings, a sensitive member filled from the host's secrets.
    pub(crate) settings: Map<String, Value>,
}

/// What `[notifications]` and the loaded libraries say, read at one moment.
pub(crate) struct Directory {
    pub(crate) config: NotificationsConfig,
    offers: Vec<Offered>,
    keeping: Option<Keeping>,
}

impl core::fmt::Debug for Directory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Directory")
            .field(
                "notifiers",
                &self.config.notifiers.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

fn built_in() -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-notify", super::desktop::library)
        .map_err(|error| format!("canoa-notify: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-notify was not accepted".to_owned());
    }
    Ok(registry)
}

impl Directory {
    /// What `config` says, with the providers of the carried libraries and the libraries its entries and extensions load.
    pub(crate) fn of(config: &Config, keeping: Option<Keeping>) -> Self {
        Self::from_arcs(
            config.notifications.clone(),
            &crate::control::providers::loaded(config),
            keeping,
        )
    }

    /// As [`Self::of`] over registries already loaded.
    #[cfg(test)]
    pub(crate) fn from_registries(
        config: NotificationsConfig,
        registries: Vec<(String, Registry)>,
        keeping: Option<Keeping>,
    ) -> Self {
        let wrapped: Vec<(String, Arc<Registry>)> = registries
            .into_iter()
            .map(|(origin, registry)| (origin, Arc::new(registry)))
            .collect();
        Self::from_arcs(config, &wrapped, keeping)
    }

    fn from_arcs(
        config: NotificationsConfig,
        registries: &[(String, Arc<Registry>)],
        keeping: Option<Keeping>,
    ) -> Self {
        let mut found: Vec<(String, Arc<Registry>)> = Vec::new();
        match built_in() {
            Ok(registry) => found.push((BUILT_IN.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the built-in notifiers are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut offers: Vec<Offered> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn Notifier>()
                .filter(|offer| !crate::disabled::off::<dyn Notifier>(offer.id()))
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn Notifier>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|notifier| notifier.describe().ok())
                else {
                    tracing::warn!(origin, provider = key, "a notifier cannot be described");
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a notifier is not offered: {problem}"
                    );
                    continue;
                }
                if offers.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "the notifier {} is already offered",
                        info.id
                    );
                    continue;
                }
                offers.push(Offered {
                    origin: origin.clone(),
                    registry: Arc::clone(&registry),
                    key,
                    info,
                });
            }
        }
        Self {
            config,
            offers,
            keeping,
        }
    }

    /// Every provider offered.
    pub(crate) fn providers(&self) -> &[Offered] {
        &self.offers
    }

    /// The provider with the id `id`.
    pub(crate) fn provider(&self, id: &str) -> Option<&Offered> {
        self.offers.iter().find(|offered| offered.info.id == id)
    }

    /// The names of the notifiers `event` goes to: the file's list, else the catalog's default.
    pub(crate) fn route(&self, event: &str) -> Vec<String> {
        self.config
            .routes
            .get(event)
            .cloned()
            .unwrap_or_else(|| events::default_route(event))
    }

    /// The entry called `name`, when there is one.
    pub(crate) fn entry(&self, name: &str) -> Option<&NotifierConfig> {
        self.config.notifiers.get(name)
    }

    /// The names of every notifier: `desktop`, then the entries.
    pub(crate) fn names(&self) -> Vec<String> {
        core::iter::once(events::DESKTOP.to_owned())
            .chain(self.config.notifiers.keys().cloned())
            .collect()
    }

    /// The notifier `name`, its settings made ready.
    ///
    /// # Errors
    ///
    /// In one line: no such notifier, its provider is not offered, or its settings do not fit.
    pub(crate) fn resolve(&self, name: &str) -> Result<Resolved<'_>, String> {
        let (provider, mut settings) = if name == events::DESKTOP {
            (events::DESKTOP, Map::new())
        } else {
            let entry = self
                .entry(name)
                .ok_or_else(|| format!("this host has no notifier {name}"))?;
            (entry.provider.as_str(), entry.settings.as_map().clone())
        };
        let offered = self.provider(provider).ok_or_else(|| {
            format!("the notifier {name} needs the provider {provider}, which is not offered")
        })?;
        if offered.origin != BUILT_IN {
            let schema = offered.schema();
            for member in crate::forms::secrets_of(&schema) {
                if let Some(secret) = self
                    .keeping
                    .as_ref()
                    .and_then(|keeping| keeping(&secret_name(name, &member)))
                {
                    settings.insert(member, Value::String(secret));
                }
            }
            if offered.info.schema.is_some() {
                crate::forms::check(offered.info.schema.as_ref(), &settings)
                    .map_err(|why| format!("the notifier {name}: {why}"))?;
            }
        }
        Ok(Resolved { offered, settings })
    }
}

/// The name a notifier's sensitive member is kept under in the host's secrets.
pub(crate) fn secret_name(notifier: &str, member: &str) -> String {
    format!("notifiers/{notifier}/{member}")
}

/// The routes of every event as Settings shows them: each event, the notifiers it goes to, and whether the file says so.
pub(crate) fn routes_json(directory: &Directory) -> Value {
    let rows: Vec<Value> = events::CATALOG
        .iter()
        .map(|event| {
            serde_json::json!({
                "id": event.id,
                "title": event.title,
                "notifiers": directory.route(event.id),
                "set": directory.config.routes.contains_key(event.id),
            })
        })
        .collect();
    serde_json::json!({ "events": rows })
}
