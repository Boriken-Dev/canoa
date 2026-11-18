//! Accounts on the control channel, the owner's alone: `accounts/catalog`, `accounts/list`, `accounts/remove`, and the two
//! sign-ins, `accounts/add` and `accounts/login`, which carry their progress and put the form a sign-in asks (a key, a pasted
//! code) to the application sealed, as an agent's `secret_request` is, so no typed value crosses the link in the clear.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and the clock are std's"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use coqui_acp_api::guatiao::ToValue;
use coqui_acp_api::guatiao::schema::inline_refs;
use coqui_acp_api::{Credentials, LoginAnswer, LoginForm};
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value, json};

use super::{BAD_PARAMS, NOT_FOUND, POLL, REFUSED, error_line};
use crate::accounts::commands::{self, Cfg, Event, Relay};
use crate::accounts::matching::{self, Catalogs};
use crate::cli::Failed;
use crate::config::{Config, EntryKind, Layers, Settings};
use crate::home::Home;
use crate::provider::map_of;
use crate::secrets::Hub;
use crate::secrets::ask::{Asked, Asker, METHOD};

type Refusal = (i32, String);

/// The accounts the host's own configuration names, and what asks the person for a typed value.
pub(crate) struct Accounts {
    layers: Layers,
    home: PathBuf,
    hub: Arc<Hub>,
    /// How long a question put to the person waits for its answer.
    ask_wait: Duration,
    /// Whether this is the host of a portal, whose every socket only the person's own browser reaches.
    portal: bool,
}

impl core::fmt::Debug for Accounts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Accounts").finish_non_exhaustive()
    }
}

/// A harness the host serves as a source of its own when canoa.cfg has no entry of that name: an entry that offers accounts
/// takes the name, and the source with it.
const BUILT_IN_SOURCES: [(&str, &str); 3] = [
    ("Claude", "claude"),
    ("Codex", "codex"),
    ("OpenCode", "opencode"),
];

/// The entries that run a built-in provider: those canoa.cfg has, then the built-in sources it has not named. A built-in source
/// reaches only this computer's own application, except on the host of a portal, whose sockets only the person's browser reaches.
fn entries_of(config: &Config, portal: bool) -> Vec<Value> {
    let catalogs = Catalogs::new(config);
    let mut entries: Vec<Value> = config
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Harness {
                harness, accounts, ..
            } => Some(json!({
                "name": entry.name,
                "harness": harness,
                "accounts": accounts,
                "open": entry.peers.is_none(),
                "accepts": catalogs.accepts(harness),
                "usable": matching::usable(config, &catalogs, harness),
            })),
            _ => None,
        })
        .collect();
    for (name, harness) in BUILT_IN_SOURCES {
        if config.entries.iter().all(|entry| entry.name != name) {
            entries.push(json!({
                "name": name, "harness": harness, "accounts": [], "open": portal,
                "accepts": catalogs.accepts(harness),
                "usable": matching::usable(config, &catalogs, harness),
            }));
        }
    }
    entries
}

/// The keys an entry made by an offer starts with. A built-in source keeps what it was (its title, the profile it is
/// launched with) and reaches no paired key unless the host is a portal's; any other entry made here is titled by its name and no
/// paired key reaches it.
fn made_keys(entry: &str, built_in: bool, portal: bool) -> Vec<(&'static str, toml::Value)> {
    let mut keys = vec![("title", toml::Value::String(entry.to_owned()))];
    if !(built_in && portal) {
        keys.push(("peers", toml::Value::Array(Vec::new())));
        // The profile is the person's own for a built-in source; a portal host gives it to its harness entries whatever the file
        // says, and a file may ask for it only of an entry that names its keys.
        if built_in {
            keys.push(("own_profile", toml::Value::Boolean(true)));
        }
    }
    keys
}

