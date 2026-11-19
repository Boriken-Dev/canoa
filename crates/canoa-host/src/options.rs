//! What a source offers before a session starts: the models of an `acp_harness` provider with their versions and each
//! model's efforts, asked of the provider itself with no session and no model turn. What `canoa options` prints.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes and environments are std's"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::Mutex;
use std::sync::mpsc;

use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountModelsQuery, AccountProvider, AcpHarness, CacheLifetime,
    HARNESS_LOGIN, LOCAL_ACCOUNT, ModelInfo, ModelListing, ProcessSpawner, UnknownKind,
};
use serde_json::{Map, Value, json};

use crate::spawn::{Command, Stdio};

pub(crate) mod limits;
mod probe;

use crate::accounts::{Held, NoAccounts};
use crate::config::{Config, EntryKind, Settings, cache_lifetime_name};
use crate::kind;

/// How long the program's `--version` is waited for.
const VERSION_WAIT: Duration = Duration::from_secs(10);
/// The most of a version line kept, in bytes.
const VERSION_BYTES: usize = 128;

/// How long a probe session is waited for, unless the caller says another.
pub const PROBE_WAIT: Duration = Duration::from_secs(20);

/// What a probe session answered, by the entry, its program and the program's version: asked once per version.
static PROBED: Mutex<BTreeMap<String, Choices>> = Mutex::new(BTreeMap::new());

/// How a source is asked beyond its own settings.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Asking {
    /// How long a probe session is waited for.
    pub wait: Duration,
    /// The account of the entry to ask as, with the host's accounts; `None` asks as the automatic `local` account does, on the
    /// login the harness has where it runs.
    pub(crate) account: Option<(String, Arc<crate::accounts::Accounts>)>,
    /// Whether the account may be any of the provider's, as for the person at this computer or a key trusted with accounts, and
    /// not only one the entry lists.
    pub(crate) any_account: bool,
}

impl Default for Asking {
    fn default() -> Self {
        Self {
            wait: PROBE_WAIT,
            account: None,
            any_account: false,
        }
    }
}

/// What one source offers before it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Choices {
    /// Whether the provider listed its models; when not, `models` is empty and `unknown_because` says why.
    pub known: bool,
    /// Why it could not list them.
    pub unknown_because: Option<String>,
    /// What kind of reason that is, when the provider could not be started: `not_installed` or `failed`; `None` for a source
    /// that lists its models only in its session.
    pub unknown_kind: Option<&'static str>,
    /// Whether the provider listed its models and says nobody is signed in on the machine's own login.
    pub signed_out: bool,
    /// The models in the provider's order, each with its own efforts.
    pub models: Vec<ModelInfo>,
    /// The modes a probe session listed, as (id, name), for a program that lists them only in its session; empty for any
    /// other source.
    pub modes: Vec<(String, String)>,
    /// The mode the probe session started in.
    pub default_mode: Option<String>,
    /// The program the source runs, as the provider names it.
    pub program: Option<String>,
    /// The first line of the program's `--version`, when it answered: what a cache of these choices is keyed by.
    pub program_version: Option<String>,
    /// What the provider does with a start request's cache lifetime, in its own sentence (`ignored: ...` when it has no
    /// control for it); `None` when it did not say.
    pub cache_lifetime: Option<String>,
    /// The lifetimes beyond the default the provider honours, each with its own label: the only ones a session may ask.
    pub cache_choices: Vec<(CacheLifetime, String)>,
    /// The entry's own `cache_lifetime`, when canoa.cfg sets one: what its sessions ask unless the person chooses.
    pub configured_cache_lifetime: Option<String>,
    /// For the models of a pool, parallel to `models`: the accounts of the pool that list each one. Empty for one account's.
    pub model_accounts: Vec<Vec<String>>,
    /// For the models of a pool: each account of it and why it listed none, when it did not.
    pub members: Vec<(String, Option<String>)>,
}

