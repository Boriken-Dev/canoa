//! The backend made of an `sdk_provider`: one for each of the person's accounts a provider serves (`openai:work`), and one for each
//! server the person offers by address (`[sdk.NAME]`).
//!
//! The host holds none of a vendor's address, models or request shapes: the provider's description says which accounts it serves and
//! how the key is applied, and the account's own model API address is where the calls go. The key is read from the account store (or
//! from the variable a table names) when a request is made, put on the request by [`HostHttp`] and never given to the provider.

use alloc::sync::Arc;
use core::time::Duration;
use std::collections::BTreeSet;
use std::path::PathBuf;

use canoa::network::Network;
use canoa::sdk::{LentHttp as _, OpenRequest, SIGNED_OUT_PREFIX, SdkInfo, SdkProvider, Server};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountKindInfo, EndpointSlot, MODEL_API, ModelListing};

use super::http::HostHttp;
use super::{Answer, Backend, Call, Failure, Operation};
use crate::accounts::matching::Catalogs;
use crate::config::{AccountConfig, Config, Layers, SdkTable};

/// Where a request's key comes from.
#[derive(Clone)]
enum KeySource {
    /// The server asks for none.
    None,
    /// The variable a table names.
    Env(String),
    /// A credential of an account the host keeps.
    Account {
        layers: Layers,
        home: PathBuf,
        name: String,
        credential: String,
    },
}

/// Which provider answers.
#[derive(Clone)]
enum Source {
    /// The one the host carries.
    Server,
    /// One a loaded library offers.
    Library(Arc<Registry>, String),
}

/// One offer: a provider over an address, with a key.
pub(super) struct SdkBackend {
    id: String,
    title: String,
    location: String,
    info: SdkInfo,
    source: Source,
    address: String,
    key: KeySource,
    settings: String,
    /// What the offer does: the provider's operations, narrowed by what the person said the server does.
    can: BTreeSet<String>,
}

impl core::fmt::Debug for SdkBackend {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SdkBackend")
            .field("id", &self.id)
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl SdkBackend {
    fn with_provider<T>(&self, f: impl FnOnce(&dyn SdkProvider) -> T) -> Result<T, Failure> {
        match &self.source {
            Source::Server => Ok(f(&Server)),
            Source::Library(registry, id) => {
                let offer = registry
                    .offers::<dyn SdkProvider>()
                    .find(|offer| offer.id() == id)
                    .ok_or_else(|| {
                        Failure::new(format!("the provider {id} is no longer loaded"))
                    })?;
                let instance = offer
                    .default_instance()
                    .ok_or_else(|| Failure::new(format!("the provider {id} cannot be built")))?;
                Ok(f(instance))
            }
        }
    }

    fn key(&self) -> Result<Option<String>, Failure> {
        match &self.key {
            KeySource::None => Ok(None),
            KeySource::Env(name) => match std::env::var(name) {
                Ok(key) if !key.is_empty() => Ok(Some(key)),
                _ => Err(Failure::new(format!("the variable {name} is not set"))),
            },
            KeySource::Account {
                layers,
                home,
                name,
                credential,
            } => {
                let config = Config::load_layers(layers)
                    .map_err(|error| Failure::new(format!("canoa.cfg: {error}")))?;
                crate::control::speech_accounts::read_credential(&config, home, name, credential)
                    .map(Some)
                    .map_err(|why| Failure {
                        why,
                        signed_out: true,
                    })
            }
        }
    }

    fn open(
        &self,
        network: &Arc<Network>,
        timeout: Duration,
    ) -> Result<coqui_acp_api::guatiao::library::Object<dyn canoa::sdk::Sdk>, Failure> {
        let key = self.key()?;
        let http = HostHttp::new(
            &self.address,
            self.info.auth.clone(),
            key,
            Arc::clone(network),
            timeout,
        );
        let request = OpenRequest::new(&self.address, &self.settings);
        self.with_provider(|provider| provider.open(request, http.into_object()))?
            .map_err(failure)
    }
}

fn failure(error: coqui_acp_api::guatiao::library::ProviderError) -> Failure {
    let why = error.to_string();
    Failure {
        signed_out: why.contains(SIGNED_OUT_PREFIX),
        why,
    }
}

impl Backend for SdkBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn location(&self) -> &str {
        &self.location
    }

    fn serves(&self, operation: Operation) -> bool {
        self.can.contains(operation.capability())
    }

    fn serve(
        &self,
        network: &Arc<Network>,
        call: Call,
        timeout: Duration,
    ) -> Result<Answer, Failure> {
        let sdk = self.open(network, timeout)?;
        match call {
            Call::Complete(request) => sdk.chat(request).map(Answer::Complete),
            Call::Embed(request) => sdk.embed(request).map(Answer::Embed),
            Call::Rerank(request) => sdk.rerank(request).map(Answer::Rerank),
        }
        .map_err(failure)
    }

