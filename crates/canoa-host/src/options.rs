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
use crate::config::{Config, EntryKind, Settings, cache_lifetime_label, cache_lifetime_name};
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
    /// The lifetimes beyond the default the provider honours, each with the host's label for it: the only ones a session may ask.
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
        Arc::default(),
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
        EntryKind::Harness { options, .. } => {
            let mut built = match harness_of(entry, crate::makers::Makers::of(config)) {
                Ok(built) => built,
                Err(why) => return Some(Choices::failed(why)),
            };
            let mut choices = match &asking.account {
                Some((account, store)) => {
                    built = match offering(config, entry, built, store, asking.any_account) {
                        Ok(built) => built,
                        Err(why) => return Some(Choices::failed(why)),
                    };
                    if built.pool_named(account, asking.any_account).is_none()
                        && built.account_named(account, asking.any_account).is_none()
                    {
                        return Some(Choices::failed(format!(
                            "{name} does not offer the account {account}"
                        )));
                    }
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

/// The provider the harness entry `entry` runs, configured as the entry says, offering no account yet.
///
/// # Errors
///
/// Why it cannot be built: the entry runs no harness, or its provider is missing or refuses its settings.
pub(crate) fn harness_of(
    entry: &crate::config::Entry,
    makers: Arc<crate::makers::Makers>,
) -> Result<kind::Harness, String> {
    let EntryKind::Harness {
        harness,
        origin,
        executable,
        options,
        settings,
        account_settings,
        ..
    } = &entry.kind
    else {
        return Err(format!("{} runs no harness", entry.name));
    };
    let registry = Arc::new(kind::registry(origin.as_ref()).map_err(|error| error.to_string())?);
    let configured = crate::serve::harness_settings(
        &registry,
        entry,
        harness,
        executable.as_ref(),
        options,
        settings.as_ref(),
    );
    kind::Harness::new(
        &registry,
        harness,
        (&configured, &account_settings.clone().unwrap_or_default()),
        false,
        Vec::new(),
        makers,
    )
    .map_err(|error| error.to_string())
}

/// `built`, the provider of the harness entry `entry`, offering what the entry offers: its accounts (each pool it names standing
/// for the accounts it holds), the pools, and canoa.cfg as it stands, kept in `store`. With `any`, the accounts of the provider the
/// entry does not list are there to be chosen too.
///
/// # Errors
///
/// Why the entry's accounts cannot be offered: a pool its harness cannot take, an account it cannot run.
pub(crate) fn offering(
    config: &Config,
    entry: &crate::config::Entry,
    built: kind::Harness,
    store: &Arc<crate::accounts::Accounts>,
    any: bool,
) -> Result<kind::Harness, String> {
    let EntryKind::Harness {
        harness, accounts, ..
    } = &entry.kind
    else {
        return Err(format!("{} runs no harness", entry.name));
    };
    let listed =
        crate::serve::entry_accounts(config, harness, accounts, &built)?.unwrap_or_default();
    let pools = match built.catalog() {
        Some(_) => {
            let catalogs = crate::accounts::matching::Catalogs::with(config, harness, &built);
            crate::pools::for_entry(config, &catalogs, harness, accounts)?
        }
        None => Vec::new(),
    };
    let default_pool = accounts
        .first()
        .filter(|first| crate::pools::is_pool(config, first))
        .cloned();
    let others = if any {
        crate::serve::other_accounts(config, harness, &listed, &built)
    } else {
        Vec::new()
    };
    let unfit = crate::serve::unfit_accounts(config, harness, &built);
    Ok(built
        .offering(store, listed, others)
        .with_unfit(unfit)
        .with_pools(pools, default_pool)
        .with_config(Arc::new(config.clone())))
}

/// The harness other than `runs` whose own account provider made the account `held`, when one did: the login of another harness, or
/// a kind another harness's provider offers.
fn other_harness(config: &Config, held: &crate::accounts::Held, runs: &str) -> Option<String> {
    let kind = &held.grant().kind;
    let owner = if kind == coqui_acp_api::HARNESS_LOGIN {
        Some(held.owner().to_owned())
    } else {
        crate::accounts::matching::Catalogs::new(config)
            .made(kind)
            .and_then(|(made, _)| made.harness)
    };
    owner.filter(|owner| owner != runs)
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
    if let Some(maker) = built.makers().for_kind(&held.grant().kind) {
        // A maker lists what it made, whichever harness is asked.
        owner = Some((maker.provider(), store.source()));
    } else if let Some(made_by) = other_harness(config, &held, built.id()) {
        match crate::accounts::commands::provider_for(config, &made_by).and_then(|other| {
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
            // The account's own provider, a maker included, is given every credential the account holds.
            (
                if other.is_some() {
                    held.provider_grant()
                } else {
                    held.grant()
                },
                other,
            )
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
                    .map(|lifetime| (lifetime, cache_lifetime_label(lifetime).to_owned()))
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

/// What the provider of the account `name` of canoa.cfg reports of its allowances, with the account-side readers sent to
/// `endpoints` where it names one; see [`usage_for`].
///
/// # Errors
///
/// Why the account could not be asked: it is not in canoa.cfg, its provider could not be built, or its credential is held.
#[cfg(test)]
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
    usage_for(config, (name, account), store, endpoints)
}

/// What an account's provider reported with no session: the report, and who the account is when its agent says.
#[derive(Debug)]
pub(crate) struct UsageRead {
    pub(crate) report: coqui_acp_api::UsageReport,
    /// The account's email or name and its plan, from the agent's own reader.
    pub(crate) account: Option<canoa::usage_reader::AccountDetails>,
    /// Whether the row may answer again for a while. A failed reading of what the agent stores is not kept: asking again costs
    /// nothing, and a sign-in meanwhile changes the answer.
    pub(crate) keep: bool,
}

impl UsageRead {
    /// The row `usage/read` and `usage/limits` list: the report, and `details` (`label`, `plan`) when the agent said who it is.
    pub(crate) fn row(self) -> Result<Value, String> {
        let mut row = serde_json::to_value(self.report).map_err(|error| error.to_string())?;
        if let (Some(object), Some(details)) = (row.as_object_mut(), self.account) {
            object.insert(
                "details".to_owned(),
                json!({"label": details.label, "plan": details.plan}),
            );
        }
        Ok(row)
    }
}

/// What the provider of the account `name` reports of its allowances with no session, as one row of `usage/limits` (`account`,
/// `observed_at`, `buckets`, `limit`); `supported: false` with a reason when the provider learns them only in a turn. The account
/// is `name` in canoa.cfg or the automatic `local` account of a harness (`AccountConfig::local`, which no file holds). Asked
/// through an entry of the account's provider that offers it, else any that runs the provider, else the provider's own defaults.
///
/// # Errors
///
/// Why the account could not be asked: its provider could not be built, or its credential is held.
pub(crate) fn usage_for(
    config: &Config,
    (name, account): (&str, &crate::config::AccountConfig),
    store: &Arc<crate::accounts::Accounts>,
    endpoints: &limits::Endpoints,
) -> Result<Value, String> {
    usage_report_for(config, (name, account), store, endpoints)?.row()
}

pub(crate) fn usage_report_for(
    config: &Config,
    (name, account): (&str, &crate::config::AccountConfig),
    store: &Arc<crate::accounts::Accounts>,
    endpoints: &limits::Endpoints,
) -> Result<UsageRead, String> {
    let harness = crate::accounts::matching::Catalogs::new(config).reading_harness(account)?;
    let harness = harness.as_str();
    let runs = |entry: &&crate::config::Entry| matches!(&entry.kind, EntryKind::Harness { harness: each, .. } if each == harness);
    let entry = config
        .entries
        .iter()
        .filter(runs)
        .find(|entry| {
            matches!(&entry.kind, EntryKind::Harness { accounts, .. } if accounts.iter().any(|each| each == name))
        })
        .or_else(|| config.entries.iter().find(runs));
    let (registry, configured, mut account_settings) = match entry.map(|entry| (entry, &entry.kind))
    {
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
        )) => {
            let registry =
                Arc::new(kind::registry(origin.as_ref()).map_err(|error| error.to_string())?);
            let configured = crate::serve::harness_settings(
                &registry,
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            );
            (
                registry,
                configured,
                account_settings
                    .clone()
                    .unwrap_or_default()
                    .as_map()
                    .clone(),
            )
        }
        _ => (
            Arc::new(kind::registry(None).map_err(|error| error.to_string())?),
            Settings::new(Map::new()),
            Map::new(),
        ),
    };
    let own = account_settings.clone();
    // A stand-in for the provider's usage endpoint is the account provider's own setting.
    if let Some(endpoint) = endpoints.of(harness) {
        account_settings
            .entry("usage_endpoint")
            .or_insert_with(|| Value::String(endpoint.to_owned()));
    }
    let built = kind::Harness::new(
        &registry,
        harness,
        (&configured, &Settings::new(account_settings)),
        false,
        Vec::new(),
        crate::makers::Makers::of(config),
    )
    .map_err(|error| error.to_string())?;
    let listed = crate::serve::offered_accounts(config, harness, &[name.to_owned()], &built)?;
    let built = built.offering(store, listed, Vec::new());
    let report = limits::read(&built, name, account)?;
    if report.supported {
        return Ok(UsageRead {
            report,
            account: None,
            keep: true,
        });
    }
    Ok(
        usage_via_reader(&registry, harness, (&configured, &own), account).unwrap_or(UsageRead {
            report,
            account: None,
            keep: true,
        }),
    )
}

/// The account's figures from the harness's usage reader, with no session open: read from what the agent stores when the reader
/// can, else read from the program started for the moment.
fn usage_via_reader(
    registry: &Arc<coqui_acp_api::guatiao::library::Registry>,
    harness: &str,
    (configured, own): (&Settings, &Map<String, Value>),
    account: &crate::config::AccountConfig,
) -> Option<UsageRead> {
    let preset = crate::bundled::ALL
        .iter()
        .find(|bundle| bundle.preset.harness == Some(harness))
        .map(|bundle| &bundle.preset);
    let reader_id = preset.and_then(|preset| preset.usage)?;
    let reader = crate::usage_splice::Reader::offered_by(reader_id, &[Arc::clone(registry)])
        .or_else(|| crate::usage_splice::Reader::named(reader_id))?;
    if reader.info().stored {
        return Some(usage_stored(&reader, configured, own, account));
    }
    let program = configured
        .as_map()
        .get("executable")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(OsString::from)
        .or_else(|| {
            std::env::var_os(crate::harness::executable_variable(harness))
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            preset.and_then(|preset| {
                crate::bundled::runs_managed_by_own_name(preset.program)
                    .then(|| crate::sources::managed_program(preset.program))
                    .flatten()
                    .map(|(path, _)| path.into_os_string())
            })
        })
        .or_else(|| {
            preset
                .and_then(|preset| crate::sources::find(preset.program, preset.search))
                .map(std::path::PathBuf::into_os_string)
        })?;
    let args: Vec<OsString> = preset
        .map(|preset| preset.args.iter().map(OsString::from).collect())
        .unwrap_or_default();
    let asked = match probe::ask_usage(&reader, &program, &args, &[]) {
        Ok(asked) => asked,
        Err(why) => {
            let name = preset.map_or(harness, |preset| preset.name);
            return Some(UsageRead {
                report: coqui_acp_api::UsageReport::unsupported(format!(
                    "{name} was started to read the account and gave nothing ({}): start {name} and sign in there",
                    why.text()
                )),
                account: None,
                keep: true,
            });
        }
    };
    let mut report = coqui_acp_api::UsageReport::new();
    report.supported = true;
    report.sources = vec![coqui_acp_api::Source::Reported];
    report.account = Some(account.usage_id());
    report.observed_at = crate::usage_splice::unix_now();
    report.buckets = asked.buckets;
    Some(UsageRead {
        report,
        account: asked.account,
        keep: true,
    })
}

/// What a reader that reads the agent's stored login says with no process (`root` 0), configured with the entry's account
/// settings and the harness's `executable` when they name none. Nothing is started: a failure is the answer, with who the
/// account is when the reader could still say.
fn usage_stored(
    reader: &crate::usage_splice::Reader,
    configured: &Settings,
    own: &Map<String, Value>,
    account: &crate::config::AccountConfig,
) -> UsageRead {
    let mut config = own.clone();
    if let Some(executable) = configured.as_map().get("executable")
        && !config.contains_key("executable")
    {
        config.insert("executable".to_owned(), executable.clone());
    }
    let read = reader
        .open_configured(&Value::Object(config))
        .and_then(|opened| {
            opened
                .reading
                .read(0, false)
                .map_err(|error| error.message().to_owned())
        });
    let (mut report, details) = match read {
        Ok(reading) => match reading.failure {
            None => {
                let mut report = coqui_acp_api::UsageReport::new();
                report.supported = true;
                report.sources = vec![coqui_acp_api::Source::Reported];
                report.buckets = reading.buckets;
                (report, reading.account)
            }
            Some(failure) => (
                coqui_acp_api::UsageReport::unsupported(failure.why),
                reading.account,
            ),
        },
        Err(why) => (coqui_acp_api::UsageReport::unsupported(why), None),
    };
    report.account = Some(account.usage_id());
    report.observed_at = crate::usage_splice::unix_now();
    UsageRead {
        keep: report.supported,
        report,
        account: details,
    }
}

#[cfg(test)]
mod tests;
