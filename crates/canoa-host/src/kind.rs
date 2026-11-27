//! The host's end of the `acp_harness` kind: the providers an entry can offer by id, the instance an entry configures, and
//! the request a connection starts with. The bundled agents' libraries are registered in this process (`bundled`); an outside
//! provider is loaded from the library or source an entry names, into a registry of its own, so one entry never resolves an id
//! that only another entry's file offers. Both are chosen and started the same way.
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
    AccountCatalog, AccountKindInfo, AccountProvider, AccountsLink, AcpHarness, CacheLifetime,
    Environment, HARNESS_LOGIN, HarnessNotice, LOCAL_ACCOUNT, ProcessSpawner, SignedInCheck,
    SignedInState, StartRequest,
};

use crate::accounts::matching::Runs;
use crate::accounts::{ACCOUNT_META, Accounts, Held, Watcher};
use crate::config::{AccountConfig, Config, Origin, Settings, cache_lifetime_named};
use crate::makers::Makers;
use crate::provider::{ProviderError, map_of, open};
use coqui_acp_pool::accounts::EntryPool;

mod settle;

/// The `_meta` key of a client's `initialize` that says how long its session's prompt cache should live: an object
/// `{"lifetime": "default" | "short" | "long", "chosen": bool}`. `chosen` (true when absent) says the person chose it for
/// this session, which wins over the entry's `cache_lifetime`; false is the client's default for its kind of session, which
/// the entry's setting replaces.
pub const CACHE_META: &str = "dev.boriken.canoa/cache_lifetime";

/// The `_meta` key of a client's `initialize` that names the model its session starts on, by the id its account lists. A model
/// the account does not list refuses the start, naming both.
pub const MODEL_META: &str = "dev.boriken.canoa/model";

/// The setting through which a profiled connection lets a client's plugin reach the harness: Claude Code's provider has it.
const CLIENT_PLUGINS: &str = "allow_client_plugins";

/// A registry holding what one entry can choose from: the bundled agents' providers when the entry has no origin, and only what
/// its origin loads when it has one, so an id is resolved from that entry's own file and no other.
///
/// # Errors
///
/// [`ProviderError::Load`] or [`ProviderError::Source`] for the built-in providers or an origin that does not load.
pub(crate) fn registry(origin: Option<&Origin>) -> Result<Registry, ProviderError> {
    let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
    match origin {
        Some(origin) => {
            open(&mut registry, origin)?;
            crate::vendors::note(&registry);
        }
        None => crate::bundled::register(&mut registry, crate::bundled::ALL)
            .map_err(ProviderError::Load)?,
    }
    Ok(registry)
}

/// One setting of an instance that differs from the provider's default, for the log: its key, value and default.
pub(crate) type Departure = (String, String, String);

/// How the login a harness keeps for itself stands on this computer, as its account provider answers
/// (`AccountProvider::signed_in`): the host keeps no list of files or variables of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Login {
    /// A login is there. Holds the name of the variable of the environment that signs the program in when that is what counted,
    /// never its value.
    SignedIn(Option<String>),
    /// Every place the program keeps a login was looked at and none holds one: why.
    SignedOut(String),
    /// The provider could not tell: why. The account is listed, since a login that cannot be looked at may be there.
    Unknown(String),
}

impl Login {
    /// What the provider's answer says; a provider that fails to answer is unknown.
    fn of(answer: Result<SignedInCheck, coqui_acp_api::guatiao::library::ProviderError>) -> Self {
        match answer {
            Ok(check) => match check.state {
                SignedInState::SignedIn => Self::SignedIn(check.variable),
                SignedInState::SignedOut => Self::SignedOut(
                    check
                        .reason
                        .unwrap_or_else(|| "no login is where the program keeps one".to_owned()),
                ),
                // A state a newer provider adds is one this host cannot read.
                _ => Self::Unknown(
                    check
                        .reason
                        .unwrap_or_else(|| "the provider could not tell".to_owned()),
                ),
            },
            Err(error) => Self::Unknown(error.message().to_owned()),
        }
    }

    /// How the row of the account `local` says it signs in, when it is a key of the environment.
    pub(crate) fn via(&self) -> Option<String> {
        match self {
            Self::SignedIn(Some(variable)) => Some(format!("environment key ({variable})")),
            _ => None,
        }
    }

