//! Accounts on the control channel, the owner's alone: `accounts/providers`, `accounts/catalog`, `accounts/list`, `accounts/remove`, and the two
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

use super::params::{Refusal, refusal, string_list, text};
use super::{BAD_PARAMS, NOT_FOUND, POLL, REFUSED, error_line};
use crate::accounts::commands::{self, Cfg, Event, Relay};
use crate::accounts::matching::{self, Catalogs};
use crate::config::{Config, EntryKind, Layers, Settings};
use crate::home::Home;
use crate::provider::map_of;
use crate::secrets::Hub;
use crate::secrets::ask::{Asked, Asker, METHOD};

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
                "pools": pools_of(config, &catalogs, harness, accounts),
            })),
            _ => None,
        })
        .collect();
    for (name, harness) in crate::bundled::harness_sources() {
        if config.entries.iter().all(|entry| entry.name != name) {
            entries.push(json!({
                "name": name, "harness": harness, "accounts": [], "open": portal,
                "accepts": catalogs.accepts(harness),
                "usable": matching::usable(config, &catalogs, harness),
                "pools": pools_of(config, &catalogs, harness, &[]),
            }));
        }
    }
    entries
}

/// The pools an entry of `harness` offers, as `accounts/entries` says them: each with its standard, whether the host made it,
/// whether the entry names it, and its accounts, each saying whether the entry offers it by itself. The host's `any-<standard>`
/// is left out while fewer than two accounts hold it, though a session that names it still resolves it. An entry that names a pool
/// its harness cannot take has none listed here and does not start.
fn pools_of(
    config: &Config,
    catalogs: &Catalogs<'_>,
    harness: &str,
    names: &[String],
) -> Vec<Value> {
    crate::pools::for_entry(config, catalogs, harness, names)
        .unwrap_or_default()
        .iter()
        .filter(|pool| crate::pools::worth_listing(pool.automatic, pool.members.len()))
        .map(|pool| {
            json!({
                "name": pool.name,
                "standard": pool.standard,
                "automatic": pool.automatic,
                "listed": pool.listed,
                "accounts": pool.members.iter()
                    .map(|member| json!({"name": member.name, "listed": member.listed}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// `accounts/providers`: every account provider this host carries, once: each agent's own and each maker. A row says its id, its title
/// and what it is, the agent it belongs to (`harness`, `null` for a maker, which no agent owns), the kinds it makes and its own
/// ways to sign in, and which kinds report usage.
fn providers_of(config: &Config) -> Vec<Value> {
    crate::accounts::providers::all(config)
        .into_iter()
        .map(|listed| {
            json!({
                "id": listed.id,
                "title": listed.title,
                "description": listed.description,
                "harness": listed.harness,
                "kinds": listed.offer.kinds.iter()
                    .map(|kind| json!({"id": kind.id, "title": kind.title, "types": kind.provides}))
                    .collect::<Vec<_>>(),
                "logins": listed.offer.logins.iter()
                    .map(|login| json!({"id": login.id, "title": login.title, "produces": login.produces}))
                    .collect::<Vec<_>>(),
                "usage": listed.offer.usage,
                "local": listed.offer.local,
            })
        })
        .collect()
}

/// The keys an entry made by an offer starts with. A built-in source keeps what it was (its title) and reaches no paired key
/// unless the host is a portal's; any other entry made here is titled by its name and no paired key reaches it.
fn made_keys(entry: &str, built_in: bool, portal: bool) -> Vec<(&'static str, toml::Value)> {
    let mut keys = vec![("title", toml::Value::String(entry.to_owned()))];
    if !(built_in && portal) {
        keys.push(("peers", toml::Value::Array(Vec::new())));
    }
    keys
}

fn offer(
    own: &std::path::Path,
    config: &Config,
    portal: bool,
    params: &Value,
) -> Result<Value, Refusal> {
    let entry = text(params, "entry")?;
    let names = string_list(params, "accounts")?;
    crate::config::check_name(entry).map_err(|error| (BAD_PARAMS, error.to_string()))?;
    let known = entries_of(config, portal);
    let in_file = config.entries.iter().any(|each| each.name == entry);
    let built_in = !in_file && crate::bundled::harness_sources().any(|(name, _)| name == entry);
    let harness = match known.iter().find(|each| each["name"] == entry) {
        Some(each) => each["harness"].as_str().unwrap_or_default().to_owned(),
        None => {
            let asked = text(params, "harness").ok();
            asked
                .filter(|harness| crate::config::is_built_in_harness(harness))
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
        if crate::pools::is_pool(config, name) {
            if names[..index].contains(name) {
                return Err((BAD_PARAMS, format!("{name} is listed twice")));
            }
            let catalogs = catalogs.get_or_insert_with(|| Catalogs::new(config));
            crate::pools::expand(config, catalogs, harness, core::slice::from_ref(name))
                .map_err(|why| (REFUSED, format!("{entry} runs {harness}: {why}")))?;
            continue;
        }
        let account = config
            .accounts
            .get(name)
            .ok_or((NOT_FOUND, format!("no account called {name}")))?;
        let makers = crate::makers::Makers::of(config);
        let catalogs = catalogs.get_or_insert_with(|| Catalogs::new(config));
        if matching::borrowed(&makers, account, harness, catalogs.of(harness).as_ref()) {
            let fit = match (catalogs.of(harness), catalogs.kind(account)) {
                (Some(catalog), Some(info)) => {
                    matching::fits(&makers, &catalog, harness, account, &info)
                }
                _ => Err(format!(
                    "{harness} does not list what {} provides",
                    account.kind
                )),
            };
            if let Err(why) = fit {
                return Err((
                    REFUSED,
                    format!(
                        "{name} is a {} account, and {entry} runs {harness}: {why}",
                        account.kind
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
        self.load()
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
            "accounts/providers" => Ok(json!({"providers": providers_of(&config)})),
            "accounts/catalog" => {
                let provider = text(params, "provider")?;
                let (kinds, logins) =
                    commands::catalog_of(&config, provider).map_err(|failed| {
                        if failed.status == 2 {
                            (NOT_FOUND, failed.why)
                        } else {
                            refusal(&failed)
                        }
                    })?;
                let kinds = kinds.iter().map(json_of).collect::<Result<Vec<_>, _>>()?;
                let logins = logins.iter().map(json_of).collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"provider": provider, "kinds": kinds, "logins": logins}))
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
            "pools/list" => Ok(super::pools::list(&config)),
            "pools/set" => super::pools::set(&self.layers.own, &config, params),
            "pools/remove" => super::pools::remove(&self.layers.own, &config, params),
            "pools/turn" => super::pools::turn(self, params),
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
            let provider = match long {
                Long::Add => Some(text(&params, "provider")?.to_owned()),
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
                provider,
                method,
                inputs_of(&params)?,
                self.config()?,
            ))
        })();
        let (asker, name, provider, method, inputs, config) = match started {
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
                                provider: &commands::Signing<'_>,
                                method: &str| {
                    commands::sign_in_relayed(
                        accounts,
                        provider,
                        method,
                        &inputs,
                        Arc::clone(&relay),
                    )
                };
                match (long, provider.as_deref()) {
                    (Long::Add, Some(provider)) => commands::add_with(
                        cfg,
                        &name,
                        provider,
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

    const ACCOUNTS: &str = "[host.accounts.c]\nkind = \"codex-api-key\"\nid = \"id-c\"\n\n[host.accounts.w]\nkind = \"anthropic-api-key\"\nid = \"id-w\"\n";

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
        assert!(!written.contains("own_profile"), "{written}");
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