/// The entry that is the agent itself for `harness`: the one canoa.cfg names that runs it (the built-in source's name first),
/// else the built-in source's name, else the harness's name with a capital.
fn own_entry_of(config: &Config, harness: &str, apart: &str) -> String {
    let source = BUILT_IN_SOURCES
        .iter()
        .find(|(_, runs)| *runs == harness)
        .map(|(name, _)| (*name).to_owned());
    let named = |entry: &crate::config::Entry| {
        entry.name != apart
            && matches!(&entry.kind, EntryKind::Harness { harness: runs, .. } if runs == harness)
    };
    if let Some(source) = &source
        && config
            .entries
            .iter()
            .any(|entry| entry.name == *source && named(entry))
    {
        return source.clone();
    }
    if let Some(entry) = config.entries.iter().find(|entry| named(entry)) {
        return entry.name.clone();
    }
    source.unwrap_or_else(|| {
        let mut letters = harness.chars();
        let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
        format!("{}{}", first.unwrap_or_default(), letters.as_str())
    })
}

/// Whether `entry` is one an earlier version of this application made for a provider's accounts, and nothing the person wrote: named
/// `<harness>-accounts`, titled `<Harness> accounts`, reaching no paired key, with no other setting.
fn app_made_accounts_entry(entry: &crate::config::Entry, harness: &str) -> Option<Vec<String>> {
    let EntryKind::Harness {
        harness: runs,
        origin,
        executable,
        options,
        settings,
        account_settings,
        profiles,
        accounts,
    } = &entry.kind
    else {
        return None;
    };
    let mut letters = harness.chars();
    let first = letters.next()?.to_uppercase().collect::<String>();
    let title = format!("{first}{} accounts", letters.as_str());
    let plain = runs == harness
        && entry.name == format!("{harness}-accounts")
        && entry.title.as_deref() == Some(title.as_str())
        && entry.peers.as_ref().is_some_and(Vec::is_empty)
        && !entry.own_profile
        && !entry.local
        && origin.is_none()
        && executable.is_none()
        && *options == crate::config::HarnessOptions::default()
        && settings.is_none()
        && account_settings.is_none()
        && profiles.is_empty();
    plain.then(|| accounts.clone())
}

/// Folds the accounts of an entry an earlier version of this application made (`claude-accounts`, titled "Claude accounts") into the
/// agent's own entry, and takes the made entry out of canoa.cfg. An entry the person wrote, or one in another layer's file, is left as it
/// is. Returns whether the file changed.
pub(crate) fn fold_made_entries(
    own: &std::path::Path,
    config: &Config,
    portal: bool,
) -> Result<bool, Refusal> {
    let mut changed = false;
    for harness in crate::config::BUILT_IN_HARNESSES {
        let made = format!("{harness}-accounts");
        let Some(entry) = config.entries.iter().find(|entry| entry.name == made) else {
            continue;
        };
        let Some(theirs) = app_made_accounts_entry(entry, harness) else {
            continue;
        };
        if !commands::has_entry(own, &made).map_err(|failed| refusal(&failed))? {
            continue;
        }
        let target = own_entry_of(config, harness, &made);
        let mut names = config
            .entries
            .iter()
            .find(|each| each.name == target)
            .and_then(|each| match &each.kind {
                EntryKind::Harness { accounts, .. } => Some(accounts.clone()),
                _ => None,
            })
            .unwrap_or_default();
        for name in theirs {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let built_in = config.entries.iter().all(|each| each.name != target)
            && BUILT_IN_SOURCES.iter().any(|(name, _)| *name == target);
        let keys = made_keys(&target, built_in, portal);
        commands::set_offered(own, &target, harness, &names, &keys)
            .map_err(|failed| refusal(&failed))?;
        commands::remove_entry(own, &made).map_err(|failed| refusal(&failed))?;
        changed = true;
    }
    Ok(changed)
}

/// `accounts/offer` `{"entry", "accounts", "harness"?}`: the entry offers exactly those accounts, in that order, which canoa.cfg
/// then says. An entry canoa.cfg does not have is made when it is a built-in source, or when `harness` says what it runs. The host
/// reads an entry's offer when it starts, so the answer says it has to start again.
fn offer(
    own: &std::path::Path,
    config: &Config,
    portal: bool,
    params: &Value,
) -> Result<Value, Refusal> {
    let entry = text(params, "entry")?;
    let names: Vec<String> = match params.get("accounts") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or((BAD_PARAMS, "accounts is a list of names".to_owned()))?,
        _ => return Err((BAD_PARAMS, "accounts is a list of names".to_owned())),
    };
    if entry.is_empty()
        || entry.len() > 64
        || !entry
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err((
            BAD_PARAMS,
            "an entry's name is 1 to 64 letters, digits, - or _".to_owned(),
        ));
    }
    let known = entries_of(config, portal);
    let in_file = config.entries.iter().any(|each| each.name == entry);
    let built_in = !in_file && BUILT_IN_SOURCES.iter().any(|(name, _)| *name == entry);
    let harness = match known.iter().find(|each| each["name"] == entry) {
        Some(each) => each["harness"].as_str().unwrap_or_default().to_owned(),
        None => {
            let asked = text(params, "harness").ok();
            asked
                .filter(|harness| crate::config::BUILT_IN_HARNESSES.contains(harness))
                .map(str::to_owned)
                .ok_or((
                    NOT_FOUND,
                    format!("this host has no entry {entry} that runs an account's provider"),
                ))?
        }
    };
    let harness = harness.as_str();
    let mut catalogs = None;
    for (index, name) in names.iter().enumerate() {
        let account = config
            .accounts
            .get(name)
            .ok_or((NOT_FOUND, format!("no account called {name}")))?;
        if account.harness != harness {
            let catalogs = catalogs.get_or_insert_with(|| Catalogs::new(config));
            let fit = match (catalogs.of(harness), catalogs.kind(account)) {
                (Some(catalog), Some(info)) => matching::fits(&catalog, harness, account, &info),
                _ => Err(format!(
                    "{harness} does not list what {} provides",
                    account.kind
                )),
            };
            if let Err(why) = fit {
                return Err((
                    REFUSED,
                    format!(
                        "{name} signs in to {}, and {entry} runs {harness}: {why}",
                        account.harness
                    ),
                ));
            }
        }
        if names[..index].contains(name) {
            return Err((BAD_PARAMS, format!("{name} is listed twice")));
        }
    }
    let made = made_keys(entry, built_in, portal);
    commands::set_offered(own, entry, harness, &names, &made).map_err(|failed| refusal(&failed))?;
    Ok(json!({"entry": entry, "accounts": names, "restart": true}))
}