impl Choices {
    fn unknown(why: impl Into<String>) -> Self {
        Self {
            known: false,
            unknown_because: Some(why.into()),
            unknown_kind: None,
            signed_out: false,
            models: Vec::new(),
            modes: Vec::new(),
            default_mode: None,
            program: None,
            program_version: None,
            cache_lifetime: None,
            cache_choices: Vec::new(),
            configured_cache_lifetime: None,
            model_accounts: Vec::new(),
            members: Vec::new(),
        }
    }

    /// Why `model` is not one that `who` (an account or a pool, named as a sentence reads it) lists, naming both; `None` when it
    /// is listed, and when the provider could not list its models at all, since the session's own options then decide.
    #[must_use]
    pub fn refuses(&self, who: &str, model: &str) -> Option<String> {
        if !self.known || self.models.is_empty() || self.models.iter().any(|each| each.id == model)
        {
            return None;
        }
        let listed: Vec<&str> = self.models.iter().map(|each| each.id.as_str()).collect();
        Some(format!(
            "model {model:?} is not one {who} lists (it lists: {})",
            listed.join(", ")
        ))
    }

    /// A source that could not be asked at all: `failed`, with the reason.
    fn failed(why: impl Into<String>) -> Self {
        Self {
            unknown_kind: Some("failed"),
            ..Self::unknown(why)
        }
    }

    /// The choices as one JSON object: `known`, `unknown_because` and `unknown_kind` (`not_installed` or `failed`) when
    /// not known, `signed_out` when true, `program`, `program_version`,
    /// `cache_lifetime` (the provider's sentence on what it does with the prompt-cache lifetime),
    /// `cache_choices` (`[{id, label}]`, the lifetimes it honours beyond the default; absent when none) and
    /// `modes` (`[{id, name}]`, with `default_mode`, for a program that lists them in its session) and
    /// `models`, each `{id, name?, description?, resolved?, version?, efforts, default_effort?, is_default?}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("known".into(), json!(self.known));
        let mut put = |key: &str, value: &Option<String>| {
            if let Some(value) = value {
                out.insert(key.into(), json!(value));
            }
        };
        put("unknown_because", &self.unknown_because);
        put("program", &self.program);
        put("program_version", &self.program_version);
        put("cache_lifetime", &self.cache_lifetime);
        put("cache_lifetime_setting", &self.configured_cache_lifetime);
        put("default_mode", &self.default_mode);
        if !self.cache_choices.is_empty() {
            out.insert(
                "cache_choices".into(),
                Value::Array(
                    self.cache_choices
                        .iter()
                        .map(|(lifetime, label)| {
                            json!({"id": cache_lifetime_name(*lifetime), "label": label})
                        })
                        .collect(),
                ),
            );
        }
        if let Some(kind) = self.unknown_kind {
            out.insert("unknown_kind".into(), json!(kind));
        }
        if self.signed_out {
            out.insert("signed_out".into(), json!(true));
        }
        if !self.modes.is_empty() {
            out.insert(
                "modes".into(),
                Value::Array(
                    self.modes
                        .iter()
                        .map(|(id, name)| json!({"id": id, "name": name}))
                        .collect(),
                ),
            );
        }
        let mut models: Vec<Value> = self.models.iter().map(model_json).collect();
        for (model, accounts) in models.iter_mut().zip(&self.model_accounts) {
            model["accounts"] = json!(accounts);
        }
        out.insert("models".into(), Value::Array(models));
        if !self.members.is_empty() {
            out.insert(
                "accounts".into(),
                Value::Array(
                    self.members
                        .iter()
                        .map(|(name, why)| match why {
                            Some(why) => json!({"name": name, "listed": false, "why": why}),
                            None => json!({"name": name, "listed": true}),
                        })
                        .collect(),
                ),
            );
        }
        Value::Object(out)
    }
}

