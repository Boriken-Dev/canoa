//! The host's end of the `acp_harness` kind: the providers an entry can offer by id, the instance an entry configures, and
//! the request a connection starts with. `claude`, `codex` and `pi` are compiled into this binary and registered in this process;
//! an outside provider is loaded from the library or source an entry names, into a registry of its own, so one entry never
//! resolves an id that only another entry's file offers. Both are chosen and started the same way.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environments and paths are std's"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::Path;

use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::InstantiateError;
use coqui_acp_api::guatiao::schema::{SchemaRef, differences, inline_refs};
use coqui_acp_api::{
    AccountCatalog, AccountKeeper, AccountKindInfo, AcpHarness, CacheLifetime, Environment,
    ProcessSpawner, StartRequest,
};
use coqui_acp_claude::ClaudeProvider;
use coqui_acp_codex::CodexProvider;
use coqui_acp_pi::PiProvider;

use crate::accounts::{ACCOUNT_META, Accounts, Held, NoAccounts};
use crate::config::{AccountConfig, Origin, Settings, cache_lifetime_named};
use crate::provider::{ProviderError, map_of, open};

// What `Registry::register_local` takes: the three built-in providers, linked into this binary.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-built-in",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ClaudeProvider, CodexProvider, PiProvider]
);

/// The `_meta` key of a client's `initialize` that says how long its session's prompt cache should live: an object
/// `{"lifetime": "default" | "short" | "long", "chosen": bool}`. `chosen` (true when absent) says the person chose it for
/// this session, which wins over the entry's `cache_lifetime`; false is the client's default for its kind of session, which
/// the entry's setting replaces.
pub const CACHE_META: &str = "dev.boriken.canoa/cache_lifetime";

/// The setting through which a profiled connection lets a client's plugin reach the harness: Claude Code's provider has it.
const CLIENT_PLUGINS: &str = "allow_client_plugins";

/// A registry holding what one entry can choose from: the built-in providers when the entry has no origin, and only what
/// its origin loads when it has one, so an id is resolved from that entry's own file and no other.
///
/// # Errors
///
/// [`ProviderError::Load`] or [`ProviderError::Source`] for the built-in providers or an origin that does not load.
pub(crate) fn registry(origin: Option<&Origin>) -> Result<Registry, ProviderError> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    match origin {
        Some(origin) => open(&mut registry, origin)?,
        None => {
            let loading = registry
                .register_local("canoa", library)
                .map_err(|error| ProviderError::Load(error.to_string()))?;
            if loading.loaded().is_none() {
                return Err(ProviderError::Load(
                    "the built-in harness providers were not accepted".to_owned(),
                ));
            }
        }
    }
    Ok(registry)
}

/// One setting of an instance that differs from the provider's default, for the log: its key, value and default.
pub(crate) type Departure = (String, String, String);

/// One entry's provider, configured: what each of its connections starts from.
pub(crate) struct Harness {
    plain: Arc<dyn AcpHarness>,
    /// The same configuration with client plugins allowed, for a connection that has a profile; `None` when the provider
    /// has no such setting.
    profiled: Option<Arc<dyn AcpHarness>>,
    roots: Vec<String>,
    departures: Vec<Departure>,
    /// What the provider says of its accounts; `None` when it could not say.
    catalog: Option<AccountCatalog>,
    /// The accounts the entry offers, when it offers any.
    accounts: Option<Offer>,
    /// The entry's `cache_lifetime`: what its sessions ask when the client does not choose.
    lifetime: Option<CacheLifetime>,
    /// Dropped after the instances, which point into what it loaded.
    _registry: Arc<Registry>,
}

/// An account an entry offers: its name, its configuration and its kind in the provider's catalog.
pub(crate) type Offered = (String, AccountConfig, AccountKindInfo);

/// The accounts an entry offers, in its order.
struct Offer {
    host: Arc<Accounts>,
    offered: Vec<Offered>,
    /// Number the plain and the profiled instance's sequences, so one's are not taken for the other's.
    sources: (u64, u64),
}

/// What one connection runs on: the account it chose, and what the provider is handed for it.
pub(crate) struct Connection {
    pub(crate) harness: Arc<dyn AcpHarness>,
    pub(crate) request: StartRequest,
    pub(crate) spawner: Object<dyn ProcessSpawner>,
    pub(crate) keeper: Object<dyn AccountKeeper>,
}

impl fmt::Debug for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Harness")
            .field("profiled", &self.profiled.is_some())
            .field("roots", &self.roots)
            .finish_non_exhaustive()
    }
}