/// Which sign-in a long request asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Long {
    Add,
    Login,
}

impl Long {
    pub(crate) fn of(method: &str) -> Option<Self> {
        match method {
            "accounts/add" => Some(Self::Add),
            "accounts/login" => Some(Self::Login),
            _ => None,
        }
    }
}

fn refusal(failed: &Failed) -> Refusal {
    (
        if failed.status == 2 {
            BAD_PARAMS
        } else {
            REFUSED
        },
        failed.why.clone(),
    )
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (BAD_PARAMS, format!("{key} is missing or not text")))
}

fn inputs_of(params: &Value) -> Result<Map<String, Value>, Refusal> {
    match params.get("inputs") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err((BAD_PARAMS, "inputs is an object".to_owned())),
    }
}

/// The value as JSON, for a type that converts to a Guatiao value.
fn json_of<T: ToValue>(value: &T) -> Result<Value, Refusal> {
    let value = value
        .to_value()
        .map_err(|error| (REFUSED, error.to_string()))?;
    serde_json::to_value(guatiao_serde::Serializable::from(&value))
        .map_err(|error| (REFUSED, error.to_string()))
}

impl Accounts {
    pub(crate) fn new(
        layers: Layers,
        home: PathBuf,
        hub: Arc<Hub>,
        ask_wait: Duration,
        portal: bool,
    ) -> Self {
        Self {
            layers,
            home,
            hub,
            ask_wait,
            portal,
        }
    }

    /// The host's home, where its accounts are kept.
    pub(super) fn home_dir(&self) -> &std::path::Path {
        &self.home
    }

    /// The canoa.cfg files the accounts are listed in.
    pub(super) const fn layers(&self) -> &Layers {
        &self.layers
    }

    /// Whether this is the host of a portal.
    pub(super) fn portal(&self) -> bool {
        self.portal
    }

    /// canoa.cfg as it is on disk now: an app's earlier sign-in wrote to it after this host started.
    pub(super) fn config(&self) -> Result<Config, Refusal> {
        let config = self.load()?;
        // An entry an earlier version made for accounts is folded into the agent's own once, the first time this is asked.
        match fold_made_entries(&self.layers.own, &config, self.portal) {
            Ok(true) => self.load(),
            Ok(false) => Ok(config),
            Err((_, why)) => {
                tracing::warn!("canoa.cfg's accounts entry was left as it is: {why}");
                Ok(config)
            }
        }
    }