fn model_json(model: &ModelInfo) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(model.id));
    for (key, value) in [
        ("name", &model.name),
        ("description", &model.description),
        ("resolved", &model.resolved),
        ("version", &model.version),
        ("default_effort", &model.default_effort),
    ] {
        if let Some(value) = value {
            out.insert(key.into(), json!(value));
        }
    }
    out.insert("efforts".into(), json!(model.efforts));
    if model.is_default {
        out.insert("is_default".into(), json!(true));
    }
    Value::Object(out)
}

/// What a built-in harness offers, run as the `canoa agent` command would run it: `executable` when given, else the
/// variable that command reads, else the harness's own name.
#[must_use]
pub fn preset(harness: &str, executable: Option<&OsStr>) -> Choices {
    let variable = crate::harness::executable_variable(harness);
    let executable = executable
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os(variable).filter(|value| !value.is_empty()));
    let mut map = serde_json::Map::new();
    if let Some(executable) = executable {
        map.insert(
            "executable".to_owned(),
            Value::String(executable.to_string_lossy().into_owned()),
        );
    }
    let registry = match kind::registry(None) {
        Ok(registry) => Arc::new(registry),
        Err(error) => return Choices::failed(error.to_string()),
    };
    match kind::Harness::new(
        &registry,
        harness,
        (&Settings::new(map), &Settings::default()),
        false,
        Vec::new(),
    ) {
        Ok(configured) => {
            let connection = configured.connection(None, None, CacheLifetime::Default);
            ask(
                &configured,
                connection.harness.as_ref(),
                (connection.spawner, NoAccounts.into_object()),
                Who::Local,
            )
        }
        Err(error) => Choices::failed(error.to_string()),
    }
}

/// What the entry `name` of `config` offers, asked on the machine's own login: a harness entry's provider lists its models;
/// an entry that runs a program speaking ACP is asked through a short probe session. `None` when `config` has no such entry.
#[must_use]
pub fn entry(config: &Config, name: &str) -> Option<Choices> {
    entry_with(config, name, &Asking::default())
}

/// What the entry `name` of `config` offers, asked as `asking` says: with an account of the entry when it names one, and
/// within its time.
#[must_use]
pub fn entry_with(config: &Config, name: &str, asking: &Asking) -> Option<Choices> {
    let entry = config.entry(name)?;
    Some(match &entry.kind {
        EntryKind::Harness {
            harness,
            origin,
            executable,
            options,
            settings,
            account_settings,
            accounts,
            ..
        } => {
            let configured = crate::serve::harness_settings(
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            );
            let registry = match kind::registry(origin.as_ref()) {
                Ok(registry) => Arc::new(registry),
                Err(error) => return Some(Choices::failed(error.to_string())),
            };
            let mut built = match kind::Harness::new(
                &registry,
                harness,
                (&configured, &account_settings.clone().unwrap_or_default()),
                false,
                Vec::new(),
            ) {
                Ok(built) => built,
                Err(error) => return Some(Choices::failed(error.to_string())),
            };
            let mut choices = match &asking.account {
                Some((account, store)) => {
                    let listed =
                        match crate::serve::entry_accounts(config, harness, accounts, &built) {
                            Ok(listed) => listed.unwrap_or_default(),
                            Err(why) => return Some(Choices::failed(why)),
                        };
                    let pools = match built.catalog() {
                        Some(catalog) => {
                            let catalogs = crate::accounts::matching::Catalogs::with(
                                config,
                                harness,
                                (catalog, built.listing()),
                                built.local_account().is_some(),
                            );
                            match crate::pools::for_entry(config, &catalogs, harness, accounts) {
                                Ok(pools) => pools,
                                Err(why) => return Some(Choices::failed(why)),
                            }
                        }
                        None => Vec::new(),
                    };
                    let pooled = pools
                        .iter()
                        .any(|pool| pool.name == *account && pool.open_to(asking.any_account));
                    if !pooled
                        && !listed.iter().any(|(each, ..)| each == account)
                        && !asking.any_account
                    {
                        return Some(Choices::failed(format!(
                            "{name} does not offer the account {account}"
                        )));
                    }
                    let others = if asking.any_account {
                        crate::serve::other_accounts(config, harness, &listed, &built)
                    } else {
                        Vec::new()
                    };
                    let unfit = crate::serve::unfit_accounts(config, harness, &built);
                    built = built
                        .offering(store, listed, others)
                        .with_unfit(unfit)
                        .with_pools(pools, None);
                    match built.pool_named(account, asking.any_account) {
                        Some(pool) => pool_choices(config, &built, pool, asking.any_account),
                        None => as_account(config, &built, account, asking.any_account),
                    }
                }
                None => as_local(&built),
            };
            choices.configured_cache_lifetime = options
                .cache_lifetime
                .map(|lifetime| cache_lifetime_name(lifetime).to_owned());
            choices
        }
        EntryKind::Agent { program, args, .. } => probed(name, program, args, asking.wait),
        EntryKind::Provider { .. } | EntryKind::Tool { .. } => Choices::unknown(
            "this source lists its models in its session, and is not asked before it starts",
        ),
    })
}

