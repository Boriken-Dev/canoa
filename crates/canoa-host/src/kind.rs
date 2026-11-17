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

use canoa_claude::acp::{ClaudeAccounts, ClaudeProvider};
use canoa_codex::acp::{CodexAccounts, CodexProvider};
use canoa_pi::acp::{PiAccounts, PiProvider};
use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::InstantiateError;
use coqui_acp_api::guatiao::schema::{SchemaRef, differences, inline_refs};
use coqui_acp_api::{
    AccountCatalog, AccountKindInfo, AccountProvider, AcpHarness, CacheLifetime, Environment,
    HarnessNotice, ProcessSpawner, StartRequest,
};

use crate::accounts::matching::Runs;
use crate::accounts::{ACCOUNT_META, Accounts, Held, Watcher};
use crate::config::{AccountConfig, Origin, Settings, cache_lifetime_named};
use crate::provider::{ProviderError, map_of, open};

// What `Registry::register_local` takes: the three built-in harness providers and their account providers, linked into this binary.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-built-in",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        ClaudeProvider,
        ClaudeAccounts,
        CodexProvider,
        CodexAccounts,
        PiProvider,
        PiAccounts
    ]
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
    /// The provider id the entry names, as a history's origin says it.
    id: String,
    plain: Arc<dyn AcpHarness>,
    /// The same configuration with client plugins allowed, for a connection that has a profile; `None` when the provider
    /// has no such setting.
    profiled: Option<Arc<dyn AcpHarness>>,
    roots: Vec<String>,
    departures: Vec<Departure>,
    /// What the provider says of the accounts it applies: the standards it accepts and how; `None` when it could not say.
    catalog: Option<AccountCatalog>,
    /// The account kinds and sign-in methods the harness's account provider offers, with the host's own beside them: what an
    /// account of this provider can be, and how it is made.
    listing: AccountCatalog,
    /// The harness's account provider, which signs in, renews, watches and reads plan limits; `None` when it offers none.
    accounts_of: Option<Arc<dyn AccountProvider>>,
    /// The accounts the entry offers, when it offers any.
    accounts: Option<Offer>,
    /// The entry's `cache_lifetime`: what its sessions ask when the client does not choose.
    lifetime: Option<CacheLifetime>,
    /// The entry's name, which the log carries with each notice its connections' providers give.
    entry: String,
    /// What the shell the entry uses is told to the harness with: variables added to what each connection starts with, a
    /// profile's own of the same name taking the place of them.
    shell_env: Vec<(String, String)>,
    /// Dropped after the instances, which point into what it loaded.
    _registry: Arc<Registry>,
}

/// An account an entry offers: its name, its configuration and its kind in the provider's catalog.
pub(crate) type Offered = (String, AccountConfig, AccountKindInfo);

/// The accounts an entry offers, in its order.
struct Offer {
    host: Arc<Accounts>,
    offered: Vec<Offered>,
    /// The other accounts of the entry's provider, which only a key trusted with accounts may choose.
    others: Vec<Offered>,
    /// The accounts of other providers that cannot run here, with why, which only a key trusted with accounts is told.
    unfit: Vec<(String, String)>,
    /// Number the plain and the profiled instance's sequences, so one's are not taken for the other's.
    sources: (u64, u64),
}