impl Harness {
    /// The provider `id` of `registry`, configured with `settings`; with `profiles`, also the same with client plugins
    /// allowed. `roots` are the entry's working roots, which every connection's provider holds a session and a resumed
    /// thread to.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Missing`] for an id nothing offers, [`ProviderError::WrongKind`] for one that is not an
    /// `acp_harness`, and [`ProviderError::Settings`] and its kin for a configuration it refuses, the profiled one included.
    pub(crate) fn new(
        registry: &Arc<Registry>,
        id: &str,
        settings: &Settings,
        profiles: bool,
        roots: Vec<String>,
    ) -> Result<Self, ProviderError> {
        let (plain, departures) =
            instance(registry, id, settings).map_err(|error| match error {
                ProviderError::Schema(why) => ProviderError::Settings(why),
                other => other,
            })?;
        let catalog = plain.accounts().ok();
        let profiled = if profiles {
            let mut map = settings.as_map().clone();
            map.insert(CLIENT_PLUGINS.to_owned(), serde_json::Value::Bool(true));
            match instance(registry, id, &Settings::new(map)) {
                Ok((profiled, _)) => Some(profiled),
                // A provider with no such setting takes no client plugin, which is the point of the switch.
                Err(ProviderError::Schema(_)) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        Ok(Self {
            plain,
            profiled,
            roots,
            departures,
            catalog,
            accounts: None,
            lifetime: None,
            _registry: Arc::clone(registry),
        })
    }

    /// This harness asking `lifetime` of its provider for the sessions whose client chooses none.
    pub(crate) const fn with_lifetime(mut self, lifetime: Option<CacheLifetime>) -> Self {
        self.lifetime = lifetime;
        self
    }

    /// The prompt-cache lifetime a connection asks, from its client's first message and the entry's setting: the client's
    /// choice for this session, else the entry's, else what the client asks of its kind of session, else the provider's own.
    ///
    /// # Errors
    ///
    /// Why the client's value is refused, for the client.
    pub(crate) fn lifetime_of(&self, first: &[u8]) -> Result<CacheLifetime, String> {
        let meta = serde_json::from_slice::<serde_json::Value>(first)
            .ok()
            .and_then(|message| {
                message
                    .get("params")?
                    .get("_meta")?
                    .get(CACHE_META)
                    .cloned()
            });
        let Some(meta) = meta else {
            return Ok(self.lifetime.unwrap_or_default());
        };
        let asked = meta
            .get("lifetime")
            .and_then(serde_json::Value::as_str)
            .and_then(cache_lifetime_named)
            .ok_or_else(|| "the cache lifetime is default, short or long".to_owned())?;
        let chosen = match meta.get("chosen") {
            None => true,
            Some(serde_json::Value::Bool(chosen)) => *chosen,
            Some(_) => return Err("chosen is true or false".to_owned()),
        };
        Ok(if chosen {
            asked
        } else {
            self.lifetime.unwrap_or(asked)
        })
    }

    /// What the provider says of its accounts and sign-ins.
    pub(crate) const fn catalog(&self) -> Option<&AccountCatalog> {
        self.catalog.as_ref()
    }

    /// The provider as configured, for the commands that sign in.
    pub(crate) fn provider(&self) -> &Arc<dyn AcpHarness> {
        &self.plain
    }

    /// This harness offering `offered`, each by name with its configuration and its kind in the catalog, kept in `host`.
    pub(crate) fn offering(mut self, host: &Arc<Accounts>, offered: Vec<Offered>) -> Self {
        self.accounts = Some(Offer {
            host: Arc::clone(host),
            offered,
            sources: (host.source(), host.source()),
        });
        self
    }

    /// The account the client's first message chooses for its connection, held for it: the one it names in `_meta`, else the
    /// entry's first; `None` for an entry that offers none. `holder` says who is holding it.
    ///
    /// # Errors
    ///
    /// Why the choice is refused, for the client.
    pub(crate) fn choose(&self, first: &[u8], holder: &str) -> Result<Option<Held>, String> {
        let Some(offer) = &self.accounts else {
            return Ok(None);
        };
        let named = serde_json::from_slice::<serde_json::Value>(first)
            .ok()
            .and_then(|message| {
                message
                    .get("params")?
                    .get("_meta")?
                    .get(ACCOUNT_META)
                    .cloned()
            });
        let (name, config, info) = match named {
            None => offer.offered.first(),
            Some(serde_json::Value::String(name)) => {
                offer.offered.iter().find(|(offered, ..)| *offered == name)
            }
            Some(_) => return Err("the account is named by its name, as text".to_owned()),
        }
        .ok_or_else(|| "this entry does not offer that account".to_owned())?;
        let held = offer
            .host
            .begin(name, config, info, holder)
            .map_err(|refusal| refusal.client_reason())?;
        // What using the kind weakens is said at every start, and so is a credential the host's own environment sets that
        // the account takes the place of.
        for notice in &info.notices {
            tracing::info!("account {name}: {notice}");
        }
        if config.kind != coqui_acp_api::HARNESS_LOGIN
            && let Some(catalog) = &self.catalog
        {
            let set: Vec<&str> = catalog
                .credential_variables
                .iter()
                .filter(|variable| std::env::var_os(variable).is_some())
                .map(String::as_str)
                .collect();
            if !set.is_empty() {
                tracing::info!(
                    "non-default: the host's environment sets {}, which account {name} takes the place of for its sessions",
                    set.join(", ")
                );
            }
        }
        Ok(Some(held))
    }

    /// The instance a connection runs on, and the request it starts with. `environment` and `skills` are a profile's: the
    /// environment its program is given, and the directory its skills are written under, which the provider may work in
    /// beside the entry's roots. An entry with no roots takes any directory, and so does the connection.
    pub(crate) fn connection(
        &self,
        profile: Option<(Environment, &Path)>,
        account: Option<&Held>,
        lifetime: CacheLifetime,
    ) -> Connection {
        let mut request = StartRequest::builder().cache_lifetime(lifetime);
        for root in &self.roots {
            request = request.working_root(root.clone());
        }
        let mut source = self.accounts.as_ref().map_or(0, |offer| offer.sources.0);
        let harness = match profile {
            Some((environment, skills)) => {
                if self.profiled.is_some() {
                    source = self.accounts.as_ref().map_or(0, |offer| offer.sources.1);
                }
                request = request.environment(environment);
                if !self.roots.is_empty() {
                    request = request.working_root(skills.to_string_lossy().into_owned());
                }
                Arc::clone(self.profiled.as_ref().unwrap_or(&self.plain))
            }
            None => Arc::clone(&self.plain),
        };
        let (spawner, keeper) = match (account, &self.accounts) {
            (Some(held), Some(_)) => {
                request = request.account(held.grant());
                (held.spawner(), held.keeper(source))
            }
            _ => (
                coqui_acp_harness::LocalSpawner::new().into_object(),
                NoAccounts.into_object(),
            ),
        };
        Connection {
            harness,
            request: request.build(),
            spawner,
            keeper,
        }
    }

    /// How the configured instance departs from the provider's defaults: what its schema shows, then what the provider
    /// itself says of rules a setting holds as text.
    pub(crate) fn non_defaults(&self) -> Vec<Departure> {
        let mut all = self.departures.clone();
        all.extend(
            self.plain
                .non_defaults()
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.setting, d.value, d.default)),
        );
        all
    }
}

