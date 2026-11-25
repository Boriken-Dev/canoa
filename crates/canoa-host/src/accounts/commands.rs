//! `canoa accounts list|add|login|remove`: what a person does with accounts, and what an application hosting the library
//! runs as jobs. The credential never passes through a command line: a typed key is read from the terminal without echo,
//! or from standard input when that is not one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, threads and channels are std's"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coqui_acp_api::guatiao::schema::{SchemaRef, inline_refs};
use coqui_acp_api::{AccountKindInfo, AccountProvider, LoginMethodInfo};

use super::login;
use super::providers::{self, Listed};
use super::{Accounts, Refusal, check_settings};
use crate::cli::{Failed, Format, Front};
use crate::config::{AccountConfig, Config, EntryKind, Settings, check_label, is_built_in_harness};
use crate::home::Home;
use crate::{kind, serve};

mod cfg;
mod interact;

/// What a sign-in runs through: an account provider, and the makers that route a method it does not list itself.
pub(crate) struct Signing<'a> {
    pub(crate) provider: &'a dyn AccountProvider,
    pub(crate) makers: &'a crate::makers::Makers,
}

impl<'a> From<&'a Listed> for Signing<'a> {
    fn from(listed: &'a Listed) -> Self {
        Self {
            provider: &*listed.provider,
            makers: &listed.makers,
        }
    }
}

#[cfg(test)]
pub(crate) use cfg::append_table;
pub(crate) use cfg::{
    Change, check_cfg, config_writes, edit_cfg, read_cfg, remove_table, set_offered, set_pool,
    set_table, toml_value,
};
use cfg::{append_account, remove_account};
#[cfg(test)]
pub(crate) use cfg::{remove_entry, write_cfg};
pub(crate) use interact::{Event, Relay};
use interact::{cancel_on_input_closing, sign_in};

/// What a command works on.
pub(crate) struct Env<'a> {
    pub(crate) front: &'a dyn Front,
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
    pub(crate) home: Home,
}

/// What a command works on, without a front: where canoa.cfg is written, what it says and the host's home.
#[derive(Clone, Copy)]
pub(crate) struct Cfg<'a> {
    pub(crate) config_path: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) home: &'a Home,
}

impl Env<'_> {
    pub(crate) fn cfg(&self) -> Cfg<'_> {
        Cfg {
            config_path: &self.config_path,
            config: &self.config,
            home: &self.home,
        }
    }
}

/// The provider of `harness`, as the first entry that runs it configures it, else the built-in one.
pub(crate) fn provider_for(config: &Config, harness: &str) -> Result<kind::Harness, Failed> {
    let entry = config.entries.iter().find(
        |entry| matches!(&entry.kind, EntryKind::Harness { harness: id, .. } if id == harness),
    );
    let (registry, settings, account_settings) = match entry.map(|entry| (entry, &entry.kind)) {
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
            let registry = kind::registry(origin.as_ref()).map_err(Failed::run)?;
            let settings = serve::harness_settings(
                &registry,
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            );
            (
                registry,
                settings,
                account_settings.clone().unwrap_or_default(),
            )
        }
        _ if is_built_in_harness(harness) => {
            let registry = kind::registry(None).map_err(Failed::run)?;
            let settings = serve::builtin_provider_settings(&registry, harness, None, None);
            (registry, settings, Settings::default())
        }
        _ => {
            return Err(Failed::usage(format!(
                "no entry runs a provider called {harness}: add an agent entry with harness = \"{harness}\" and its library or source"
            )));
        }
    };
    kind::Harness::new(
        &Arc::new(registry),
        harness,
        (&settings, &account_settings),
        false,
        Vec::new(),
        crate::makers::Makers::of(config),
    )
    .map_err(|error| Failed::run(format!("the {harness} provider is unusable: {error}")))
}