/// What the harness lists on the login it has where it runs, as the automatic `local` account is.
pub(crate) fn as_local(built: &kind::Harness) -> Choices {
    let connection = built.connection(None, None, CacheLifetime::Default);
    ask(
        built,
        connection.harness.as_ref(),
        (connection.spawner, NoAccounts.into_object()),
        Who::Local,
    )
}

/// What the account `account` of `built`'s entry lists, asked as a session on it would be: the account is held for the call, an
/// account another provider made is listed by that provider, which knows what its kind is, and one a gateway serves by the
/// gateway's own model. With `any`, the account may be one the entry does not list.
pub(crate) fn as_account(
    config: &Config,
    built: &kind::Harness,
    account: &str,
    any: bool,
) -> Choices {
    let Some(store) = built.accounts_host() else {
        return Choices::failed(format!("{} offers no accounts", built.id()));
    };
    let held = match built.hold_account(account, "choices", any) {
        Ok(taken) => taken,
        Err(refusal) => return Choices::failed(refusal.client_reason()),
    };
    let mut owner = None;
    if held.owner() != built.id() {
        match crate::accounts::commands::provider_for(config, held.owner()).and_then(|other| {
            other
                .account_provider()
                .map(Arc::clone)
                .map_err(crate::cli::Failed::run)
        }) {
            Ok(provider) => owner = Some((provider, store.source())),
            Err(failed) => return Choices::failed(failed.to_string()),
        }
    }
    let connection = built.connection(None, Some(&held), CacheLifetime::Default);
    let keeper = held.keeper(
        owner
            .as_ref()
            .map_or_else(|| built.plain_source(), |(_, source)| *source),
    );
    ask(
        built,
        connection.harness.as_ref(),
        (connection.spawner, keeper),
        Who::Held(&held, owner.map(|(provider, _)| provider)),
    )
}

/// What a pool lists: the models any of its accounts a connection may use lists, each once, with the accounts that list it. A
/// member that could not list says why, and the pool is known when any member is.
pub(crate) fn pool_choices(
    config: &Config,
    built: &kind::Harness,
    pool: &crate::pools::EntryPool,
    any: bool,
) -> Choices {
    let mut merged = Choices::unknown("no account of the pool listed its models");
    let mut models: Vec<ModelInfo> = Vec::new();
    let mut of: Vec<Vec<String>> = Vec::new();
    for member in pool.usable(any) {
        let one = as_account(config, built, &member.name, any);
        let why = (!one.known).then(|| {
            one.unknown_because
                .clone()
                .unwrap_or_else(|| "it lists no models before a session".to_owned())
        });
        if one.known && !merged.known {
            merged = Choices {
                models: Vec::new(),
                members: Vec::new(),
                ..one.clone()
            };
        }
        for model in one.models {
            match models.iter().position(|seen| seen.id == model.id) {
                Some(at) => of[at].push(member.name.clone()),
                None => {
                    of.push(vec![member.name.clone()]);
                    models.push(model);
                }
            }
        }
        merged.members.push((member.name.clone(), why));
    }
    merged.models = models;
    merged.model_accounts = of;
    merged
}