    fn load(&self) -> Result<Config, Refusal> {
        match Config::load_layers(&self.layers) {
            Err(crate::ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                let dir = self
                    .layers
                    .own
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""));
                Config::from_str_in("", dir).map_err(|error| (REFUSED, error.to_string()))
            }
            loaded => loaded.map_err(|error| (REFUSED, error.to_string())),
        }
    }

    /// `accounts/catalog`, `accounts/list` and `accounts/remove`.
    pub(crate) fn call(&self, method: &str, params: &Value) -> Result<Value, Refusal> {
        let config = self.config()?;
        let home = Home::at(self.home.clone());
        let cfg = Cfg {
            config_path: &self.layers.own,
            config: &config,
            home: &home,
        };
        match method {
            "accounts/catalog" => {
                let harness = text(params, "harness")?;
                let (kinds, logins) = commands::catalog_of(&config, harness).map_err(|failed| {
                    if failed.status == 2 {
                        (NOT_FOUND, failed.why)
                    } else {
                        refusal(&failed)
                    }
                })?;
                let kinds = kinds.iter().map(json_of).collect::<Result<Vec<_>, _>>()?;
                let logins = logins.iter().map(json_of).collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"harness": harness, "kinds": kinds, "logins": logins}))
            }
            "accounts/list" => {
                let rows = commands::rows(cfg).map_err(|failed| refusal(&failed))?;
                let rows: Vec<Value> = rows.iter().map(commands::Row::json).collect();
                Ok(json!({"accounts": rows}))
            }
            "accounts/remove" => {
                commands::remove_with(cfg, text(params, "name")?)
                    .map_err(|failed| refusal(&failed))?;
                Ok(json!({"removed": true}))
            }
            "accounts/entries" => Ok(json!({"entries": entries_of(&config, self.portal)})),
            "accounts/offer" => offer(&self.layers.own, &config, self.portal, params),
            _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
        }
    }

    /// `accounts/add` or `accounts/login` on `channel`, which carries `accounts/progress` and the form's request until the
    /// answer. The link closing, or the ask wait (four minutes) with no answer to a question, cancels the sign-in. Other requests on the link
    /// are answered as they come by `others`.
    pub(crate) fn sign_in<C: Channel>(
        &self,
        long: Long,
        request: &Value,
        (channel, peer): (&C, Option<&str>),
        others: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
    ) -> Vec<u8> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let started = (|| {
            let asker = self.hub.asker_of(peer).ok_or((
                REFUSED,
                "this host has no secret to seal a typed answer with, so it cannot sign in"
                    .to_owned(),
            ))?;
            let name = text(&params, "name")?.to_owned();
            let harness = match long {
                Long::Add => Some(text(&params, "harness")?.to_owned()),
                Long::Login => None,
            };
            let method = match long {
                Long::Add => Some(text(&params, "method")?.to_owned()),
                Long::Login => params
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            Ok::<_, Refusal>((
                asker,
                name,
                harness,
                method,
                inputs_of(&params)?,
                self.config()?,
            ))
        })();
        let (asker, name, harness, method, inputs, config) = match started {
            Ok(started) => started,
            Err((code, message)) => return error_line(&id, code, &message),
        };
        let home = Home::at(self.home.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let (events, incoming) = mpsc::channel();
        let relay = Arc::new(Relay::new(events, Arc::clone(&cancel)));
        let mut asking: Option<Pending> = None;
        thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let cfg = Cfg {
                    config_path: &self.layers.own,
                    config: &config,
                    home: &home,
                };
                let mut sign = |accounts: &crate::accounts::Accounts,
                                provider: &crate::kind::Harness,
                                method: &str| {
                    commands::sign_in_relayed(
                        accounts,
                        provider,
                        method,
                        &inputs,
                        Arc::clone(&relay),
                    )
                };
                match (long, harness.as_deref()) {
                    (Long::Add, Some(harness)) => commands::add_with(
                        cfg,
                        &name,
                        harness,
                        method.as_deref(),
                        &inputs,
                        &mut sign,
                    ),
                    _ => commands::login_with(cfg, &name, method.as_deref(), &inputs, &mut sign),
                }
            });
            loop {
                let finished = worker.is_finished();
                while let Ok(event) = incoming.try_recv() {
                    self.event(event, &id, channel, &asker, &mut asking, &cancel);
                }
                if finished {
                    break;
                }
                if let Ok(event) = incoming.recv_timeout(POLL) {
                    self.event(event, &id, channel, &asker, &mut asking, &cancel);
                }
                if asking
                    .as_ref()
                    .is_some_and(|pending| pending.since.elapsed() > self.ask_wait)
                {
                    cancel.store(true, Ordering::SeqCst);
                    if let Some(pending) = asking.take() {
                        let _ = pending
                            .reply
                            .send(Err("the person gave no answer".to_owned()));
                    }
                }
                match channel.receive(Some(Duration::from_millis(1))) {
                    Ok(Some(line)) => {
                        self.heard(&line, &asker, &mut asking, others, channel, &cancel)
                    }
                    Err(Failure::Timeout) => {}
                    Ok(None) | Err(_) => {
                        cancel.store(true, Ordering::SeqCst);
                        if let Some(pending) = asking.take() {
                            let _ = pending.reply.send(Err("the link ended".to_owned()));
                        }
                    }
                }
            }
            match worker.join() {
                Ok(Ok(added)) => {
                    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": added.json()}))
                        .unwrap_or_default()
                }
                Ok(Err(failed)) => {
                    let (code, message) = refusal(&failed);
                    error_line(&id, code, &message)
                }
                Err(_) => error_line(&id, REFUSED, "the sign-in stopped without an answer"),
            }
        })
    }

    /// What the sign-in says or asks, put on the link.
    fn event<C: Channel>(
        &self,
        event: Event,
        id: &Value,
        channel: &C,
        asker: &Asker,
        asking: &mut Option<Pending>,
        cancel: &AtomicBool,
    ) {
        let progress = |body: Value| {
            let mut params = body;
            params["id"] = id.clone();
            serde_json::to_vec(
                &json!({"jsonrpc": "2.0", "method": "accounts/progress", "params": params}),
            )
            .unwrap_or_default()
        };
        let sent = match event {
            Event::Say(line) => channel.send(&progress(json!({"say": line}))).is_ok(),
            Event::Open(open) => channel
                .send(&progress(json!({"open_url": {
                    "url": open.url, "user_code": open.user_code, "expires_ms": open.expires_ms}})))
                .is_ok(),
            Event::Ask(form, reply) => {
                if asking.is_some() {
                    let _ = reply.send(Err("a question is already open".to_owned()));
                    return;
                }
                match put_form(asker, &form, "accounts", self.ask_wait) {
                    Ok((line, asked)) => {
                        if channel.send(&line).is_ok() {
                            *asking = Some(Pending {
                                asked,
                                reply,
                                since: Instant::now(),
                            });
                            true
                        } else {
                            let _ = reply.send(Err("the link ended".to_owned()));
                            false
                        }
                    }
                    Err(why) => {
                        let _ = reply.send(Err(why));
                        true
                    }
                }
            }
        };
        if !sent {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// A line from the application: the answer to the question put to it, or another request of its own.
    fn heard<C: Channel>(
        &self,
        line: &[u8],
        asker: &Asker,
        asking: &mut Option<Pending>,
        others: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
        channel: &C,
        cancel: &AtomicBool,
    ) {
        let message: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
        if message.get("method").is_some() {
            if let Some(reply) = others(line)
                && channel.send(&reply).is_err()
            {
                cancel.store(true, Ordering::SeqCst);
            }
            return;
        }
        let waiting = asking.take_if(|pending| {
            message.get("id").and_then(Value::as_str) == Some(pending.asked.request_id())
        });
        let Some(Pending { asked, reply, .. }) = waiting else {
            return;
        };
        let answer = match message.get("result") {
            Some(result) => open_answer(asker, asked, result),
            None => Err("the person gave no answer".to_owned()),
        };
        let _ = reply.send(answer);
    }
}

/// The request that puts `form` to the person, sealed to the key `asker` asks, as a line for the link, and what opens the answer.
/// A control request that carries a typed value uses this and no other way to ask for it.
pub(super) fn put_form(
    asker: &Asker,
    form: &LoginForm,
    session: &str,
    wait: Duration,
) -> Result<(Vec<u8>, Asked), String> {
    let schema = inline_refs(&form.schema)
        .ok()
        .and_then(|schema| serde_json::to_string(&guatiao_serde::Serializable::from(&schema)).ok())
        .unwrap_or_default();
    let (params, asked) = asker.ask(session, &form.title, &schema, wait)?;
    let line = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": asked.request_id(), "method": METHOD, "params": params,
    }))
    .map_err(|_| "the form cannot be put to the person".to_owned())?;
    Ok((line, asked))
}