    fn models(&self, network: &Arc<Network>) -> Result<ModelListing, Failure> {
        self.open(network, Duration::from_secs(30))?
            .models()
            .map_err(failure)
    }
}

/// The slot of `kind` that says where its model API is called.
fn model_api(kind: &AccountKindInfo) -> Option<&EndpointSlot> {
    kind.endpoints.iter().find(|slot| slot.purpose == MODEL_API)
}

/// Where `account`'s model API is called: the setting the kind's slot names, else the slot's default.
fn address_of(kind: &AccountKindInfo, account: &AccountConfig) -> Option<String> {
    let slot = model_api(kind)?;
    slot.key
        .as_deref()
        .and_then(|key| account.settings.as_ref()?.as_map().get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(slot.default.as_deref())
        .map(|address| address.trim_end_matches('/').to_owned())
}

fn same(one: &str, other: &str) -> bool {
    one.trim().trim_end_matches('/') == other.trim().trim_end_matches('/')
}

/// The providers the libraries describe.
fn providers(registries: &[Arc<Registry>]) -> Vec<(SdkInfo, Source)> {
    let mut found: Vec<(SdkInfo, Source)> = vec![];
    if let Ok(info) = Server.describe() {
        found.push((info, Source::Server));
    }
    for registry in registries {
        for offer in registry.offers::<dyn SdkProvider>() {
            let id = offer.id().to_owned();
            let Some(instance) = offer.default_instance() else {
                tracing::warn!(provider = id.as_str(), "its sdk provider cannot be built");
                continue;
            };
            let Ok(info) = instance.describe() else {
                tracing::warn!(
                    provider = id.as_str(),
                    "its sdk provider cannot be described"
                );
                continue;
            };
            if let Some(problem) = info.problem() {
                tracing::warn!(
                    provider = id.as_str(),
                    "its sdk provider is not offered: {problem}"
                );
                continue;
            }
            if found.iter().any(|(each, _)| each.id == info.id) {
                tracing::warn!(
                    provider = id.as_str(),
                    "the sdk provider {} is already offered",
                    info.id
                );
                continue;
            }
            found.push((info, Source::Library(Arc::clone(registry), id)));
        }
    }
    found
}

/// Every backend the person's accounts and `[sdk.NAME]` tables make, with the providers `registries` carry. `layers` and `home` say
/// where the accounts' keys are read from; with none, no account is offered.
pub(crate) fn backends_of(
    config: &Config,
    registries: &[Arc<Registry>],
    accounts: Option<(&Layers, &std::path::Path)>,
) -> Vec<Arc<dyn Backend>> {
    let providers = providers(registries);
    let mut made: Vec<Arc<dyn Backend>> = Vec::new();
    if let Some((layers, home)) = accounts {
        let kinds = Catalogs::new(config);
        for (name, account) in &config.accounts {
            let beside = kinds.kinds_beside(&account.kind);
            let Some(kind) = beside.iter().find(|kind| kind.id == account.kind) else {
                continue;
            };
            for (info, source) in &providers {
                if !info
                    .standards
                    .iter()
                    .any(|standard| kind.provides.contains(standard))
                {
                    continue;
                }
                let Some(address) = address_of(kind, account) else {
                    continue;
                };
                if info
                    .address
                    .as_deref()
                    .is_some_and(|pinned| !same(pinned, &address))
                {
                    tracing::info!(
                        "the account {name} is not offered to {}: its key is for {address}, which the provider is not measured at",
                        info.id
                    );
                    continue;
                }
                made.push(Arc::new(SdkBackend {
                    id: format!("{}:{name}", info.id),
                    title: format!("{}, account {name}", info.title),
                    location: info.location.clone(),
                    can: info
                        .capabilities
                        .iter()
                        .map(|each| each.id.clone())
                        .collect(),
                    info: info.clone(),
                    source: source.clone(),
                    address,
                    key: KeySource::Account {
                        layers: layers.clone(),
                        home: home.to_path_buf(),
                        name: name.clone(),
                        credential: info.credential.clone(),
                    },
                    settings: "{}".to_owned(),
                }));
            }
        }
    }
    for (name, table) in &config.assist.sdk {
        match table_backend(name, table, &providers) {
            Ok(backend) => made.push(Arc::new(backend)),
            Err(why) => tracing::warn!("[sdk.{name}] is not offered: {why}"),
        }
    }
    made
}

fn table_backend(
    name: &str,
    table: &SdkTable,
    providers: &[(SdkInfo, Source)],
) -> Result<SdkBackend, String> {
    let (info, source) = providers
        .iter()
        .find(|(info, _)| info.id == table.provider)
        .ok_or_else(|| format!("no provider {} is loaded", table.provider))?;
    if !info.standards.is_empty() {
        return Err(format!(
            "{} serves the accounts of a standard, not an address",
            info.id
        ));
    }
    let address = table
        .address
        .clone()
        .ok_or_else(|| "it names no address".to_owned())?;
    let declared: Option<BTreeSet<String>> = table
        .settings
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|each| each.as_str().map(str::to_owned))
                .collect()
        });
    let mut can: BTreeSet<String> = info
        .capabilities
        .iter()
        .map(|each| each.id.clone())
        .collect();
    if let Some(declared) = &declared {
        can.retain(|each| declared.contains(each));
    }
    Ok(SdkBackend {
        id: name.to_owned(),
        title: format!("{} at {address}", info.title),
        location: info.location.clone(),
        info: info.clone(),
        source: source.clone(),
        address,
        key: table
            .key_env
            .clone()
            .map_or(KeySource::None, KeySource::Env),
        settings: table.settings.to_string(),
        can,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::Assist;
    use crate::assist::tests_support::registries;

    /// The carried libraries describe an OpenAI provider for the accounts of `openai-api`, and none of it is the host's.
    #[test]
    fn the_carried_openai_provider_is_described_for_the_accounts_of_its_standard() {
        let found = providers(&registries());
        let (info, _) = found
            .iter()
            .find(|(info, _)| info.id == "openai")
            .expect("openai");
        assert_eq!(info.standards, ["openai-api"]);
        assert!(info.can("chat") && info.can("embed") && !info.can("rerank"));
        assert!(found.iter().any(|(info, _)| info.id == "server"));
    }

    #[test]
    fn a_table_names_a_server_and_narrows_what_it_does_to_what_the_person_said() {
        let config = crate::Config::from_str_in(
            "[sdk.local]\nprovider = \"server\"\naddress = \"http://127.0.0.1:60570\"\nkey_env = \"LOCAL_KEY\"\ncapabilities = [\"embed\"]\n\
             [sdk.nowhere]\nprovider = \"no-such-provider\"\naddress = \"http://127.0.0.1:1\"\n\
             [sdk.bare]\nprovider = \"server\"\n\
             [sdk.account]\nprovider = \"openai\"\naddress = \"http://127.0.0.1:2\"\n",
            std::path::Path::new("."),
        )
        .expect("a configuration");
        let backends = backends_of(&config, &registries(), None);
        let ids: Vec<&str> = backends.iter().map(|each| each.id()).collect();
        assert_eq!(
            ids,
            ["local"],
            "a table with no provider, no address or an account's provider is not offered"
        );
        assert!(backends[0].serves(Operation::Embed));
        assert!(!backends[0].serves(Operation::Complete));
        assert!(!backends[0].serves(Operation::Rerank));
        let service = Assist::new(
            backends,
            indexmap::IndexMap::new(),
            core::time::Duration::from_secs(1),
        );
        assert_eq!(
            service.first(Operation::Embed, "default").as_deref(),
            Some("local")
        );
    }

    /// One real call to OpenAI through the OpenAI provider, over the host's own HTTP, with the key from the variable.
    #[test]
    #[ignore = "needs a real OpenAI API key in CANOA_TEST_OPENAI_KEY and spends a few tokens"]
    fn a_real_openai_key_lists_models_with_their_capabilities_and_embeds() {
        let found = providers(&registries());
        let (info, source) = found
            .into_iter()
            .find(|(info, _)| info.id == "openai")
            .expect("openai");
        let backend = SdkBackend {
            id: "openai:real".to_owned(),
            title: info.title.clone(),
            location: info.location.clone(),
            can: info
                .capabilities
                .iter()
                .map(|each| each.id.clone())
                .collect(),
            info,
            source,
            address: "https://api.openai.com/v1".to_owned(),
            key: KeySource::Env("CANOA_TEST_OPENAI_KEY".to_owned()),
            settings: "{}".to_owned(),
        };
        let network = Network::current();
        let models = backend.models(&network).expect("listed").models;
        let embed = canoa::sdk::models_for(&models, "embed");
        assert!(!embed.offered.is_empty(), "{models:?}");
        let answer = backend
            .serve(
                &network,
                Call::Embed(canoa::sdk::EmbedRequest::new(
                    &embed.offered[0].id,
                    vec!["a canoe on a river".to_owned()],
                )),
                Duration::from_secs(30),
            )
            .expect("embedded");
        assert!(matches!(answer, Answer::Embed(reply) if reply.vectors.len() == 1));
    }
}
