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

use core::borrow::Borrow;
use std::path::Path;
use std::sync::Arc;

use canoa::speech::account::{AccountSpeech, AccountSpeechService};
use canoa::speech::output::account::{AccountSpeechOutput, AccountSpeechOutputService};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountKindInfo, EndpointSlot, MODEL_API};
use serde_json::{Value, json};

use crate::accounts::matching::Catalogs;
use crate::config::{AccountConfig, Config};

/// Collects described services from `registries` through `AccountSpeech` or `AccountSpeechOutput`.
fn collect_services<K, T, F>(
    registries: &[impl Borrow<Registry>],
    mut extract: F,
    id_of: fn(&T) -> &str,
    label: &str,
) -> Vec<T>
where
    K: ?Sized + coqui_acp_api::guatiao::library::Kind + 'static,
    F: FnMut(&K) -> Option<(Vec<T>, Option<String>)>,
{
    let mut services: Vec<T> = Vec::new();
    for registry in registries {
        for offer in registry.borrow().offers::<K>() {
            let id = offer.id().to_owned();
            let Some(instance) = offer.default_instance() else {
                tracing::warn!(
                    provider = id.as_str(),
                    "its speech {label} services cannot be built"
                );
                continue;
            };
            let Some((items, problem)) = extract(instance) else {
                tracing::warn!(
                    provider = id.as_str(),
                    "its speech {label} services cannot be described"
                );
                continue;
            };
            if let Some(problem) = problem {
                tracing::warn!(
                    provider = id.as_str(),
                    "its speech {label} services are not offered: {problem}"
                );
                continue;
            }
            for service in items {
                let sid = id_of(&service).to_owned();
                if services.iter().any(|each| id_of(each) == sid) {
                    tracing::warn!(
                        provider = id.as_str(),
                        "the speech service {sid} is already offered"
                    );
                } else {
                    services.push(service);
                }
            }
        }
    }
    services
}

/// The services the libraries this host loads describe.
#[derive(Debug, Default, Clone)]
pub(crate) struct Services {
    services: Vec<AccountSpeechService>,
}

impl Services {
    /// The services of the libraries this build carries and, when the host has a canoa.cfg, of the ones its entries and extensions
    /// load.
    pub(crate) fn of(config: Option<&Config>) -> Self {
        Self::from_registries(&registries(config))
    }

    /// The services `registries` offer, in the order they are given. A service that cannot be offered, or whose id an earlier one took,
    /// is left out and logged.
    pub(crate) fn from_registries(registries: &[impl Borrow<Registry>]) -> Self {
        let services = collect_services::<dyn AccountSpeech, _, _>(
            registries,
            |provider| {
                let info = provider.describe().ok()?;
                let problem = info.problem();
                Some((info.services, problem))
            },
            |service| &service.id,
            "recognition",
        );
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
            .filter_map(|service| {
                Some((
                    service,
                    verdict_for(
                        &service.standard,
                        &service.address,
                        &service.title,
                        Direction::Dictation,
                        kinds,
                        account,
                    )?,
                ))
            })
            .collect()
    }
}

/// The text-to-speech services the libraries this host loads describe (`account_speech_output`).
#[derive(Debug, Default, Clone)]
pub(crate) struct OutputServices {
    services: Vec<AccountSpeechOutputService>,
}

impl OutputServices {
    /// The services of the libraries this build carries and, when canoa.cfg names extensions, of those too.
    pub(crate) fn of(config: Option<&Config>) -> Self {
        Self::from_registries(&registries(config))
    }

    /// The services `registries` offer, in the order they are given.
    pub(crate) fn from_registries(registries: &[impl Borrow<Registry>]) -> Self {
        let services = collect_services::<dyn AccountSpeechOutput, _, _>(
            registries,
            |provider| {
                let info = provider.describe().ok()?;
                let problem = info.problem();
                Some((info.services, problem))
            },
            |service| &service.id,
            "output",
        );
        Self { services }
    }

    /// Exactly these services, as a test stands in for the libraries.
    #[cfg(test)]
    pub(crate) fn from_services(services: Vec<AccountSpeechOutputService>) -> Self {
        Self { services }
    }

    /// The service called `id`.
    pub(crate) fn find(&self, id: &str) -> Option<&AccountSpeechOutputService> {
        self.services.iter().find(|service| service.id == id)
    }

    /// Each service `account` can be offered for text-to-speech, using the same standard and address matching rule as dictation.
    pub(crate) fn offered(
        &self,
        kinds: &Catalogs<'_>,
        account: &AccountConfig,
    ) -> Vec<(&AccountSpeechOutputService, Result<(), String>)> {
        self.services
            .iter()
            .filter_map(|service| {
                Some((
                    service,
                    verdict_for(
                        &service.standard,
                        &service.address,
                        &service.title,
                        Direction::Speaking,
                        kinds,
                        account,
                    )?,
                ))
            })
            .collect()
    }
}