fn describe_methods(methods: &[LoginMethodInfo]) -> String {
    if methods.is_empty() {
        return "it has none".to_owned();
    }
    methods
        .iter()
        .map(|method| format!("{} ({})", method.id, method.title))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `inputs` against the schema the method asks them by: each is one the method names, of its type, and none is a secret,
/// which is asked for and never taken from a command line.
fn check_inputs(
    method: &LoginMethodInfo,
    inputs: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), Failed> {
    let schema = inline_refs(&method.inputs)
        .map_err(|_| Failed::run("the method's inputs cannot be read"))?;
    let schema = SchemaRef::new(&schema)
        .ok_or_else(|| Failed::run("the method's inputs are not a schema"))?;
    for (key, value) in inputs {
        if schema.find(key).is_some_and(|field| field.is_sensitive()) {
            return Err(Failed::usage(format!(
                "{key} is a secret: the sign-in asks for it, and it is never taken from the command line"
            )));
        }
        super::check_key(schema, key, value, &format!("an input of {}", method.id))
            .map_err(Failed::usage)?;
    }
    Ok(())
}

/// `--input KEY=VALUE` as the sign-in's inputs.
pub(crate) fn inputs_of(
    pairs: &[String],
) -> Result<serde_json::Map<String, serde_json::Value>, Failed> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(Failed::usage("--input is KEY=VALUE"));
        };
        map.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    }
    Ok(map)
}

pub(crate) fn accounts_of(cfg: Cfg<'_>) -> Result<Arc<Accounts>, Failed> {
    Accounts::open(cfg.config, cfg.home.dir()).map_err(|error| Failed::run(error.to_string()))
}

fn state_of(accounts: &Accounts, account: &AccountConfig) -> (String, u64, Option<String>) {
    if account.kind == coqui_acp_api::HARNESS_LOGIN {
        return ("signed in where it runs".to_owned(), 0, None);
    }
    match accounts.load(&account.id) {
        Ok(None) => ("needs sign-in".to_owned(), 0, None),
        Ok(Some(record)) => {
            let state = if record.needs_sign_in {
                "needs sign-in"
            } else if accounts.held(&account.id) {
                "in use"
            } else {
                "signed in"
            };
            (state.to_owned(), record.revision, record.expires_at.clone())
        }
        Err(error) => (format!("unreadable: {error}"), 0, None),
    }
}

/// One account as `list` shows it.
pub(crate) struct Row {
    pub(crate) name: String,
    /// The id of the account provider that made the account; empty when no provider this host can build makes its kind.
    pub(crate) provider: String,
    /// The harness the provider belongs to when it is an agent's own; `None` for a maker's account.
    pub(crate) agent: Option<String>,
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) state: String,
    pub(crate) revision: u64,
    pub(crate) expires_at: Option<String>,
    /// The account's types: every standard it works for, as its provider answers them for this account.
    pub(crate) types: Vec<String>,
    /// The harnesses that can run a session on the account: those that take one of its types, by the rule a session is started with.
    pub(crate) fits: Vec<String>,
    /// Whether the host makes the account itself (the `local` of a harness that has a login of its own) and no canoa.cfg names
    /// it, so it can be neither removed nor signed in again.
    pub(crate) automatic: bool,
    /// What the person is told of the account's settings, such as an address that is plain http beyond this computer.
    pub(crate) warnings: Vec<String>,
    /// The account provider that made it when `[host.disabled]` leaves that provider off: the account stays in canoa.cfg, is not
    /// offered and cannot be signed in again until the provider is turned on.
    pub(crate) disabled: Option<String>,
    /// How the automatic `local` account signs in when it is the key of the environment that does (`environment key (ANTHROPIC_API_KEY)`:
    /// the variable's name, never its value); `None` for the login the program keeps for itself and for any other account.
    pub(crate) via: Option<String>,
    /// Why the provider could not say whether the login of the automatic `local` account is there, which is listed all the same;
    /// `None` for any other account and for a login that is.
    pub(crate) note: Option<String>,
}

impl Row {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name, "provider": self.provider, "agent": self.agent, "kind": self.kind,
            "id": self.id, "state": self.state, "revision": self.revision, "expires_at": self.expires_at,
            "types": self.types, "fits": self.fits, "automatic": self.automatic,
            "warnings": self.warnings, "disabled": self.disabled, "via": self.via, "note": self.note,
        })
    }
}