    /// What the row of the account `local` tells the person when the provider could not say whether the login is there.
    pub(crate) fn note(&self) -> Option<String> {
        match self {
            Self::Unknown(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// Whether the account `local` is listed: unless the program has no login at all.
    pub(crate) const fn listed(&self) -> bool {
        !matches!(self, Self::SignedOut(_))
    }
}

/// A question to a harness's account provider, kept until someone needs the answer.
#[derive(Clone)]
pub(crate) struct LoginProbe {
    provider: Arc<dyn AccountProvider>,
    environment: Environment,
}

impl LoginProbe {
    /// Asks the provider whether the program's own login exists.
    pub(crate) fn ask(&self) -> Login {
        ask(self.provider.as_ref(), self.environment.clone())
    }
}

#[cfg(test)]
thread_local! {
    /// What a unit test makes every harness's own login stand as: no test looks at the person's home, keyring or environment.
    static TEST_LOGIN: core::cell::RefCell<Login> = const { core::cell::RefCell::new(Login::SignedIn(None)) };
}

/// What `provider` says of the login its program keeps when asked with `environment`.
#[cfg(not(test))]
fn ask(provider: &dyn AccountProvider, environment: Environment) -> Login {
    use coqui_acp_api::SignedInQuery;
    Login::of(provider.signed_in(SignedInQuery::new().environment(environment)))
}

/// What a unit test says it is, so that no test asks a real provider.
#[cfg(test)]
fn ask(_: &dyn AccountProvider, _: Environment) -> Login {
    TEST_LOGIN.with(|slot| slot.borrow().clone())
}

/// Runs `f` with every harness's own login standing as `login`, for a unit test.
#[cfg(test)]
pub(crate) fn with_login<T>(login: Login, f: impl FnOnce() -> T) -> T {
    let before = TEST_LOGIN.with(|slot| slot.replace(login));
    let out = f();
    TEST_LOGIN.with(|slot| *slot.borrow_mut() = before);
    out
}

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
    /// The account kinds and sign-in methods the harness's own account provider offers, and no maker's: what an account of this
    /// provider can be, and how it is made.
    listing: AccountCatalog,
    /// The id of the account provider the harness declares.
    account_provider_id: Option<String>,
    /// The harness's account provider, which signs in, renews, watches, reads plan limits and lists models; `None` when it offers none.
    accounts_of: Option<Arc<dyn AccountProvider>>,
    /// Whether the account provider offers the harness's own login as the automatic `local` account.
    local: bool,
    /// The account makers the host carries, whose kinds the listing joins and whose accounts this harness runs.
    makers: Arc<Makers>,
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
    /// canoa.cfg as the host read it when it started: what lists the models of an account another provider made.
    config: Option<Arc<Config>>,
    /// The pools the entry offers, which a connection may name where it names an account.
    pools: Vec<EntryPool>,
    /// The pool the entry's first `accounts` name stands for, which a connection that names no account runs on.
    default_pool: Option<String>,
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
    /// thread to. The account provider the harness declares (`HarnessInfo::accounts`) is configured with the second of
    /// `settings`, and with the harness's settings the link names where the account provider declares them.
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
        makers: Arc<Makers>,
    ) -> Result<Self, ProviderError> {
        let (plain, departures) =
            instance(registry, id, settings).map_err(|error| match error {
                ProviderError::Schema(why) => ProviderError::Settings(why),
                other => other,
            })?;
        let catalog = plain.accounts().ok();
        let link = plain.describe().ok().and_then(|info| info.accounts);
        let (accounts_of, listing, local) =
            account_side(registry, link.as_ref(), (settings, account_settings))?;
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
            account_provider_id: link.map(|link| link.provider),
            accounts_of,
            local,
            makers,
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

    /// Where the accounts this entry offers are kept; `None` for an entry that offers none.
    pub(crate) fn accounts_host(&self) -> Option<&Arc<Accounts>> {
        self.accounts.as_ref().map(|offer| &offer.host)
    }

    /// The entry's working roots; none lets any directory.
    pub(crate) fn roots(&self) -> &[String] {
        &self.roots
    }

    /// What the provider says of the accounts it applies: the standards it accepts, and the variables it clears.
    pub(crate) const fn catalog(&self) -> Option<&AccountCatalog> {
        self.catalog.as_ref()
    }

    /// The account makers in force for this harness: the bundled libraries' and those canoa.cfg's libraries add.
    pub(crate) const fn makers(&self) -> &Arc<Makers> {
        &self.makers
    }

    /// The kinds of account and the ways to sign in that the harness's own account provider offers, each kind saying the standards
    /// it provides.
    pub(crate) const fn listing(&self) -> &AccountCatalog {
        &self.listing
    }

    /// The id of the account provider the harness declares; `None` when it declares none.
    pub(crate) fn account_provider_id(&self) -> Option<&str> {
        self.account_provider_id.as_deref()
    }

    /// The automatic `local` account: the harness run on its own login, as its configuration stands, when its account provider
    /// offers it. It is in no `[host.accounts]`: the host lists it for the entry, first unless the entry names accounts.
    pub(crate) fn local_account(&self) -> Option<Offered> {
        if !self.local {
            return None;
        }
        let info = self.listing.kind(HARNESS_LOGIN)?.clone();
        let mut config = AccountConfig::local(&self.id);
        config.id = LOCAL_ACCOUNT.to_owned();
        Some((LOCAL_ACCOUNT.to_owned(), config, info))
    }

    /// How its own login stands on this computer, asked of its account provider with the environment its sessions would have (the
    /// host's, and the variables the entry's shell adds); `None` when it offers no such login. It starts no program and reaches no
    /// network, and is asked again each time, so a login made meanwhile is seen.
    pub(crate) fn login(&self) -> Option<Login> {
        self.login_probe().map(|probe| probe.ask())
    }

    /// What asks how its own login stands, later and when it is needed: a keyring is looked at no earlier than that. `None` when it
    /// offers no such login.
    pub(crate) fn login_probe(&self) -> Option<LoginProbe> {
        if !self.local {
            return None;
        }
        Some(LoginProbe {
            provider: Arc::clone(self.accounts_of.as_ref()?),
            environment: Environment::from_iter(self.shell_env.clone()),
        })
    }

    /// This harness as an account is held for it.
    fn runs(&self) -> Runs<'_> {
        Runs {
            harness: &self.id,
            catalog: self.catalog.as_ref(),
            makers: Some(&self.makers),
        }
    }

    /// The account provider as configured, for what signs in, renews and reads plan limits.
    ///
    /// # Errors
    ///
    /// Why this harness has none: it declares no account provider, or its library does not offer the one it declares.
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
            config: None,
            pools: Vec::new(),
            default_pool: None,
        });
        self
    }

    /// This harness offering `pools`, the first named by `default_pool` being what a connection that names no account runs on.
    pub(crate) fn with_pools(
        mut self,
        pools: Vec<EntryPool>,
        default_pool: Option<String>,
    ) -> Self {
        if let Some(offer) = &mut self.accounts {
            offer.pools = pools;
            offer.default_pool = default_pool;
        }
        self
    }

    /// The pool `name` as this entry offers it to a connection: the entry's own, the host's `any-<standard>`, or with a grant for
    /// accounts (`any`) any other the harness accepts.
    pub(crate) fn pool_named(&self, name: &str, any: bool) -> Option<&EntryPool> {
        self.accounts
            .as_ref()?
            .pools
            .iter()
            .find(|pool| pool.name == name && pool.open_to(any))
    }

    /// This harness knowing canoa.cfg as the host read it, which a connection's start is checked against.
    pub(crate) fn with_config(mut self, config: Arc<Config>) -> Self {
        if let Some(offer) = &mut self.accounts {
            offer.config = Some(config);
        }
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

    /// The account the first message chooses, not yet held: what [`Self::choose`] holds. `None` for an entry that offers
    /// none, or when the client names `null` and may.
    fn pick(&self, first: &[u8], any: bool) -> Result<Option<&Offered>, String> {
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
        let picked = match named {
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
        Ok(Some(picked))
    }

    /// Whether the session the first message starts runs on the automatic `local` account: it names it, or names none and the entry
    /// has no account or pool to start on before it. A client that names `null` asks for the login the program has where it runs.
    fn runs_on_local(&self, first: &[u8]) -> bool {
        let named = serde_json::from_slice::<serde_json::Value>(first)
            .ok()
            .and_then(|message| {
                message
                    .get("params")?
                    .get("_meta")?
                    .get(ACCOUNT_META)
                    .cloned()
            });
        match named {
            None => self.accounts.as_ref().is_none_or(|offer| {
                offer.default_pool.is_none()
                    && offer.offered.first().is_none_or(|(name, config, _)| {
                        name == LOCAL_ACCOUNT && config.kind == HARNESS_LOGIN
                    })
            }),
            Some(serde_json::Value::String(name)) => name == LOCAL_ACCOUNT,
            Some(_) => false,
        }
    }

    /// The label, kind id and provided standards of the account the first message chooses, which a session's environment
    /// names as `AGENTS_ACCOUNT` and its kind and provider; nothing of its credentials. `None` when there is none, or the
    /// choice is one `choose` refuses.
    pub(crate) fn account_facts(
        &self,
        first: &[u8],
        any: bool,
    ) -> Option<(String, String, Vec<String>)> {
        self.pick(first, any)
            .ok()
            .flatten()
            .map(|(name, config, info)| (name.clone(), config.kind.clone(), info.provides.clone()))
    }

    /// The variables that holding the account the first message chooses would set for the harness's program, with no
    /// credential among them and nothing held or made: its directory and endpoint, as the harness's own provider names
    /// them. A profile's layers read them before a session starts. The names the specification reserves are left out.
    pub(crate) fn account_environment(&self, first: &[u8], any: bool) -> Vec<(String, String)> {
        let (Some(offer), Ok(Some((_, config, info)))) = (&self.accounts, self.pick(first, any))
        else {
            return Vec::new();
        };
        let grant = offer.host.preview(config, info, self.runs());
        let set = crate::vendors::behave(&self.id, |vendor| vendor.account_variables(grant))
            .and_then(Result::ok)
            .unwrap_or_default();
        set.into_iter()
            .map(|variable| (variable.name, variable.value))
            .filter(|(name, _)| !coqui_aos::is_reserved_variable(name))
            .collect()
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
        if self.runs_on_local(first)
            && let Some(Login::SignedOut(why)) = self.login()
        {
            return Err(format!(
                "nobody is signed in to {} on this computer ({why}): sign in to it from Accounts, or choose an account that is signed in",
                self.id
            ));
        }
        let Some(offer) = &self.accounts else {
            return Ok(None);
        };
        let Some((name, config, info)) = self.pick(first, any)? else {
            return Ok(None);
        };
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
        self.connection_as(profile, (&[], false), account, lifetime)
    }

    /// Whether a connection can be started to take a plugin of the host's (a provider that takes client plugins, built so).
    pub(crate) fn takes_plugins(&self) -> bool {
        self.profiled.is_some()
    }

    /// [`Self::connection`] for a session. One with no profile is given `identity`, the variables that say who the session is,
    /// for its program's own tools to read, as a profile's connection's environment carries them; with `plugins` it is started to
    /// take the host's plugin as a profile's is (see [`Self::takes_plugins`]).
    pub(crate) fn connection_as(
        &self,
        profile: Option<(Environment, &Path)>,
        (identity, plugins): (&[(String, String)], bool),
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
                let mut environment = Environment::from_iter(self.shell_env.clone());
                for (name, value) in identity {
                    if environment.get(name).is_none() {
                        environment.insert(name.clone(), value.clone());
                    }
                }
                if !environment.is_empty() {
                    request = request.environment(environment);
                }
                match &self.profiled {
                    Some(profiled) if plugins => {
                        source = self.accounts.as_ref().map_or(0, |offer| offer.sources.1);
                        Arc::clone(profiled)
                    }
                    _ => Arc::clone(&self.plain),
                }
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
        if !held.rotates() || self.listing.kind(&grant.kind).is_none() || grant.state_dir.is_none()
        {
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
    let offered = harness
        .describe()
        .is_ok_and(|info| info.cache_choices.contains(&lifetime));
    if offered {
        lifetime
    } else {
        CacheLifetime::Default
    }
}

/// Whether the `acp_harness` provider `id` of `registry` declares a setting called `key` in its settings schema. False when the
/// registry offers no such provider; true when the provider publishes no schema, since its keys are then not known.
pub(crate) fn declares_setting(registry: &Registry, id: &str, key: &str) -> bool {
    let Some(Ok(offer)) = registry.offer::<dyn AcpHarness>(id) else {
        return false;
    };
    let Some(schema) = offer
        .config_schema()
        .and_then(|schema| inline_refs(schema).ok())
    else {
        return true;
    };
    SchemaRef::new(&schema).is_some_and(|schema| schema.find(key).is_some())
}

/// An instance of provider `id`, from the entry's `settings`, which the provider checks against its schema; and where the
/// settings depart from the schema's defaults.
pub(crate) fn instance(
    registry: &Registry,
    id: &str,
    settings: &Settings,
) -> Result<(Arc<dyn AcpHarness>, Vec<Departure>), ProviderError> {
    if crate::disabled::off::<dyn AcpHarness>(id) {
        return Err(ProviderError::Missing);
    }
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

/// A harness's account provider, what it offers listed with the kinds the account makers offer that it can run, and whether it
/// offers the harness's own login as the automatic `local` account.
type AccountSide = (Option<Arc<dyn AccountProvider>>, AccountCatalog, bool);

/// The account provider `link` names (the one harness `harness` declares), what it offers listed with the kinds the account
/// makers offer that the harness can run (`accepts` is what it takes). `own` configures it; the harness's settings the link names
/// are given to it where it declares them, so that it runs the program the harness runs. A harness that declares none has no
/// accounts to make, and its listing holds only the login a program has where it runs.
fn account_side(
    registry: &Registry,
    link: Option<&AccountsLink>,
    (settings, own): (&Settings, &Settings),
) -> Result<AccountSide, ProviderError> {
    let mut listing = AccountCatalog::new();
    let provider = match link {
        Some(link) => account_provider(registry, link, settings, own)?,
        None => None,
    };
    let mut local = false;
    if let Some(provider) = &provider {
        let offer = provider
            .offers()
            .map_err(|error| ProviderError::Settings(error.message().to_owned()))?;
        local = offer.local;
        listing.kinds.extend(offer.kinds);
        listing.logins = offer.logins;
    }
    Ok((provider, listing, local))
}

/// The account provider `link` names, configured with `own` and with the keys of `settings` the link shares.
pub(crate) fn account_provider(
    registry: &Registry,
    link: &AccountsLink,
    settings: &Settings,
    own: &Settings,
) -> Result<Option<Arc<dyn AccountProvider>>, ProviderError> {
    account_instance(
        registry,
        &link.provider,
        &link.shared_settings,
        settings,
        own,
    )
}

/// The account provider `id`, configured with `own` and with the keys `shared` of `settings` where it declares them; `None` when
/// the registry offers none.
pub(crate) fn account_instance(
    registry: &Registry,
    id: &str,
    shared: &[String],
    settings: &Settings,
    own: &Settings,
) -> Result<Option<Arc<dyn AccountProvider>>, ProviderError> {
    if crate::disabled::off::<dyn AccountProvider>(id) {
        return Ok(None);
    }
    account_instance_unchecked(registry, id, shared, settings, own)
}

/// As [`account_instance`], for a provider `[host.disabled]` leaves off too: what the host asks of one to say what its accounts
/// were, never to run them.
pub(crate) fn account_instance_unchecked(
    registry: &Registry,
    id: &str,
    shared: &[String],
    settings: &Settings,
    own: &Settings,
) -> Result<Option<Arc<dyn AccountProvider>>, ProviderError> {
    let Some(Ok(offer)) = registry.offer::<dyn AccountProvider>(id) else {
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
        for key in shared {
            if !map.contains_key(key)
                && declares(key)
                && let Some(value) = settings.as_map().get(key)
            {
                map.insert(key.clone(), value.clone());
            }
        }
        // A provider that declares a `search_path` is told where the host looks a program up by name.
        if declares(canoa::search::SETTING) && !map.contains_key(canoa::search::SETTING) {
            map.insert(
                canoa::search::SETTING.to_owned(),
                serde_json::Value::String(
                    crate::sources::search_path().to_string_lossy().into_owned(),
                ),
            );
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
        Ok(Some(crate::accounts::routed::wrap(Arc::new(built))))
    } else if own.as_map().is_empty() {
        Ok(offer.shared().map(crate::accounts::routed::wrap))
    } else {
        Err(accounts(ProviderError::TakesNoSettings))
    }
}

#[cfg(test)]
mod tests;