/// What the person typed, from the application's `result` to the request `asked`: opened, and checked to be bound to it.
pub(super) fn open_answer(
    asker: &Asker,
    asked: Asked,
    result: &Value,
) -> Result<LoginAnswer, String> {
    let opened = asker
        .open(asked, result)?
        .ok_or_else(|| "the person gave no answer".to_owned())?;
    let values = map_of(&Settings::new(opened.plain))?;
    let mut secrets = Credentials::new();
    for (key, value) in &opened.secrets {
        secrets.insert(key.clone(), value.to_string());
    }
    Ok(LoginAnswer::new(
        coqui_acp_api::guatiao::Value::from(values),
        secrets,
    ))
}

/// The form put to the person, waiting for what it answers.
struct Pending {
    asked: Asked,
    reply: mpsc::Sender<Result<LoginAnswer, String>>,
    since: Instant,
}

#[cfg(test)]
mod offer_tests {
    use super::*;

    const ACCOUNTS: &str = "[host.accounts.c]\nharness = \"codex\"\nkind = \"openai-api-key\"\nid = \"id-c\"\n\n[host.accounts.w]\nharness = \"claude\"\nkind = \"anthropic-api-key\"\nid = \"id-w\"\n";

    fn scratch(name: &str) -> std::path::PathBuf {
        static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("canoa-offer-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("made");
        dir
    }

    fn offered(portal: bool, params: &Value) -> (Result<Value, Refusal>, String, Config) {
        offered_in(ACCOUNTS, portal, params)
    }

    fn offered_in(
        text: &str,
        portal: bool,
        params: &Value,
    ) -> (Result<Value, Refusal>, String, Config) {
        let dir = scratch(if portal { "portal" } else { "plain" });
        let path = dir.join("canoa.cfg");
        std::fs::write(&path, text).expect("written");
        let config = Config::from_str_in(text, &dir).expect("parses");
        let answer = offer(&path, &config, portal, params);
        let written = std::fs::read_to_string(&path).expect("read");
        let after = Config::from_str_in(&written, &dir).expect("the edited file reads");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        (answer, written, after)
    }

    fn entry<'a>(config: &'a Config, name: &str) -> &'a crate::config::Entry {
        config
            .entries
            .iter()
            .find(|each| each.name == name)
            .expect(name)
    }

    #[test]
    fn a_built_in_source_keeps_what_it_was_and_reaches_no_paired_key_until_it_is_the_portals() {
        let params = json!({"entry": "Codex", "accounts": ["c"]});
        let (answer, written, after) = offered(false, &params);
        answer.expect("offered");
        let codex = entry(&after, "Codex");
        assert_eq!(codex.peers.as_ref().map(Vec::len), Some(0), "{written}");
        assert!(codex.own_profile, "{written}");
        assert!(!written.contains("bypass"), "{written}");
        assert!(written.contains("title = \"Codex\""), "{written}");
        let (answer, written, after) = offered(true, &params);
        answer.expect("offered");
        assert!(entry(&after, "Codex").peers.is_none(), "{written}");
        assert!(!written.contains("own_profile"), "{written}");
    }

    #[test]
    fn an_offer_to_a_provider_with_no_entry_makes_the_agent_itself_not_an_accounts_entry() {
        let params = json!({"entry": "Pi", "harness": "pi", "accounts": []});
        let (answer, written, _) = offered(true, &params);
        answer.expect("offered");
        assert!(!written.contains("accounts\""), "{written}");
        let params = json!({"entry": "Pi", "harness": "pi", "accounts": ["p"]});
        let text = format!(
            "{ACCOUNTS}
[host.accounts.p]
harness = \"pi\"
kind = \"pi-oauth\"
id = \"id-p\"
"
        );
        let (answer, written, after) = offered_in(&text, false, &params);
        answer.expect("offered");
        let made = entry(&after, "Pi");
        assert_eq!(made.peers.as_ref().map(Vec::len), Some(0), "{written}");
        assert!(written.contains("title = \"Pi\""), "{written}");
        assert!(!written.contains("Pi accounts"), "{written}");
        let (answer, ..) = offered(false, &json!({"entry": "nowhere", "accounts": []}));
        assert_eq!(answer.expect_err("no such entry").0, NOT_FOUND);
    }

    #[test]
    fn offering_to_claude_edits_claude_and_makes_no_second_entry() {
        let params = json!({"entry": "Claude", "accounts": ["w"]});
        let (answer, written, after) = offered(false, &params);
        answer.expect("offered");
        assert_eq!(after.entries.len(), 1, "{written}");
        assert!(!written.contains("claude-accounts"), "{written}");
        assert!(!written.contains("Claude accounts"), "{written}");
        assert!(written.contains("accounts = [\"w\"]"), "{written}");
    }

    const MADE: &str = "