/// What a program speaking ACP lists in a session of its own, asked once per version of the program.
fn probed(name: &str, program: &OsString, args: &[OsString], wait: Duration) -> Choices {
    let program_text = program.to_string_lossy().into_owned();
    let version = version_of(&program_text);
    let key = version.as_ref().map(|version| {
        format!(
            "{name}\0{program_text}\0{}\0{version}",
            args.iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        )
    });
    if let Some(key) = &key
        && let Some(held) = PROBED.lock().ok().and_then(|held| held.get(key).cloned())
    {
        return held;
    }
    let mut choices = match probe::ask(program, args, &[], wait) {
        Ok(probed) if probed.models.is_empty() => Choices::unknown(
            "this agent's session lists no models or efforts, so they are known once a session has started",
        ),
        Ok(probed) => Choices {
            known: true,
            unknown_because: None,
            models: probed.models,
            modes: probed.modes,
            default_mode: probed.default_mode,
            ..Choices::unknown("")
        },
        Err(probe::Failure::NotInstalled(why)) => Choices {
            unknown_kind: Some("not_installed"),
            ..Choices::unknown(why)
        },
        Err(failure) => Choices::failed(failure.text().to_owned()),
    };
    choices.program = Some(program_text);
    choices.program_version = version;
    if choices.known
        && let Some(key) = key
        && let Ok(mut held) = PROBED.lock()
    {
        held.insert(key, choices.clone());
    }
    choices
}

/// Whose models are asked for.
enum Who<'a> {
    /// The automatic `local` account: the harness on its own login, as its configuration stands.
    Local,
    /// An account held for the call, and the account provider of another harness that made it, when the harness's own did not.
    Held(&'a Held, Option<Arc<dyn AccountProvider>>),
}

/// The models of the account asked for, as the account provider of its kind lists them (an account a gateway serves is listed by
/// the gateway's own model, which no account provider describes).
fn models_of(
    built: &kind::Harness,
    (spawner, keeper): (Object<dyn ProcessSpawner>, Object<dyn AccountKeeper>),
    who: Who<'_>,
) -> ModelListing {
    let (grant, other) = match who {
        Who::Local => (AccountGrant::new(LOCAL_ACCOUNT, HARNESS_LOGIN), None),
        Who::Held(held, other) => {
            if let Some(listing) = held.gateway_models() {
                return listing;
            }
            (held.grant(), other)
        }
    };
    let provider = match other {
        Some(provider) => provider,
        None => match built.account_provider() {
            Ok(provider) => Arc::clone(provider),
            Err(why) => return ModelListing::unknown(why),
        },
    };
    provider
        .models(AccountModelsQuery::new(grant), spawner, keeper)
        .unwrap_or_else(|error| ModelListing::unavailable(UnknownKind::Failed, error.message()))
}

/// Asks the account provider for the models of the account `who` names, and `harness` for its program and its version. The
/// provider placed the account's login for the call and tells `keeper` of a renewal the program made.
fn ask(
    built: &kind::Harness,
    harness: &dyn AcpHarness,
    handed: (Object<dyn ProcessSpawner>, Object<dyn AccountKeeper>),
    who: Who<'_>,
) -> Choices {
    let listing = models_of(built, handed, who);
    let info = harness.describe().ok();
    let program = harness.program().ok().flatten();
    let program_version = program.as_deref().and_then(version_of);
    Choices {
        known: listing.known,
        unknown_because: listing.unknown_because,
        unknown_kind: listing.unknown_kind.map(|kind| match kind {
            UnknownKind::NotInstalled => "not_installed",
            _ => "failed",
        }),
        signed_out: listing.signed_out,
        models: listing.models,
        modes: Vec::new(),
        default_mode: None,
        program,
        program_version,
        cache_lifetime: info.as_ref().and_then(|info| info.cache_lifetime.clone()),
        cache_choices: info
            .map(|info| {
                info.cache_choices
                    .into_iter()
                    .map(|choice| (choice.lifetime, choice.label))
                    .collect()
            })
            .unwrap_or_default(),
        configured_cache_lifetime: None,
        model_accounts: Vec::new(),
        members: Vec::new(),
    }
}