/// Every account canoa.cfg names, with its state, then the automatic `local` account of each harness that has a login of its own.
/// A `local`'s `id` is the one usage is recorded and reported under, which tells the harnesses' apart.
pub(crate) fn rows(cfg: Cfg<'_>) -> Result<Vec<Row>, Failed> {
    Ok(rows_of(cfg, &accounts_of(cfg)?))
}

/// The kinds each account provider that `[host.disabled]` leaves off would make, by provider id, read from the libraries the host
/// has loaded: what says which of canoa.cfg's accounts the switch put out of reach.
pub(crate) fn kinds_of_off_providers(config: &crate::config::Config) -> Vec<(String, Vec<String>)> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for (_, provider) in config
        .disabled
        .each()
        .filter(|(kind, _)| *kind == "account_provider")
    {
        for (_, registry) in crate::control::providers::loaded(config).iter() {
            let Ok(Some(made)) = crate::kind::account_instance_unchecked(
                registry,
                provider,
                &[],
                &crate::config::Settings::default(),
                &crate::config::Settings::default(),
            ) else {
                continue;
            };
            let Ok(offer) = made.offers() else {
                continue;
            };
            found.push((
                provider.to_owned(),
                offer.kinds.iter().map(|kind| kind.id.clone()).collect(),
            ));
            break;
        }
    }
    found
}

fn rows_of(cfg: Cfg<'_>, accounts: &Arc<Accounts>) -> Vec<Row> {
    let catalogs = super::matching::Catalogs::new(cfg.config);
    let off = kinds_of_off_providers(cfg.config);
    let mut rows: Vec<Row> = cfg
        .config
        .accounts
        .iter()
        .map(|(name, account)| {
            let (state, revision, expires_at) = state_of(accounts, account);
            let kind = catalogs.kind(account);
            let made = catalogs.made(&account.kind).map(|(made, _)| made);
            let disabled = made.is_none().then(|| {
                off.iter()
                    .find(|(_, kinds)| kinds.contains(&account.kind))
                    .map(|(provider, _)| provider.clone())
            });
            Row {
                name: name.clone(),
                agent: made.as_ref().and_then(|made| made.harness.clone()),
                provider: made.map(|made| made.provider).unwrap_or_default(),
                kind: account.kind.clone(),
                id: account.id.clone(),
                state,
                revision,
                expires_at,
                types: kind
                    .as_ref()
                    .map(|info| info.provides.clone())
                    .unwrap_or_default(),
                fits: kind
                    .as_ref()
                    .map(|info| super::matching::fitting(&catalogs, account, info))
                    .unwrap_or_default(),
                warnings: kind
                    .as_ref()
                    .map(|info| super::address_notes(name, info, account.settings.as_ref()))
                    .unwrap_or_default(),
                automatic: false,
                disabled: disabled.flatten(),
                via: None,
                note: None,
            }
        })
        .collect();
    for (harness, kind) in catalogs.locals() {
        let account = AccountConfig::local(&harness);
        let (state, revision, expires_at) = state_of(accounts, &account);
        let login = catalogs.login(&harness);
        let (via, note) = login
            .as_ref()
            .map_or((None, None), |login| (login.via(), login.note()));
        rows.push(Row {
            name: coqui_acp_api::LOCAL_ACCOUNT.to_owned(),
            id: account.usage_id(),
            kind: kind.id,
            types: catalogs.login_standard(&harness).into_iter().collect(),
            provider: catalogs.provider_id_of(&harness).unwrap_or_default(),
            agent: Some(harness.clone()),
            fits: vec![harness],
            state,
            revision,
            expires_at,
            automatic: true,
            via,
            note,
            warnings: Vec::new(),
            disabled: None,
        });
    }
    rows
}

/// `canoa accounts list`.
pub(crate) fn list(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    let accounts = accounts_of(env.cfg())?;
    for row in rows_of(env.cfg(), &accounts) {
        let line = match format {
            Format::Json => row.json().to_string(),
            Format::Text => {
                let said = row.via.as_deref().or(row.note.as_deref());
                let line = format!("{} {} {} {}", row.name, row.provider, row.kind, row.state);
                match said {
                    Some(said) => format!("{line} {said}"),
                    None => line,
                }
            }
        };
        env.front.print(&line).map_err(Failed::run)?;
        if format == Format::Text {
            for warning in &row.warnings {
                env.front
                    .print(&format!("non-default: {warning}"))
                    .map_err(Failed::run)?;
            }
        }
    }
    if format == Format::Text
        && let Some(note) = accounts.note()
    {
        env.front.print(note).map_err(Failed::run)?;
    }
    Ok(())
}