[host.agents.claude-accounts]
harness = \"claude\"
title = \"Claude accounts\"
peers = []
accounts = [\"w\", \"x\"]
";

    #[test]
    fn an_entry_an_earlier_version_made_is_folded_into_the_agents_own_once() {
        let text = format!(
            "{ACCOUNTS}
[host.accounts.x]
harness = \"claude\"
kind = \"anthropic-api-key\"
id = \"id-x\"
{MADE}"
        );
        let dir = scratch("fold");
        let path = dir.join("canoa.cfg");
        std::fs::write(&path, &text).expect("written");
        let layers = Layers::only(path.clone());
        let config = Config::load_layers(&layers).expect("reads");
        assert!(fold_made_entries(&path, &config, false).expect("folds"));
        let written = std::fs::read_to_string(&path).expect("read");
        let after = Config::from_str_in(&written, &dir).expect("reads");
        assert!(
            after
                .entries
                .iter()
                .all(|each| each.name != "claude-accounts"),
            "{written}"
        );
        let claude = entry(&after, "Claude");
        match &claude.kind {
            EntryKind::Harness { accounts, .. } => assert_eq!(accounts, &["w", "x"], "{written}"),
            _ => panic!("{written}"),
        }
        assert_eq!(claude.peers.as_ref().map(Vec::len), Some(0), "{written}");
        let config = Config::load_layers(&layers).expect("reads");
        assert!(!fold_made_entries(&path, &config, false).expect("nothing left"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), written);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn an_entry_of_the_persons_with_that_name_is_never_folded() {
        // Another title, a key the app never writes, or paired keys it names: the person's own.
        for made in [
            MADE.replace("Claude accounts", "My work Claude"),
            format!(
                "{MADE}model = \"x\"
"
            ),
            MADE.replace("peers = []", "peers = [\"abc\"]"),
        ] {
            let text = format!("{ACCOUNTS}{made}");
            let dir = scratch("kept");
            let path = dir.join("canoa.cfg");
            std::fs::write(&path, &text).expect("written");
            let Ok(config) = Config::load_layers(&Layers::only(path.clone())) else {
                std::fs::remove_dir_all(&dir).expect("cleanup");
                continue;
            };
            assert!(
                !fold_made_entries(&path, &config, false).expect("left"),
                "{made}"
            );
            assert_eq!(std::fs::read_to_string(&path).expect("read"), text);
            std::fs::remove_dir_all(&dir).expect("cleanup");
        }
    }

    #[test]
    fn a_built_in_source_is_open_only_on_a_portal() {
        let config = Config::from_str_in(ACCOUNTS, &std::env::temp_dir()).expect("parses");
        for (portal, open) in [(false, false), (true, true)] {
            let entries = entries_of(&config, portal);
            let claude = entries
                .iter()
                .find(|each| each["name"] == "Claude")
                .expect("Claude");
            assert_eq!(claude["open"], open, "portal {portal}");
        }
    }
}
