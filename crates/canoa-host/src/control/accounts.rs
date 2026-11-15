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
use coqui_acp_api::{Credentials, LoginAnswer};
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value, json};

use super::{BAD_PARAMS, NOT_FOUND, POLL, REFUSED, error_line};
use crate::accounts::commands::{self, Cfg, Event, Relay};
use crate::cli::Failed;
use crate::config::{Config, EntryKind, Layers, Settings};
use crate::home::Home;
use crate::provider::map_of;
use crate::secrets::Hub;
use crate::secrets::ask::{Asked, Asker, METHOD};

/// How long a question put to the person waits for its answer.
const ASK_WAIT: Duration = Duration::from_secs(600);

type Refusal = (i32, String);

/// The accounts the host's own configuration names, and what asks the person for a typed value.
pub(crate) struct Accounts {
    layers: Layers,
    home: PathBuf,
    hub: Arc<Hub>,
}

impl core::fmt::Debug for Accounts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Accounts").finish_non_exhaustive()
    }
}

/// A harness the host serves as a source of its own when canoa.cfg has no entry of that name: an entry that offers accounts
/// takes the name, and the source with it.
const BUILT_IN_SOURCES: [(&str, &str); 2] = [("Claude", "claude"), ("Codex", "codex")];

/// The entries that run a built-in provider: those canoa.cfg has, then the built-in sources it has not named.
fn entries_of(config: &Config) -> Vec<Value> {
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
            })),
            _ => None,
        })
        .collect();
    for (name, harness) in BUILT_IN_SOURCES {
        if config.entries.iter().all(|entry| entry.name != name) {
            entries.push(json!({
                "name": name, "harness": harness, "accounts": [], "open": true,
            }));
        }
    }
    entries
}

/// `accounts/offer` `{"entry", "accounts"}`: the entry offers exactly those accounts, in that order, which canoa.cfg then says. The
/// host reads an entry's offer when it starts, so the answer says it has to start again.
fn offer(own: &std::path::Path, config: &Config, params: &Value) -> Result<Value, Refusal> {
    let entry = text(params, "entry")?;
    let names: Vec<String> = match params.get("accounts") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or((BAD_PARAMS, "accounts is a list of names".to_owned()))?,
        _ => return Err((BAD_PARAMS, "accounts is a list of names".to_owned())),
    };
    let known = entries_of(config);
    let harness = known
        .iter()
        .find(|each| each["name"] == entry)
        .and_then(|each| each["harness"].as_str())
        .ok_or((
            NOT_FOUND,
            format!("this host has no entry {entry} that runs an account's provider"),
        ))?;
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
    for (index, name) in names.iter().enumerate() {
        let account = config
            .accounts
            .get(name)
            .ok_or((NOT_FOUND, format!("no account called {name}")))?;
        if account.harness != harness {
            return Err((
                REFUSED,
                format!(
                    "{name} signs in to {}, and {entry} runs {harness}",
                    account.harness
                ),
            ));
        }
        if names[..index].contains(name) {
            return Err((BAD_PARAMS, format!("{name} is listed twice")));
        }
    }
    commands::set_offered(own, entry, harness, &names).map_err(|failed| refusal(&failed))?;
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
    pub(crate) fn new(layers: Layers, home: PathBuf, hub: Arc<Hub>) -> Self {
        Self { layers, home, hub }
    }

    /// canoa.cfg as it is on disk now: an app's earlier sign-in wrote to it after this host started.
    fn config(&self) -> Result<Config, Refusal> {
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
            "accounts/entries" => Ok(json!({"entries": entries_of(&config)})),
            "accounts/offer" => offer(&self.layers.own, &config, params),
            _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
        }
    }

    /// `accounts/add` or `accounts/login` on `channel`, which carries `accounts/progress` and the form's request until the
    /// answer. The link closing, or ten minutes with no answer to a question, cancels the sign-in. Other requests on the link
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
            let asker = match peer {
                Some(peer) => self.hub.asker_for(peer),
                None => self.hub.asker(),
            }
            .ok_or((
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
                    .is_some_and(|pending| pending.since.elapsed() > ASK_WAIT)
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
                let schema = inline_refs(&form.schema)
                    .ok()
                    .and_then(|schema| {
                        serde_json::to_string(&guatiao_serde::Serializable::from(&schema)).ok()
                    })
                    .unwrap_or_default();
                match asker.ask(&form.title, &schema) {
                    Ok((params, asked)) => {
                        let line = serde_json::to_vec(&json!({
                            "jsonrpc": "2.0", "id": asked.request_id(), "method": METHOD, "params": params,
                        }))
                        .unwrap_or_default();
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
            Some(result) => asker.open(asked, result).and_then(|opened| {
                let opened = opened.ok_or_else(|| "the person gave no answer".to_owned())?;
                let values = map_of(&Settings::new(opened.plain))?;
                let mut secrets = Credentials::new();
                for (key, value) in &opened.secrets {
                    secrets.insert(key.clone(), value.to_string());
                }
                Ok(LoginAnswer::new(
                    coqui_acp_api::guatiao::Value::from(values),
                    secrets,
                ))
            }),
            None => Err("the person gave no answer".to_owned()),
        };
        let _ = reply.send(answer);
    }
}

/// The form put to the person, waiting for what it answers.
struct Pending {
    asked: Asked,
    reply: mpsc::Sender<Result<LoginAnswer, String>>,
    since: Instant,
}