/// `canoa accounts providers`: each account provider with the ways it signs in.
pub(crate) fn providers(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    for listed in providers::all(&env.config) {
        let methods: Vec<&str> = listed
            .offer
            .logins
            .iter()
            .map(|method| method.id.as_str())
            .collect();
        let line = match format {
            Format::Json => serde_json::json!({
                "id": listed.id, "title": listed.title, "harness": listed.harness,
                "logins": methods,
            })
            .to_string(),
            Format::Text => format!("{} {} ({})", listed.id, listed.title, methods.join(", ")),
        };
        env.front.print(&line).map_err(Failed::run)?;
    }
    Ok(())
}

/// A lowercase version 4 UUID, which Canoa never gives to two accounts.
pub(crate) fn new_id() -> Result<String, Failed> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| Failed::run(format!("no randomness: {error}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = data_encoding::HEXLOWER.encode(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// What a sign-in made, as the settings an account of that kind keeps in canoa.cfg.
fn settings_of(
    outcome: &coqui_acp_api::LoginOutcome,
) -> Result<serde_json::Map<String, serde_json::Value>, Failed> {
    guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(
        &outcome.settings,
    )
    .map_err(|_| Failed::run("the sign-in's settings are not a table"))
}

/// What a sign-in made or renewed, for the line that tells it.
pub(crate) struct Added {
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) label: Option<String>,
    pub(crate) checked: bool,
}

impl Added {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({"account": {
            "name": self.name, "provider": self.provider, "kind": self.kind, "id": self.id,
            "label": self.label, "checked": self.checked}})
    }
}