/// An instance of provider `id`, from the entry's `settings`, which the provider checks against its schema; and where the
/// settings depart from the schema's defaults.
fn instance(
    registry: &Registry,
    id: &str,
    settings: &Settings,
) -> Result<(Arc<dyn AcpHarness>, Vec<Departure>), ProviderError> {
    let offer = registry
        .offer::<dyn AcpHarness>(id)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    if offer.builds_instances() {
        let config = Value::from(map_of(settings).map_err(ProviderError::Settings)?);
        let built = offer.instantiate(&config).map_err(|error| match error {
            InstantiateError::Invalid(error) => ProviderError::Schema(error.to_string()),
            InstantiateError::Refused(error) => ProviderError::Settings(error.message().to_owned()),
            other => ProviderError::Settings(other.to_string()),
        })?;
        let departures = offer
            .config_schema()
            .and_then(|schema| inline_refs(schema).ok())
            .and_then(|schema| {
                let schema = SchemaRef::new(&schema)?;
                differences(schema, &config, None, "[secret]").ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                (
                    d.key(),
                    d.value.map(|v| v.to_string()).unwrap_or_default(),
                    d.baseline.map(|v| v.to_string()).unwrap_or_default(),
                )
            })
            .collect();
        Ok((Arc::new(built), departures))
    } else if settings.as_map().is_empty() {
        let shared = offer.shared().ok_or(ProviderError::NeedsConfiguration)?;
        Ok((shared, Vec::new()))
    } else {
        Err(ProviderError::TakesNoSettings)
    }
}
