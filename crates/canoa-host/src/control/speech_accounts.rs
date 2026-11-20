//! The speech-to-text services the accounts the person has can call, as the libraries this host loads describe them (`account_speech`).
//!
//! The host holds no service's address, models or credential: a library says which kinds of account (the standard they provide at the
//! address their model API starts from) may call a service, where a recording is posted, which credential holds the key and which
//! models transcribe. This file matches an account to those descriptions; [`super::speech_offers`] offers and lends from the match.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the registry is the host's"
)]

use canoa::speech::account::{AccountSpeech, AccountSpeechService};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountKindInfo, EndpointSlot, MODEL_API};
use serde_json::{Value, json};

use crate::accounts::matching::Catalogs;
use crate::config::{AccountConfig, Config};

/// The services the libraries this host loads describe.
#[derive(Debug, Default, Clone)]
pub(crate) struct Services {
    services: Vec<AccountSpeechService>,
}

impl Services {
    /// The services of the libraries this build carries and, when the host has a canoa.cfg, of the ones its entries and extensions
    /// load.
    pub(crate) fn of(config: Option<&Config>) -> Self {
        let mut carried = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        if let Err(why) = crate::bundled::register_included(&mut carried) {
            tracing::warn!("the libraries this build carries are not read for speech: {why}");
        }
        let mut registries = vec![carried];
        if let Some(config) = config {
            registries.extend(
                super::providers::loaded(config)
                    .into_iter()
                    .map(|(_, registry)| registry),
            );
        }
        Self::from_registries(&registries)
    }

    /// The services `registries` offer, in the order they are given. A service that cannot be offered, or whose id an earlier one took,
    /// is left out and logged.
    pub(crate) fn from_registries(registries: &[Registry]) -> Self {
        let mut services: Vec<AccountSpeechService> = Vec::new();
        for registry in registries {
            for offer in registry.offers::<dyn AccountSpeech>() {
                let id = offer.id().to_owned();
                let Some(info) = offer
                    .default_instance()
                    .and_then(|provider| provider.describe().ok())
                else {
                    tracing::warn!(
                        provider = id.as_str(),
                        "its speech services cannot be described"
                    );
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        provider = id.as_str(),
                        "its speech services are not offered: {problem}"
                    );
                    continue;
                }
                for service in info.services {
                    if services.iter().any(|each| each.id == service.id) {
                        tracing::warn!(
                            provider = id.as_str(),
                            "the speech service {} is already offered",
                            service.id
                        );
                    } else {
                        services.push(service);
                    }
                }
            }
        }
        Self { services }
    }

    /// Exactly these services, as a test that stands in for the libraries has them.
    #[cfg(test)]
    pub(crate) fn from_services(services: Vec<AccountSpeechService>) -> Self {
        Self { services }
    }

    /// The service called `id`.
    pub(crate) fn find(&self, id: &str) -> Option<&AccountSpeechService> {
        self.services.iter().find(|service| service.id == id)
    }

    /// The models of the service a recording is posted to at `url`: none for an address no library describes.
    pub(crate) fn models_at(&self, url: &str) -> Vec<Value> {
        self.services
            .iter()
            .filter(|service| same(&service.url, url))
            .flat_map(|service| service.models.iter())
            .map(|model| json!({"id": model.id, "label": model.label}))
            .collect()
    }

    /// Each service `account` can be offered, and whether it can be used: `Ok` for a key that may call it and the reason for an
    /// account that is not one. A service of which the account's provider has no kind at all is not listed.
    pub(crate) fn offered(
        &self,
        kinds: &Catalogs<'_>,
        account: &AccountConfig,
    ) -> Vec<(&AccountSpeechService, Result<(), String>)> {
        self.services
            .iter()
            .filter_map(|service| Some((service, verdict(service, kinds, account)?)))
            .collect()
    }
}

/// Whether two addresses are the same, whatever slash ends them.
fn same(one: &str, other: &str) -> bool {
    one.trim().trim_end_matches('/') == other.trim().trim_end_matches('/')
}

/// The slot of `kind` that says where its model API is called.
fn model_api(kind: &AccountKindInfo) -> Option<&EndpointSlot> {
    kind.endpoints.iter().find(|slot| slot.purpose == MODEL_API)
}

/// Whether `kind` is a key for `service`: it provides the service's standard and its model API starts from the service's address.
fn is_key_for(service: &AccountSpeechService, kind: &AccountKindInfo) -> bool {
    kind.provides.contains(&service.standard)
        && model_api(kind)
            .and_then(|slot| slot.default.as_deref())
            .is_some_and(|address| same(address, &service.address))
}

/// What an account's kind says of a service: `None` for an account of a provider none of whose kinds is a key for it, `Ok` for a key
/// that may call it and the reason for an account that is not one.
fn verdict(
    service: &AccountSpeechService,
    kinds: &Catalogs<'_>,
    account: &AccountConfig,
) -> Option<Result<(), String>> {
    let listing = kinds.listing(&account.harness)?;
    listing
        .kinds
        .iter()
        .find(|kind| is_key_for(service, kind))?;
    let Some(kind) = listing
        .kind(&account.kind)
        .filter(|kind| is_key_for(service, kind))
    else {
        return Some(Err(format!(
            "an account of kind {} is not an API key for {}, and Canoa has no measured way to send audio to {} with it, so it is not offered: add an API key as an account to use {}'s speech-to-text",
            account.kind, service.title, service.title, service.title
        )));
    };
    let moved = model_api(kind)
        .and_then(|slot| slot.key.as_deref())
        .and_then(|key| account.settings.as_ref()?.as_map().get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty() && !same(address, &service.address));
    Some(match moved {
        Some(address) => Err(format!(
            "its key is for {}, not for {}, and Canoa has not measured that address's speech-to-text, so it is not offered",
            address.trim_end_matches('/'),
            service.address.trim_end_matches('/')
        )),
        None => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services() -> Services {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        crate::bundled::register_included(&mut registry).expect("the carried libraries");
        Services::from_registries(&[registry])
    }

    #[test]
    fn the_models_of_a_service_are_listed_for_the_address_it_takes_a_recording_at_and_for_no_other()
    {
        let all = services();
        let described = all.services.first().expect("a carried service").clone();
        let rows = all.models_at(&described.url);
        assert_eq!(rows.len(), described.models.len());
        assert_eq!(rows[0]["id"], described.models[0].id);
        assert_eq!(
            all.models_at(&format!("{}/", described.url)),
            rows,
            "a slash at the end is the same address"
        );
        assert!(
            all.models_at("https://example.test/v1/audio/transcriptions")
                .is_empty()
        );
        assert!(all.models_at("").is_empty());
    }

    #[test]
    fn a_service_a_library_describes_once_is_not_taken_by_a_second_of_the_same_id() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        crate::bundled::register_included(&mut registry).expect("the carried libraries");
        let mut again = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        crate::bundled::register_included(&mut again).expect("the carried libraries");
        let twice = Services::from_registries(&[registry, again]);
        assert_eq!(twice.services.len(), services().services.len());
    }
}