/// What runs a sign-in: the stored accounts, the provider and the method's id, and what it made.
pub(crate) type Signer<'a> =
    dyn FnMut(&Accounts, &Signing<'_>, &str) -> Result<coqui_acp_api::LoginOutcome, Failed> + 'a;

/// `canoa accounts add NAME`.
pub(crate) fn add(
    env: &Env<'_>,
    name: &str,
    provider: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    (format, until_stdin_closes): (Format, bool),
) -> Result<(), Failed> {
    let added = add_with(
        env.cfg(),
        name,
        provider,
        method,
        inputs,
        &mut |accounts, provider, method| {
            let cancel = cancel_on_input_closing(env.front, until_stdin_closes)?;
            sign_in(env, accounts, provider, method, inputs, format, &cancel)
        },
    )?;
    let line = match format {
        Format::Json => added.json().to_string(),
        Format::Text => format!("added {name} ({})", added.kind),
    };
    env.front.print(&line).map_err(Failed::run)
}

/// Adds the account `name` through the account provider `provider`, by one of its own ways to sign in `method`, signing in through
/// `signer`.
pub(crate) fn add_with(
    cfg: Cfg<'_>,
    name: &str,
    provider: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    signer: &mut Signer<'_>,
) -> Result<Added, Failed> {
    check_label(name).map_err(Failed::usage)?;
    if name == coqui_acp_api::LOCAL_ACCOUNT {
        return Err(Failed::usage(format!(
            "{name} is taken: it is the automatic account of a harness that has a login of its own"
        )));
    }
    if cfg.config.accounts.contains_key(name) {
        return Err(Failed::usage(format!("an account called {name} exists")));
    }
    let listed = providers::find(cfg.config, provider)?;
    let methods = &listed.offer.logins;
    let Some(method) = method.and_then(|id| methods.iter().find(|method| method.id == id)) else {
        return Err(Failed::usage(format!(
            "--method is one of the ways {} signs in: {}",
            listed.title,
            describe_methods(methods)
        )));
    };
    check_inputs(method, inputs)?;
    let accounts = accounts_of(cfg)?;
    let outcome = signer(&accounts, &Signing::from(&listed), &method.id)?;
    // A sign-in that made the agent's own login (Antigravity's, which the program stores for itself) adds nothing to canoa.cfg:
    // that login is the automatic `local` account its harness already has.
    if outcome.kind == coqui_acp_api::HARNESS_LOGIN
        && let Some(harness) = &listed.harness
    {
        return Ok(Added {
            name: coqui_acp_api::LOCAL_ACCOUNT.to_owned(),
            provider: listed.id.clone(),
            kind: outcome.kind.clone(),
            id: format!("{}:{harness}", coqui_acp_api::LOCAL_ACCOUNT),
            label: outcome.label.clone(),
            checked: outcome.checked,
        });
    }
    let kind = listed.offer.kind(&outcome.kind).ok_or_else(|| {
        Failed::run(format!(
            "the sign-in made a {} account, which the provider does not list",
            outcome.kind
        ))
    })?;
    let settings = settings_of(&outcome)?;
    check_account_settings(kind, &settings)?;
    let id = new_id()?;
    let expires = outcome.expires_at.clone();
    accounts
        .store_login(&id, outcome.credentials.expose(), expires)
        .map_err(|error| Failed::run(error.to_string()))?;
    let account = AccountConfig {
        owner: None,
        kind: outcome.kind.clone(),
        id: id.clone(),
        settings: (!settings.is_empty()).then(|| Settings::new(settings)),
    };
    if let Err(failed) = append_account(
        &crate::config_store::FileStore::at(cfg.config_path),
        name,
        &account,
    ) {
        let _ = accounts.forget_credentials(&id);
        let _ = accounts.forget_directory(&id);
        return Err(failed);
    }
    Ok(Added {
        name: name.to_owned(),
        provider: listed.id.clone(),
        kind: outcome.kind.clone(),
        id,
        label: outcome.label.clone(),
        checked: outcome.checked,
    })
}

fn check_account_settings(
    kind: &AccountKindInfo,
    settings: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), Failed> {
    check_settings(kind, Some(&Settings::new(settings.clone())))
        .map_err(|why| Failed::run(format!("the sign-in's settings are refused: {why}")))
}

/// `canoa accounts login NAME`: signing in again to an account that needs it.
pub(crate) fn login(
    env: &Env<'_>,
    name: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    (format, until_stdin_closes): (Format, bool),
) -> Result<(), Failed> {
    let added = login_with(
        env.cfg(),
        name,
        method,
        inputs,
        &mut |accounts, provider, method| {
            let cancel = cancel_on_input_closing(env.front, until_stdin_closes)?;
            sign_in(env, accounts, provider, method, inputs, format, &cancel)
        },
    )?;
    let line = match format {
        Format::Json => serde_json::json!({"account": {"name": name, "id": added.id}}).to_string(),
        Format::Text => format!("signed in {name}"),
    };
    env.front.print(&line).map_err(Failed::run)
}

/// Signs in again to the account `name`, through `signer`, holding the account while it does.
pub(crate) fn login_with(
    cfg: Cfg<'_>,
    name: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    signer: &mut Signer<'_>,
) -> Result<Added, Failed> {
    let account = cfg
        .config
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no account called {name}")))?;
    let catalogs = super::matching::Catalogs::new(cfg.config);
    let (made, _) = catalogs.made(&account.kind).ok_or_else(|| {
        Failed::usage(format!(
            "no account provider this host can build makes a {} account",
            account.kind
        ))
    })?;
    let listed = providers::find(cfg.config, &made.provider)?;
    let making: Vec<&LoginMethodInfo> = listed
        .offer
        .logins
        .iter()
        .filter(|method| method.produces == account.kind)
        .collect();
    let method = match (method, making.as_slice()) {
        (Some(id), _) => making.iter().find(|method| method.id == id).copied(),
        (None, [only]) => Some(*only),
        (None, _) => None,
    };
    let Some(method) = method else {
        return Err(Failed::usage(format!(
            "--method is one of the ways to sign in to a {} account: {}",
            account.kind,
            making
                .iter()
                .map(|method| format!("{} ({})", method.id, method.title))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    check_inputs(method, inputs)?;
    let accounts = accounts_of(cfg)?;
    let _lease = accounts.exclusive(&account.id).map_err(refused)?;
    let outcome = signer(&accounts, &Signing::from(&listed), &method.id)?;
    if outcome.kind != account.kind {
        return Err(Failed::run(format!(
            "the sign-in made a {} account, and {name} is a {} one",
            outcome.kind, account.kind
        )));
    }
    accounts
        .store_login(
            &account.id,
            outcome.credentials.expose(),
            outcome.expires_at.clone(),
        )
        .map_err(|error| Failed::run(error.to_string()))?;
    Ok(Added {
        name: name.to_owned(),
        provider: listed.id.clone(),
        kind: account.kind.clone(),
        id: account.id.clone(),
        label: outcome.label.clone(),
        checked: outcome.checked,
    })
}

fn refused(refusal: Refusal) -> Failed {
    Failed::run(refusal.to_string())
}

/// `canoa accounts remove NAME`: forgets the credential and the account's state, and the account from canoa.cfg. Nothing is
/// signed out at the provider.
pub(crate) fn remove(env: &Env<'_>, name: &str) -> Result<(), Failed> {
    remove_with(env.cfg(), name)?;
    env.front
        .print(&format!("removed {name}"))
        .map_err(Failed::run)
}

/// Removes the account `name`, which no entry may offer and no connection may hold.
pub(crate) fn remove_with(cfg: Cfg<'_>, name: &str) -> Result<(), Failed> {
    if name == coqui_acp_api::LOCAL_ACCOUNT {
        return Err(Failed::usage(format!(
            "{name} is the automatic account of a harness that has a login of its own: there is nothing of it to remove"
        )));
    }
    let account = cfg
        .config
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no account called {name}")))?;
    if let Some(entry) = cfg.config.entries.iter().find(|entry| {
        matches!(&entry.kind, EntryKind::Harness { accounts, .. } if accounts.iter().any(|offered| offered == name))
    }) {
        return Err(Failed::usage(format!(
            "entry {} offers {name}: take it out of that entry's accounts first",
            entry.name
        )));
    }
    if let Some((held, _)) = cfg
        .config
        .pools
        .iter()
        .find(|(_, pool)| pool.accounts.iter().any(|member| member == name))
    {
        return Err(Failed::usage(format!(
            "pool {held} holds {name}: take it out of that pool's accounts first"
        )));
    }
    let accounts = accounts_of(cfg)?;
    let lease = accounts.exclusive(&account.id).map_err(refused)?;
    use canoa::config_store::ConfigStore as _;
    let store = crate::config_store::FileStore::at(cfg.config_path);
    let key = canoa::config_store::Key::new(["host", "accounts", name]);
    let before = store
        .get(key)
        .map_err(|error| Failed::run(error.to_string()))?
        .value;
    remove_account(&store, name)?;
    // The stored credential goes with the account; were it to stay, nothing would name it any more.
    if let Err(error) = accounts.forget_credentials(&account.id) {
        if let Some(before) = before {
            let _ = store.change(canoa::config_store::Changes::of(vec![
                canoa::config_store::Op::set(["host", "accounts", name], before),
            ]));
        }
        return Err(Failed::run(error.to_string()));
    }
    if let Err(error) = accounts.forget_directory(&account.id) {
        tracing::warn!("the directory of the removed account {name} could not be removed: {error}");
    }
    drop(lease);
    Ok(())
}

/// The kinds and the sign-in methods the account provider `provider` offers, and no other provider's.
pub(crate) fn catalog_of(
    config: &Config,
    provider: &str,
) -> Result<(Vec<AccountKindInfo>, Vec<LoginMethodInfo>), Failed> {
    let listed = providers::find(config, provider)?;
    Ok((listed.offer.kinds, listed.offer.logins))
}

/// Runs a sign-in whose prompts a [`Relay`] carries, for a caller that shows them itself.
pub(crate) fn sign_in_relayed(
    accounts: &Accounts,
    provider: &Signing<'_>,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    relay: Arc<Relay>,
) -> Result<coqui_acp_api::LoginOutcome, Failed> {
    login::sign_in(
        accounts,
        provider.makers,
        provider.provider,
        method,
        inputs,
        relay,
        None,
    )
    .map_err(|error| Failed::run(error.to_string()))
}