/// The first line `program --version` prints, when it answers in time.
fn version_of(program: &str) -> Option<String> {
    let (send, receive) = mpsc::channel();
    let program = program.to_owned();
    std::thread::spawn(move || {
        let _ = send.send(
            Command::new(program)
                .arg("--version")
                .stdin(Stdio::Null)
                .stderr(Stdio::Null)
                .output(),
        );
    });
    let output = receive.recv_timeout(VERSION_WAIT).ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut end = line.len().min(VERSION_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Some(line[..end].to_string())
}

/// What the provider of the account `name` reports of its allowances with no session, as one row of `usage/limits`
/// (`account`, `observed_at`, `buckets`, `limit`); `supported: false` with a reason when the provider learns them only in a turn.
/// Asked through an entry of the account's provider that offers it, else any that runs the provider, else the provider's own
/// defaults.
///
/// # Errors
///
/// Why the account could not be asked: it is not in canoa.cfg, its provider could not be built, or its credential is held.
pub(crate) fn usage_of(
    config: &Config,
    name: &str,
    store: &Arc<crate::accounts::Accounts>,
) -> Result<Value, String> {
    usage_at(config, name, store, &limits::Endpoints::default())
}

/// [`usage_of`], with the account-side readers sent to `endpoints` where it names one.
pub(crate) fn usage_at(
    config: &Config,
    name: &str,
    store: &Arc<crate::accounts::Accounts>,
    endpoints: &limits::Endpoints,
) -> Result<Value, String> {
    let account = config
        .accounts
        .get(name)
        .ok_or_else(|| format!("no account called {name}"))?;
    let harness = account.harness.as_str();
    let runs = |entry: &&crate::config::Entry| matches!(&entry.kind, EntryKind::Harness { harness: each, .. } if each == harness);
    let entry = config
        .entries
        .iter()
        .filter(runs)
        .find(|entry| {
            matches!(&entry.kind, EntryKind::Harness { accounts, .. } if accounts.iter().any(|each| each == name))
        })
        .or_else(|| config.entries.iter().find(runs));
    let (origin, configured, mut account_settings) = match entry.map(|entry| (entry, &entry.kind)) {
        Some((
            entry,
            EntryKind::Harness {
                origin,
                executable,
                options,
                settings,
                account_settings,
                ..
            },
        )) => (
            origin.as_ref(),
            crate::serve::harness_settings(
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            ),
            account_settings
                .clone()
                .unwrap_or_default()
                .as_map()
                .clone(),
        ),
        _ => (None, Settings::new(Map::new()), Map::new()),
    };
    // A stand-in for the provider's usage endpoint is the account provider's own setting.
    if let Some(endpoint) = endpoints.of(harness) {
        account_settings
            .entry("usage_endpoint")
            .or_insert_with(|| Value::String(endpoint.to_owned()));
    }
    let registry = Arc::new(kind::registry(origin).map_err(|error| error.to_string())?);
    let built = kind::Harness::new(
        &registry,
        harness,
        (&configured, &Settings::new(account_settings)),
        false,
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let listed = crate::serve::offered_accounts(config, harness, &[name.to_owned()], &built)?;
    let built = built.offering(store, listed, Vec::new());
    serde_json::to_value(limits::read(&built, name, account)?).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