/// What one connection runs on: the account it chose, and what the provider is handed for it.
pub(crate) struct Connection {
    pub(crate) harness: Arc<dyn AcpHarness>,
    pub(crate) request: StartRequest,
    pub(crate) spawner: Object<dyn ProcessSpawner>,
    /// What begins the watch of the account's login, for one that rotates and is the provider's own: begun once the harness has
    /// placed the login, and held until the account is given back.
    pub(crate) watch: Option<Watcher>,
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
    /// thread to. The account provider the same library offers as `<id>_accounts` is configured with the second of
    /// `settings`, and with the harness's `executable` and `harness` settings where it declares them.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Missing`] for an id nothing offers, [`ProviderError::WrongKind`] for one that is not an
    /// `acp_harness`, and [`ProviderError::Settings`] and its kin for a configuration it refuses, the profiled one and
    /// the account provider's included.
    pub(crate) fn new(
        registry: &Arc<Registry>,
        id: &str,
        (settings, account_settings): (&Settings, &Settings),
        profiles: bool,
        roots: Vec<String>,
    ) -> Result<Self, ProviderError> {
        let (plain, departures) =
            instance(registry, id, settings).map_err(|error| match error {
                ProviderError::Schema(why) => ProviderError::Settings(why),
                other => other,
            })?;
        let catalog = plain.accounts().ok();
        let (accounts_of, listing) = account_side(registry, id, settings, account_settings)?;
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
            id: id.to_owned(),
            plain,
            profiled,
            roots,
            departures,
            catalog,
            listing,
            accounts_of,
            accounts: None,
            lifetime: None,
            entry: String::new(),
            shell_env: Vec::new(),
            _registry: Arc::clone(registry),
        })
    }

    /// This harness serving the entry `entry`, whose name the log carries with a notice.
    pub(crate) fn with_entry(mut self, entry: &str) -> Self {
        entry.clone_into(&mut self.entry);
        self
    }

    /// What a connection's spawner does with a notice its provider gives: the log, at `info`, with the entry and the kind.
    /// A notice holds no client text and no secret value.
    fn log_notices(&self) -> impl Fn(HarnessNotice) + Send + Sync + 'static + use<> {
        let entry = self.entry.clone();
        move |notice| {
            tracing::info!(entry, kind = notice.kind.as_str(), "{}", notice.text);
        }
    }

    /// This harness starting each connection's program with `env` added: how the shell the entry uses is told to it.
    pub(crate) fn with_shell_env(mut self, env: Vec<(String, String)>) -> Self {
        self.shell_env = env;
        self
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
            .ok_or_else(|| "the cache lifetime is default, off, short or long".to_owned())?;
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

    /// The provider id the entry names.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The entry's working roots; none lets any directory.
    pub(crate) fn roots(&self) -> &[String] {
        &self.roots
    }

    /// What the provider says of the accounts it applies: the standards it accepts, and the variables it clears.
    pub(crate) const fn catalog(&self) -> Option<&AccountCatalog> {
        self.catalog.as_ref()
    }

    /// The kinds of account and the ways to sign in that the harness's account provider offers, with the host's own beside
    /// them, each kind saying the standards it provides.
    pub(crate) const fn listing(&self) -> &AccountCatalog {
        &self.listing
    }

    /// This harness as an account is held for it.
    fn runs(&self) -> Runs<'_> {
        Runs {
            harness: &self.id,
            catalog: self.catalog.as_ref(),
        }
    }

    /// The account provider as configured, for what signs in, renews and reads plan limits.
    ///
    /// # Errors
    ///
    /// Why this harness has none: its library offers no `<id>_accounts`.
    pub(crate) fn account_provider(&self) -> Result<&Arc<dyn AccountProvider>, String> {
        self.accounts_of.as_ref().ok_or_else(|| {
            format!(
                "{} has no account provider, so it signs no one in and reads no plan limits",
                self.id
            )
        })
    }

    /// This harness offering `offered`, each by name with its configuration and its kind in the catalog, kept in `host`.
    /// `others` are the provider's other accounts, which a key trusted with accounts may choose beside them.
    pub(crate) fn offering(
        mut self,
        host: &Arc<Accounts>,
        offered: Vec<Offered>,
        others: Vec<Offered>,
    ) -> Self {
        self.accounts = Some(Offer {
            host: Arc::clone(host),
            offered,
            others,
            unfit: Vec::new(),
            sources: (host.source(), host.source()),
        });
        self
    }

    /// This harness, knowing why each of `unfit` (an account's name and the reason) cannot run a session on it.
    pub(crate) fn with_unfit(mut self, unfit: Vec<(String, String)>) -> Self {
        if let Some(offer) = &mut self.accounts {
            offer.unfit = unfit;
        }
        self
    }

    /// The accounts this entry offers, in its order; none when it offers none.
    pub(crate) fn offered(&self) -> &[Offered] {
        self.accounts.as_ref().map_or(&[], |offer| &offer.offered)
    }

    /// The account `name` this entry offers or, with `any`, another account of its provider a key trusted with accounts may use.
    pub(crate) fn account_named(&self, name: &str, any: bool) -> Option<&Offered> {
        let offer = self.accounts.as_ref()?;
        offer
            .offered
            .iter()
            .chain(any.then_some(&offer.others).into_iter().flatten())
            .find(|(each, ..)| each == name)
    }

    /// The number of the plain instance's account provider, which the keeper of a call outside a connection reports under.
    pub(crate) fn plain_source(&self) -> u64 {
        self.accounts.as_ref().map_or(0, |offer| offer.sources.0)
    }

    /// The account `name`, held for a call that is not a connection (moving a session between accounts). With `any`, it may be
    /// an account of the provider the entry does not list.
    ///
    /// # Errors
    ///
    /// Why it cannot be had: not offered, held by another connection, or its store failed.
    pub(crate) fn hold_account(
        &self,
        name: &str,
        holder: &str,
        any: bool,
    ) -> Result<Held, crate::accounts::Refusal> {
        let (Some(offer), Some((name, config, info))) =
            (&self.accounts, self.account_named(name, any))
        else {
            return Err(crate::accounts::Refusal::Store(
                "this entry does not offer that account".to_owned(),
            ));
        };
        offer.host.begin(name, config, info, self.runs(), holder)
    }

    /// The grant of the account `name`, for a call that only reads with its credential (no lease, nothing started).
    ///
    /// # Errors
    ///
    /// Why it cannot be had: not offered, no credentials, or its store failed.
    pub(crate) fn read_account(
        &self,
        name: &str,
    ) -> Result<coqui_acp_api::AccountGrant, crate::accounts::Refusal> {
        let (Some(offer), Some((_, config, info))) =
            (&self.accounts, self.account_named(name, false))
        else {
            return Err(crate::accounts::Refusal::Store(
                "this entry does not offer that account".to_owned(),
            ));
        };
        offer.host.read_grant(config, info)
    }

    /// The account the client's first message chooses for its connection, held for it: the one it names in `_meta`, else the
    /// entry's first; `None` for an entry that offers none. `holder` says who is holding it. With `any`, the client may also
    /// name an account of the provider the entry does not list, or `null` for the login the program has where it runs.
    ///
    /// # Errors
    ///
    /// Why the choice is refused, for the client.
    pub(crate) fn choose(
        &self,
        first: &[u8],
        holder: &str,
        any: bool,
    ) -> Result<Option<Held>, String> {
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
        let listed = |name: &str| offer.offered.iter().find(|(offered, ..)| offered == name);
        let (name, config, info) = match named {
            None => match offer.offered.first() {
                Some(first) => Some(first),
                None => return Ok(None),
            },
            Some(serde_json::Value::Null) if any => return Ok(None),
            Some(serde_json::Value::String(ref name)) => listed(name).or_else(|| {
                any.then(|| offer.others.iter().find(|(other, ..)| other == name))
                    .flatten()
            }),
            Some(_) => return Err("the account is named by its name, as text".to_owned()),
        }
        .ok_or_else(|| {
            let why = any
                .then(|| match &named {
                    Some(serde_json::Value::String(name)) => offer
                        .unfit
                        .iter()
                        .find(|(each, _)| each == name)
                        .map(|(_, why)| format!(": it cannot run on {}: {why}", self.id)),
                    _ => None,
                })
                .flatten();
            format!(
                "this entry does not offer that account{}",
                why.unwrap_or_default()
            )
        })?;
        let held = offer
            .host
            .begin(name, config, info, self.runs(), holder)
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
        let mut request = StartRequest::builder();
        for root in &self.roots {
            request = request.working_root(root.clone());
        }
        let mut source = self.accounts.as_ref().map_or(0, |offer| offer.sources.0);
        let harness = match profile {
            Some((mut environment, skills)) => {
                if self.profiled.is_some() {
                    source = self.accounts.as_ref().map_or(0, |offer| offer.sources.1);
                }
                for (name, value) in &self.shell_env {
                    if environment.get(name).is_none() {
                        environment.insert(name.clone(), value.clone());
                    }
                }
                request = request.environment(environment);
                if !self.roots.is_empty() {
                    request = request.working_root(skills.to_string_lossy().into_owned());
                }
                Arc::clone(self.profiled.as_ref().unwrap_or(&self.plain))
            }
            None => {
                if !self.shell_env.is_empty() {
                    request = request.environment(Environment::from_iter(self.shell_env.clone()));
                }
                Arc::clone(&self.plain)
            }
        };
        request = request.cache_lifetime(honoured(harness.as_ref(), lifetime));
        let mut watch = None;
        let spawner = match (account, &self.accounts) {
            (Some(held), Some(_)) => {
                request = request.account(held.grant());
                watch = self.watcher(held, source);
                held.spawner(self.log_notices())
            }
            _ => coqui_acp_harness::LocalSpawner::new()
                .notices(self.log_notices())
                .into_object(),
        };
        Connection {
            harness,
            request: request.build(),
            spawner,
            watch,
        }
    }

    /// What watches `held` while a connection or a call runs on it: only an account of this harness's own provider that
    /// rotates and keeps its login in a directory has a login the program renews behind the host's back, and the
    /// provider's account provider is the one that reads it.
    fn watcher(&self, held: &Held, source: u64) -> Option<Watcher> {
        let grant = held.grant();
        if !held.rotates() || held.owner() != self.id || grant.state_dir.is_none() {
            return None;
        }
        let provider = Arc::clone(self.accounts_of.as_ref()?);
        Some(Watcher::new(
            provider,
            grant,
            held.spawner(self.log_notices()),
            held.keeper(source),
        ))
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

/// `lifetime` as `harness` can take it: Off goes only to a provider that lists it, since one that predates the value
/// refuses it, and is otherwise the default.
fn honoured(harness: &dyn AcpHarness, lifetime: CacheLifetime) -> CacheLifetime {
    if lifetime != CacheLifetime::Off {
        return lifetime;
    }
    let offered = harness.describe().is_ok_and(|info| {
        info.cache_choices
            .iter()
            .any(|choice| choice.lifetime == lifetime)
    });
    if offered {
        lifetime
    } else {
        CacheLifetime::Default
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

/// The account provider the library of harness `harness` offers as `<harness>_accounts`, what it offers listed with the host's
/// own kind. `own` configures it; the harness's `executable` and `harness` settings are
/// given to it where it declares them, so that it runs the program the harness runs. A library with no such provider has no
/// accounts to make, and its listing holds only the login a program has where it runs.
fn account_side(
    registry: &Registry,
    harness: &str,
    settings: &Settings,
    own: &Settings,
) -> Result<(Option<Arc<dyn AccountProvider>>, AccountCatalog), ProviderError> {
    let mut listing = AccountCatalog::new();
    let provider = account_provider(registry, harness, settings, own)?;
    if let Some(provider) = &provider {
        let offer = provider
            .offers()
            .map_err(|error| ProviderError::Settings(error.message().to_owned()))?;
        listing.kinds.extend(offer.kinds);
        listing.logins = offer.logins;
    }
    crate::accounts::foundry::extend(&mut listing, harness);
    Ok((provider, listing))
}

fn account_provider(
    registry: &Registry,
    harness: &str,
    settings: &Settings,
    own: &Settings,
) -> Result<Option<Arc<dyn AccountProvider>>, ProviderError> {
    let Some(Ok(offer)) = registry.offer::<dyn AccountProvider>(&format!("{harness}_accounts"))
    else {
        return Ok(None);
    };
    let accounts = |error: ProviderError| match error {
        ProviderError::Schema(why) | ProviderError::Settings(why) => {
            ProviderError::Settings(format!("account provider: {why}"))
        }
        other => other,
    };
    if offer.builds_instances() {
        let schema = offer
            .config_schema()
            .and_then(|schema| inline_refs(schema).ok());
        let declares = |key: &str| {
            schema
                .as_ref()
                .and_then(SchemaRef::new)
                .is_some_and(|schema| schema.find(key).is_some())
        };
        let mut map = own.as_map().clone();
        for key in ["executable", "harness"] {
            if !map.contains_key(key)
                && declares(key)
                && let Some(value) = settings.as_map().get(key)
            {
                map.insert(key.to_owned(), value.clone());
            }
        }
        let config = Value::from(
            map_of(&Settings::new(map)).map_err(|why| accounts(ProviderError::Settings(why)))?,
        );
        let built = offer.instantiate(&config).map_err(|error| {
            accounts(match error {
                InstantiateError::Invalid(error) => ProviderError::Schema(error.to_string()),
                InstantiateError::Refused(error) => {
                    ProviderError::Settings(error.message().to_owned())
                }
                other => ProviderError::Settings(other.to_string()),
            })
        })?;
        Ok(Some(Arc::new(built)))
    } else if own.as_map().is_empty() {
        Ok(offer.shared())
    } else {
        Err(accounts(ProviderError::TakesNoSettings))
    }
}

#[cfg(test)]
mod tests;