/// Reads `credential` of `account_name` from `home`'s credential store without copying it anywhere else.
pub(crate) fn read_credential(
    config: &Config,
    home: &Path,
    account_name: &str,
    credential: &str,
) -> Result<String, String> {
    let account = config
        .accounts
        .get(account_name)
        .ok_or_else(|| format!("the account {account_name} is gone: add it again"))?;
    let store = crate::accounts::Accounts::open(config, home).map_err(|error| {
        tracing::warn!("the account store could not be opened for speech: {error}");
        "the host's account store could not be used".to_owned()
    })?;
    let record = store
        .load(&account.id)
        .map_err(|error| {
            tracing::warn!("an account could not be read for speech: {error}");
            "the host's account store could not be used".to_owned()
        })?
        .ok_or_else(|| format!("the account {account_name} holds no key: sign in to it again"))?;
    record
        .credentials
        .get(credential)
        .filter(|key| !key.is_empty())
        .cloned()
        .ok_or_else(|| format!("the account {account_name} holds no key: sign in to it again"))
}

/// Whether two addresses are the same, whatever slash ends them.
fn same(one: &str, other: &str) -> bool {
    one.trim().trim_end_matches('/') == other.trim().trim_end_matches('/')
}

/// The slot of `kind` that says where its model API is called.
fn model_api(kind: &AccountKindInfo) -> Option<&EndpointSlot> {
    kind.endpoints.iter().find(|slot| slot.purpose == MODEL_API)
}

/// Whether `kind` is a key for `(standard, address)`.
fn is_key_for(standard: &str, address: &str, kind: &AccountKindInfo) -> bool {
    kind.provides.iter().any(|each| each == standard)
        && model_api(kind)
            .and_then(|slot| slot.default.as_deref())
            .is_some_and(|default| same(default, address))
}

/// Which way a service carries speech, for the words of a refusal.
#[derive(Clone, Copy)]
enum Direction {
    /// Audio goes to the service and text comes back.
    Dictation,
    /// Text goes to the service and audio comes back.
    Speaking,
}

/// What an account's kind says of a service described by `(standard, address, title)`.
fn verdict_for(
    standard: &str,
    address: &str,
    title: &str,
    direction: Direction,
    kinds: &Catalogs<'_>,
    account: &AccountConfig,
) -> Option<Result<(), String>> {
    let beside = kinds.kinds_beside(&account.kind);
    beside
        .iter()
        .find(|kind| is_key_for(standard, address, kind))?;
    let Some(kind) = beside
        .iter()
        .find(|kind| kind.id == account.kind)
        .filter(|kind| is_key_for(standard, address, kind))
    else {
        let (what, how) = match direction {
            Direction::Dictation => ("send audio to", format!("{title}'s speech-to-text")),
            Direction::Speaking => ("speak through", format!("{title}'s voices")),
        };
        return Some(Err(format!(
            "an account of kind {} is not an API key for {title}, and Canoa has no measured way to {what} {title} with it, so it is not offered: add an API key as an account to use {how}",
            account.kind
        )));
    };
    let what = match direction {
        Direction::Dictation => "speech-to-text",
        Direction::Speaking => "text-to-speech",
    };
    let moved = model_api(kind)
        .and_then(|slot| slot.key.as_deref())
        .and_then(|key| account.settings.as_ref()?.as_map().get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !same(value, address));
    Some(match moved {
        Some(other) => Err(format!(
            "its key is for {}, not for {}, and Canoa has not measured that address's {what}, so it is not offered",
            other.trim_end_matches('/'),
            address.trim_end_matches('/')
        )),
        None => Ok(()),
    })
}

/// The registries the services are read from, each library once: with a canoa.cfg, those `providers/list` opens (the carried
/// libraries among them); without one, the carried libraries alone.
fn registries(config: Option<&Config>) -> Vec<Arc<Registry>> {
    if let Some(config) = config {
        return super::providers::loaded(config)
            .iter()
            .map(|(_, registry)| Arc::clone(registry))
            .collect();
    }
    let mut carried = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    if let Err(why) = crate::bundled::register_included(&mut carried) {
        tracing::warn!("the libraries this build carries are not read for speech: {why}");
    }
    vec![Arc::new(carried)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many speech providers `registries` offer, counting a library each time it appears.
    fn offered(registries: &[impl Borrow<Registry>]) -> usize {
        registries
            .iter()
            .map(|registry| registry.borrow().offers::<dyn AccountSpeech>().count())
            .sum()
    }

    #[test]
    fn a_host_with_a_canoa_cfg_reads_each_carried_library_once() {
        let config = Config::from_str_in("", &std::env::temp_dir()).expect("an empty canoa.cfg");
        let alone = offered(&registries(None));
        assert!(alone > 0, "the build carries a speech provider");
        assert_eq!(offered(&registries(Some(&config))), alone);
    }

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
