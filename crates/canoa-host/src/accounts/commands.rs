//! `canoa accounts list|add|login|remove`: what a person does with accounts, and what an application hosting the library
//! runs as jobs. The credential never passes through a command line: a typed key is read from the terminal without echo,
//! or from standard input when that is not one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, threads and channels are std's"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use coqui_acp_api::guatiao::schema::{SchemaRef, inline_refs};
use coqui_acp_api::{AccountKindInfo, LoginAnswer, LoginForm, LoginMethodInfo, OpenUrl};

use super::login::{self, Interaction};
use super::{Accounts, Refusal, check_settings};
use crate::cli::{Failed, Format, Front};
use crate::config::{AccountConfig, BUILT_IN_HARNESSES, Config, EntryKind, Settings, check_name};
use crate::home::Home;
use crate::{kind, serve};

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

/// What a sign-in asks of its front, passed to the thread that owns it.
pub(crate) enum Event {
    Say(String),
    Open(OpenUrl),
    Ask(LoginForm, mpsc::Sender<Result<LoginAnswer, String>>),
}

/// The sign-in's side of the conversation: it queues what the provider says and waits for the answer to a question.
pub(crate) struct Relay {
    events: Mutex<mpsc::Sender<Event>>,
    cancelled: Arc<AtomicBool>,
}

impl Relay {
    pub(crate) const fn new(events: mpsc::Sender<Event>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            events: Mutex::new(events),
            cancelled,
        }
    }

    fn send(&self, event: Event) -> bool {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(event)
            .is_ok()
    }
}

impl Interaction for Relay {
    fn say(&self, text: &str) {
        self.send(Event::Say(text.to_owned()));
    }

    fn open_url(&self, open: &OpenUrl) {
        self.send(Event::Open(open.clone()));
    }

    fn ask(&self, form: &LoginForm) -> Result<LoginAnswer, String> {
        let (reply, answer) = mpsc::channel();
        if !self.send(Event::Ask(form.clone(), reply)) {
            return Err("the sign-in is over".to_owned());
        }
        loop {
            match answer.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => return answer,
                Err(mpsc::RecvTimeoutError::Timeout) if self.cancelled() => {
                    return Err("login cancelled".to_owned());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("the sign-in is over".to_owned());
                }
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Why a typed answer cannot be taken, when the front has no way to take one.
const NO_TYPING: &str = "this way of running canoa cannot take a typed secret: sealed prompts are not available to it yet, so run `canoa accounts add` in a terminal, or add the account in the app";

/// Runs the sign-in `method` of `harness` and answers what it made, showing its prompts through `env.front`. `cancel` is set
/// when the person gives up.
fn sign_in(
    env: &Env<'_>,
    accounts: &Accounts,
    provider: &kind::Harness,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    format: Format,
    cancel: &Cancel,
) -> Result<coqui_acp_api::LoginOutcome, Failed> {
    let (events, incoming) = mpsc::channel();
    let relay = Arc::new(Relay::new(events, Arc::clone(&cancel.flag)));
    let front = env.front;
    thread::scope(|scope| {
        let pump = scope.spawn(move || {
            for event in incoming {
                match event {
                    Event::Say(text) => show(front, format, "say", &text, None),
                    Event::Open(open) => show(
                        front,
                        format,
                        "open_url",
                        &open.url,
                        Some((open.user_code.as_deref(), open.expires_ms)),
                    ),
                    Event::Ask(form, reply) => {
                        let answer = login::answer_form(&form, |title, secret| {
                            front
                                .ask(&format!("{title}: "), secret)
                                .map_err(|error| error.to_string())?
                                .ok_or_else(|| NO_TYPING.to_owned())
                        });
                        let _ = reply.send(answer);
                    }
                }
            }
        });
        let outcome = login::sign_in(
            accounts,
            &**provider.provider(),
            method,
            inputs,
            relay.clone(),
            None,
        );
        // The pump ends when the relay, which holds the only sender, is dropped.
        drop(relay);
        let _ = pump.join();
        // The sign-in's directory went with it.
        cancel.finish();
        outcome.map_err(|error| Failed::run(error.to_string()))
    })
}

/// One thing the sign-in said, as a line the person reads or a JSON line an application reads.
fn show(
    front: &dyn Front,
    format: Format,
    kind: &str,
    text: &str,
    open: Option<(Option<&str>, Option<u64>)>,
) {
    match format {
        Format::Json => {
            let line = match (kind, open) {
                ("open_url", Some((code, expires))) => serde_json::json!({"open_url": {
                    "url": text, "user_code": code, "expires_ms": expires}}),
                _ => serde_json::json!({ kind: text }),
            };
            let _ = front.print(&line.to_string());
        }
        Format::Text => match open {
            Some((code, _)) => {
                front.say(&format!("Open this page: {text}"));
                if let Some(code) = code {
                    front.say(&format!("Confirm this code there: {code}"));
                }
            }
            None => front.say(text),
        },
    }
}

/// Whether the person has given up on a sign-in, and whether it has cleaned up after itself.
struct Cancel {
    flag: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl Cancel {
    /// The sign-in has ended and removed what it made: whoever is waiting to end the process may.
    fn finish(&self) {
        *self.done.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.done.1.notify_all();
    }
}

/// How long ending the process on a cancel waits for the sign-in to remove what it made, under the five seconds after
/// which a process that ends on its input closing ends whatever is left.
const CLEANUP_WAIT: Duration = Duration::from_secs(3);

/// Cancels a sign-in when the front's input closes, for an application that runs the command as a job, and, before a front
/// that ends the process afterwards does, waits for the sign-in to clean up.
fn cancel_on_input_closing(front: &dyn Front, until: bool) -> Result<Cancel, Failed> {
    let cancel = Cancel {
        flag: Arc::new(AtomicBool::new(false)),
        done: Arc::new((Mutex::new(false), Condvar::new())),
    };
    if until {
        let (flag, done) = (Arc::clone(&cancel.flag), Arc::clone(&cancel.done));
        front
            .when_input_closes(
                1,
                Box::new(move || {
                    flag.store(true, Ordering::SeqCst);
                    let ended = done.0.lock().unwrap_or_else(PoisonError::into_inner);
                    let _ = done
                        .1
                        .wait_timeout_while(ended, CLEANUP_WAIT, |ended| !*ended);
                }),
            )
            .map_err(Failed::run)?;
    }
    Ok(cancel)
}

/// The provider of `harness`, as the first entry that runs it configures it, else the built-in one.
fn provider_for(config: &Config, harness: &str) -> Result<kind::Harness, Failed> {
    let entry = config.entries.iter().find(
        |entry| matches!(&entry.kind, EntryKind::Harness { harness: id, .. } if id == harness),
    );
    let (origin, settings) = match entry.map(|entry| (entry, &entry.kind)) {
        Some((
            entry,
            EntryKind::Harness {
                origin,
                executable,
                options,
                settings,
                ..
            },
        )) => (
            origin.clone(),
            serve::harness_settings(
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            ),
        ),
        _ if BUILT_IN_HARNESSES.contains(&harness) => (None, installed_program(harness)),
        _ => {
            return Err(Failed::usage(format!(
                "no entry runs a provider called {harness}: add an agent entry with harness = \"{harness}\" and its library or source"
            )));
        }
    };
    let registry = kind::registry(origin.as_ref()).map_err(Failed::run)?;
    kind::Harness::new(&Arc::new(registry), harness, &settings, false, Vec::new())
        .map_err(|error| Failed::run(format!("the {harness} provider is unusable: {error}")))
}

/// The settings of a built-in provider no entry configures: the program the Store installed, when it installed one.
fn installed_program(harness: &str) -> Settings {
    let managed = matches!(harness, "claude" | "codex")
        .then(|| crate::sources::managed_program(harness))
        .flatten();
    match managed {
        Some((path, _)) => Settings::new(serde_json::Map::from_iter([(
            "executable".to_owned(),
            serde_json::Value::String(path.to_string_lossy().into_owned()),
        )])),
        None => Settings::default(),
    }
}

fn methods_of(provider: &kind::Harness) -> Result<Vec<LoginMethodInfo>, Failed> {
    provider
        .catalog()
        .map(|catalog| catalog.logins.clone())
        .ok_or_else(|| Failed::run("the provider did not list its sign-in methods"))
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

fn accounts_of(cfg: Cfg<'_>) -> Result<Arc<Accounts>, Failed> {
    Accounts::open(cfg.config.account_store, cfg.home.dir())
        .map_err(|error| Failed::run(error.to_string()))
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
    pub(crate) harness: String,
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) state: String,
    pub(crate) revision: u64,
    pub(crate) expires_at: Option<String>,
}

impl Row {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name, "harness": self.harness, "kind": self.kind, "id": self.id,
            "state": self.state, "revision": self.revision, "expires_at": self.expires_at,
        })
    }
}

/// Every account canoa.cfg names, with its state.
pub(crate) fn rows(cfg: Cfg<'_>) -> Result<Vec<Row>, Failed> {
    Ok(rows_of(cfg, &accounts_of(cfg)?))
}

fn rows_of(cfg: Cfg<'_>, accounts: &Arc<Accounts>) -> Vec<Row> {
    cfg.config
        .accounts
        .iter()
        .map(|(name, account)| {
            let (state, revision, expires_at) = state_of(accounts, account);
            Row {
                name: name.clone(),
                harness: account.harness.clone(),
                kind: account.kind.clone(),
                id: account.id.clone(),
                state,
                revision,
                expires_at,
            }
        })
        .collect()
}

/// `canoa accounts list`.
pub(crate) fn list(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    let accounts = accounts_of(env.cfg())?;
    for row in rows_of(env.cfg(), &accounts) {
        let line = match format {
            Format::Json => row.json().to_string(),
            Format::Text => format!("{} {} {} {}", row.name, row.harness, row.kind, row.state),
        };
        env.front.print(&line).map_err(Failed::run)?;
    }
    if format == Format::Text
        && let Some(note) = accounts.note()
    {
        env.front.print(note).map_err(Failed::run)?;
    }
    Ok(())
}

/// A lowercase version 4 UUID, which Canoa never gives to two accounts.
fn new_id() -> Result<String, Failed> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| Failed::run(format!("no randomness: {error}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
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
    pub(crate) harness: String,
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) label: Option<String>,
    pub(crate) checked: bool,
}

impl Added {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({"account": {
            "name": self.name, "harness": self.harness, "kind": self.kind, "id": self.id,
            "label": self.label, "checked": self.checked}})
    }
}

/// What runs a sign-in: the stored accounts, the provider and the method's id, and what it made.
pub(crate) type Signer<'a> =
    dyn FnMut(&Accounts, &kind::Harness, &str) -> Result<coqui_acp_api::LoginOutcome, Failed> + 'a;

/// `canoa accounts add NAME`.
pub(crate) fn add(
    env: &Env<'_>,
    name: &str,
    harness: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    (format, until_stdin_closes): (Format, bool),
) -> Result<(), Failed> {
    let added = add_with(
        env.cfg(),
        name,
        harness,
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

/// Adds the account `name` of `harness` by the method `method`, signing in through `signer`.
pub(crate) fn add_with(
    cfg: Cfg<'_>,
    name: &str,
    harness: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    signer: &mut Signer<'_>,
) -> Result<Added, Failed> {
    check_name(name).map_err(Failed::usage)?;
    if cfg.config.accounts.contains_key(name) {
        return Err(Failed::usage(format!("an account called {name} exists")));
    }
    let provider = provider_for(cfg.config, harness)?;
    let methods = methods_of(&provider)?;
    let Some(method) = method.and_then(|id| methods.iter().find(|method| method.id == id)) else {
        return Err(Failed::usage(format!(
            "--method is one of the ways {harness} signs in: {}",
            describe_methods(&methods)
        )));
    };
    check_inputs(method, inputs)?;
    let accounts = accounts_of(cfg)?;
    let outcome = signer(&accounts, &provider, &method.id)?;
    let catalog = provider
        .catalog()
        .ok_or_else(|| Failed::run("the provider did not list its account kinds"))?;
    let kind = catalog.kind(&outcome.kind).ok_or_else(|| {
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
        harness: harness.to_owned(),
        kind: outcome.kind.clone(),
        id: id.clone(),
        settings: (!settings.is_empty()).then(|| Settings::new(settings)),
    };
    if let Err(failed) = append_account(cfg.config_path, name, &account) {
        let _ = accounts.forget_credentials(&id);
        let _ = accounts.forget_directory(&id);
        return Err(failed);
    }
    Ok(Added {
        name: name.to_owned(),
        harness: harness.to_owned(),
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
    let provider = provider_for(cfg.config, &account.harness)?;
    let methods = methods_of(&provider)?;
    let making: Vec<&LoginMethodInfo> = methods
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
    let outcome = signer(&accounts, &provider, &method.id)?;
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
        harness: account.harness.clone(),
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
    let accounts = accounts_of(cfg)?;
    let lease = accounts.exclusive(&account.id).map_err(refused)?;
    let before = read_cfg(cfg.config_path)?;
    remove_account(cfg.config_path, name)?;
    // The stored credential goes with the account; were it to stay, nothing would name it any more.
    if let Err(error) = accounts.forget_credentials(&account.id) {
        let _ = write_cfg(cfg.config_path, &before);
        return Err(Failed::run(error.to_string()));
    }
    if let Err(error) = accounts.forget_directory(&account.id) {
        tracing::warn!("the directory of the removed account {name} could not be removed: {error}");
    }
    drop(lease);
    Ok(())
}

/// The kinds and the sign-in methods the provider of `harness` lists.
pub(crate) fn catalog_of(
    config: &Config,
    harness: &str,
) -> Result<(Vec<AccountKindInfo>, Vec<LoginMethodInfo>), Failed> {
    let provider = provider_for(config, harness)?;
    let catalog = provider
        .catalog()
        .ok_or_else(|| Failed::run("the provider did not list its account kinds"))?;
    Ok((catalog.kinds.clone(), catalog.logins.clone()))
}

/// Runs a sign-in whose prompts a [`Relay`] carries, for a caller that shows them itself.
pub(crate) fn sign_in_relayed(
    accounts: &Accounts,
    provider: &kind::Harness,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    relay: Arc<Relay>,
) -> Result<coqui_acp_api::LoginOutcome, Failed> {
    login::sign_in(
        accounts,
        &**provider.provider(),
        method,
        inputs,
        relay,
        None,
    )
    .map_err(|error| Failed::run(error.to_string()))
}

/// A TOML basic string.
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON value as inline TOML; `None` for one TOML cannot hold.
fn toml_of(value: &serde_json::Value) -> Option<String> {
    Some(match value {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => quote(s),
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .filter_map(toml_of)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(map) => format!(
            "{{ {} }}",
            map.iter()
                .filter_map(|(key, value)| Some(format!("{} = {}", quote(key), toml_of(value)?)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn read_cfg(path: &Path) -> Result<String, Failed> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(Failed::run(format!("{}: {error}", path.display()))),
    }
}

/// Writes `text` over `path`, beside it first, and only if it is a canoa.cfg this host reads.
fn write_cfg(path: &Path, text: &str) -> Result<(), Failed> {
    Config::from_str_named(
        text,
        file_name(path),
        path.parent().unwrap_or_else(|| Path::new("")),
    )
    .map_err(|error| Failed::run(format!("the edited canoa.cfg would not be read: {error}")))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| Failed::run(format!("{}: {error}", dir.display())))?;
    }
    let beside = path.with_extension(format!("cfg.new{}", std::process::id()));
    fs::write(&beside, text)
        .and_then(|()| crate::replace::replace_file(&beside, path))
        .map_err(|error| {
            let _ = fs::remove_file(&beside);
            Failed::run(format!("{}: {error}", path.display()))
        })
}

/// The block `canoa accounts add` writes for an account.
fn block_of(name: &str, account: &AccountConfig) -> String {
    let mut block = format!(
        "[host.accounts.{name}]\nharness = {}\nkind = {}\nid = {}\n",
        quote(&account.harness),
        quote(&account.kind),
        quote(&account.id)
    );
    if let Some(settings) = &account.settings {
        let table = serde_json::Value::Object(settings.as_map().clone());
        if let Some(inline) = toml_of(&table) {
            block.push_str(&format!("settings = {inline}\n"));
        }
    }
    block
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("canoa.cfg")
}

/// The registered name of the format `text` is in, by its file's name or first line.
fn format_of(path: &Path, text: &str) -> Result<String, Failed> {
    let options = anycfg::Options::new().name(file_name(path));
    crate::format::formats()
        .detect(text.as_bytes(), options)
        .map(|detected| detected.format.to_owned())
        .map_err(|error| Failed::run(format!("{}: {error}", path.display())))
}

/// A JSON value as TOML's model, which is what canoa.cfg is read as; `None` for one it cannot hold.
fn toml_value(value: &serde_json::Value) -> Option<toml::Value> {
    Some(match value {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| n.as_f64().map(toml::Value::Float))?,
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(items) => {
            toml::Value::Array(items.iter().filter_map(toml_value).collect())
        }
        serde_json::Value::Object(map) => toml::Value::Table(
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), toml_value(value)?)))
                .collect(),
        ),
    })
}

/// A file in YAML or JSON, edited as a document and written back in its own format: its comments and layout are not kept.
fn edit_document(
    path: &Path,
    text: &str,
    format: &str,
    edit: impl FnOnce(&mut toml::Table) -> Result<(), Failed>,
) -> Result<String, Failed> {
    let formats = crate::format::formats();
    let name = file_name(path);
    let refused = |error: anycfg::Error| Failed::run(format!("{}: {error}", path.display()));
    let mut value = formats
        .from_slice(text.as_bytes(), anycfg::Options::new().name(name))
        .map_err(refused)?;
    let toml::Value::Table(table) = &mut value else {
        return Err(Failed::run(format!("{}: not a table", path.display())));
    };
    edit(table)?;
    let extension = Path::new(name).extension().and_then(|ext| ext.to_str());
    let bytes = if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("cfg")) {
        formats.to_auto_vec(&value, format, "cfg")
    } else {
        formats.to_vec(&value, format)
    }
    .map_err(refused)?;
    String::from_utf8(bytes).map_err(|error| Failed::run(error.to_string()))
}

fn append_account(path: &Path, name: &str, account: &AccountConfig) -> Result<(), Failed> {
    let _one = config_writes();
    let mut text = read_cfg(path)?;
    let format = format_of(path, &text)?;
    if format != "toml" {
        let edited = edit_document(path, &text, &format, |table| {
            let mut block = toml::Table::new();
            block.insert(
                "harness".into(),
                toml::Value::String(account.harness.clone()),
            );
            block.insert("kind".into(), toml::Value::String(account.kind.clone()));
            block.insert("id".into(), toml::Value::String(account.id.clone()));
            if let Some(settings) = &account.settings
                && let Some(value) =
                    toml_value(&serde_json::Value::Object(settings.as_map().clone()))
            {
                block.insert("settings".into(), value);
            }
            let host = table
                .entry("host")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let accounts = host
                .as_table_mut()
                .ok_or_else(|| Failed::run("[host] is not a table".to_owned()))?
                .entry("accounts")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            accounts
                .as_table_mut()
                .ok_or_else(|| Failed::run("[host.accounts] is not a table".to_owned()))?
                .insert(name.to_owned(), toml::Value::Table(block));
            Ok(())
        })?;
        return write_cfg(path, &edited);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&block_of(name, account));
    write_cfg(path, &text)
}

/// Held while canoa.cfg is read, changed and written back, so that an offer, an account added or removed and a setting saved never
/// undo one another: each reads the file the one before it wrote.
static CONFIG_WRITES: Mutex<()> = Mutex::new(());

/// Takes the right to change canoa.cfg, waiting for whoever has it.
pub(crate) fn config_writes() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Makes the agent entry `entry`, which runs `harness`, offer exactly `accounts` in the file at `path`, in the file's own format: an
/// entry the file does not have yet is written with its `harness` and `made` (the other keys it starts with), and none left takes
/// the key out.
pub(crate) fn set_offered(
    path: &Path,
    entry: &str,
    harness: &str,
    accounts: &[String],
    made: &[(&str, toml::Value)],
) -> Result<(), Failed> {
    let _one = config_writes();
    let mut text = read_cfg(path)?;
    let format = format_of(path, &text)?;
    if format != "toml" {
        let edited = edit_document(path, &text, &format, |table| {
            let agents = table
                .entry("host")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(|| Failed::run("[host] is not a table".to_owned()))?
                .entry("agents")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(|| Failed::run("[host.agents] is not a table".to_owned()))?;
            let fresh = !agents.contains_key(entry);
            if fresh && accounts.is_empty() {
                return Ok(());
            }
            let one = agents
                .entry(entry.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(|| Failed::run(format!("[host.agents.{entry}] is not a table")))?;
            if fresh {
                one.insert("harness".into(), toml::Value::String(harness.to_owned()));
                for (key, value) in made {
                    one.insert((*key).to_owned(), value.clone());
                }
            }
            if accounts.is_empty() {
                one.remove("accounts");
            } else {
                one.insert(
                    "accounts".into(),
                    toml::Value::Array(
                        accounts
                            .iter()
                            .map(|name| toml::Value::String(name.clone()))
                            .collect(),
                    ),
                );
            }
            Ok(())
        })?;
        return write_cfg(path, &edited);
    }
    let line = format!(
        "accounts = [{}]\n",
        accounts
            .iter()
            .map(|name| quote(name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let header = format!("[host.agents.{entry}]");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|each| each.trim() == header) else {
        if accounts.is_empty() {
            return Ok(());
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let extra: String = made
            .iter()
            .map(|(key, value)| format!("{key} = {value}\n"))
            .collect();
        text.push_str(&format!(
            "{header}\nharness = {}\n{extra}{line}",
            quote(harness)
        ));
        return write_cfg(path, &text);
    };
    let end = lines[start + 1..]
        .iter()
        .position(|each| each.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let at = lines[start + 1..end].iter().position(|each| {
        each.trim_start()
            .strip_prefix("accounts")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    let mut kept: Vec<String> = lines.iter().map(|each| (*each).to_owned()).collect();
    match at.map(|offset| start + 1 + offset) {
        Some(index) => {
            let value = kept[index].split_once('=').map_or("", |(_, value)| value);
            if value.matches('[').count() != value.matches(']').count() {
                return Err(Failed::usage(format!(
                    "canoa.cfg spells {header}'s accounts over several lines: change it by hand"
                )));
            }
            if accounts.is_empty() {
                kept.remove(index);
            } else {
                kept[index] = line;
            }
        }
        None if !accounts.is_empty() => {
            let mut after = start + 1;
            if !kept[start].ends_with('\n') {
                kept[start].push('\n');
            }
            if after > kept.len() {
                after = kept.len();
            }
            kept.insert(after, line);
        }
        None => {}
    }
    write_cfg(path, &kept.concat())
}

/// Takes the block `add` wrote for `name` out of the file. An account written some other way is for its writer to remove.
fn remove_account(path: &Path, name: &str) -> Result<(), Failed> {
    let _one = config_writes();
    let text = read_cfg(path)?;
    let format = format_of(path, &text)?;
    if format != "toml" {
        let edited = edit_document(path, &text, &format, |table| {
            let removed = table
                .get_mut("host")
                .and_then(toml::Value::as_table_mut)
                .and_then(|host| host.get_mut("accounts"))
                .and_then(toml::Value::as_table_mut)
                .and_then(|accounts| accounts.remove(name));
            removed.map(|_| ()).ok_or_else(|| {
                Failed::usage(format!(
                    "canoa.cfg does not hold [host.accounts.{name}] as `canoa accounts add` writes it: remove the account from it by hand"
                ))
            })
        })?;
        return write_cfg(path, &edited);
    }
    let header = format!("[host.accounts.{name}]");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return Err(Failed::usage(format!(
            "canoa.cfg does not hold {header} as `canoa accounts add` writes it: remove the account from it by hand"
        )));
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let mut kept: Vec<&str> = lines[..start].to_vec();
    // The blank line `add` put before the block goes with it.
    if kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    kept.extend(&lines[end..]);
    write_cfg(path, &kept.concat())
}

/// Whether the file at `path` has the agent entry `name`, in whatever format it is written.
pub(crate) fn has_entry(path: &Path, name: &str) -> Result<bool, Failed> {
    let _one = config_writes();
    let text = read_cfg(path)?;
    let value = crate::format::formats()
        .from_slice(
            text.as_bytes(),
            anycfg::Options::new().name(file_name(path)),
        )
        .map_err(|error| Failed::run(format!("{}: {error}", path.display())))?;
    Ok(value
        .as_table()
        .and_then(|table| table.get("host"))
        .and_then(|host| host.get("agents"))
        .and_then(|agents| agents.get(name))
        .is_some_and(toml::Value::is_table))
}

/// Takes the agent entry `name` out of the file at `path`, whole. Refused when the file spells it with tables of their own
/// (`[host.agents.NAME.x]`), which a caller that only means to drop a plain entry never meant.
pub(crate) fn remove_entry(path: &Path, name: &str) -> Result<(), Failed> {
    let _one = config_writes();
    let text = read_cfg(path)?;
    let format = format_of(path, &text)?;
    if format != "toml" {
        let edited = edit_document(path, &text, &format, |table| {
            table
                .get_mut("host")
                .and_then(toml::Value::as_table_mut)
                .and_then(|host| host.get_mut("agents"))
                .and_then(toml::Value::as_table_mut)
                .and_then(|agents| agents.remove(name))
                .map(|_| ())
                .ok_or_else(|| Failed::usage(format!("canoa.cfg has no [host.agents.{name}]")))
        })?;
        return write_cfg(path, &edited);
    }
    let header = format!("[host.agents.{name}]");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return Err(Failed::usage(format!(
            "canoa.cfg does not spell {header} as a table of its own: change it by hand"
        )));
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let below = format!("[host.agents.{name}.");
    if lines[end..]
        .iter()
        .any(|line| line.trim_start().starts_with(&below))
    {
        return Err(Failed::usage(format!(
            "{header} has tables of its own: change canoa.cfg by hand"
        )));
    }
    let mut kept: Vec<&str> = lines[..start].to_vec();
    if kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    kept.extend(&lines[end..]);
    write_cfg(path, &kept.concat())
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[cfg(test)]
mod format_tests {
    use super::*;

    /// A rewrite of canoa.cfg leaves its mode as the person set it, a file only this user reads included.
    #[cfg(unix)]
    #[test]
    fn a_rewrite_of_canoa_cfg_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("canoa-accounts-mode-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        for mode in [0o600, 0o644] {
            let path = dir.join(format!("{mode:o}.cfg"));
            fs::write(&path, "[desktop]\nstart_hidden = true\n").expect("write");
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("mode");
            write_cfg(&path, "[desktop]\nstart_hidden = false\n").expect("rewritten");
            let now = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(now, mode, "{mode:o} became {now:o}");
            assert!(fs::read_to_string(&path).expect("read").contains("false"));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(all(feature = "yaml", feature = "json"))]
    fn account() -> AccountConfig {
        AccountConfig {
            harness: "claude".to_owned(),
            kind: "api".to_owned(),
            id: "id-1".to_owned(),
            settings: None,
        }
    }

    /// `accounts add` and `remove` keep a YAML or JSON file in its format, with its marker or its extension.
    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn an_account_is_added_and_removed_in_the_files_own_format() {
        let dir =
            std::env::temp_dir().join(format!("canoa-accounts-format-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        for (name, text) in [
            ("a.cfg", "#!yaml\ndesktop:\n  start_hidden: true\n"),
            ("b.cfg", "#!json\n{\"desktop\": {\"start_hidden\": true}}\n"),
            ("c.yaml", "desktop:\n  start_hidden: true\n"),
            ("d.json", "{\"desktop\": {\"start_hidden\": true}}\n"),
            ("e.cfg", "[desktop]\nstart_hidden = true\n"),
        ] {
            let path = dir.join(name);
            fs::write(&path, text).expect("write");
            append_account(&path, "work", &account()).expect(name);
            let written = fs::read_to_string(&path).expect("read");
            let marker = text.lines().next().filter(|line| line.starts_with("#!"));
            if let Some(marker) = marker {
                assert!(written.starts_with(marker), "{name}: {written}");
            } else {
                assert!(!written.starts_with("#!"), "{name}: {written}");
            }
            let config = Config::from_str_named(&written, name, Path::new(".")).expect(name);
            assert_eq!(config.accounts["work"].id, "id-1", "{name}");
            assert!(
                written.contains("start_hidden"),
                "{name}: the other tables stay"
            );
            remove_account(&path, "work").expect(name);
            let after = fs::read_to_string(&path).expect("read");
            assert!(!after.contains("work"), "{name}: {after}");
            assert!(after.contains("start_hidden"), "{name}: {after}");
        }
        fs::remove_dir_all(dir).expect("cleanup");
    }

    /// An entry's offered accounts are set in a TOML, YAML or JSON file in that file's own format, whether the entry is there or not.
    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn the_accounts_an_entry_offers_are_set_in_the_files_own_format() {
        let dir = std::env::temp_dir().join(format!("canoa-offer-format-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        let offered = |text: &str, name: &str| {
            let config = Config::from_str_named(text, name, Path::new(".")).expect(name);
            let entry = config.entries.iter().find(|each| each.name == "Claude");
            match entry.map(|each| &each.kind) {
                Some(EntryKind::Harness {
                    harness, accounts, ..
                }) => Some((harness.clone(), accounts.clone())),
                _ => None,
            }
        };
        for (name, text) in [
            ("a.cfg", "#!yaml\ndesktop:\n  start_hidden: true\n"),
            ("b.cfg", "#!json\n{\"desktop\": {\"start_hidden\": true}}\n"),
            ("c.yaml", "desktop:\n  start_hidden: true\n"),
            ("e.cfg", "[desktop]\nstart_hidden = true\n"),
            (
                "f.cfg",
                "[host.agents.Claude]\n# the person's own\nharness = \"claude\"\ntitle = \"Mine\"\n\n[desktop]\nstart_hidden = true\n",
            ),
            (
                "g.cfg",
                "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"work\"]\n\n[desktop]\nstart_hidden = true\n",
            ),
        ] {
            let path = dir.join(name);
            fs::write(&path, text).expect("write");
            for who in ["work", "other"] {
                let mut made = account();
                made.id = format!("id-{who}");
                append_account(&path, who, &made).expect(name);
            }
            let both = vec!["other".to_owned(), "work".to_owned()];
            set_offered(&path, "Claude", "claude", &both, &[]).expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name),
                Some(("claude".to_owned(), both.clone())),
                "{name}: {written}"
            );
            assert!(
                written.contains("start_hidden"),
                "{name}: other tables stay"
            );
            if name == "f.cfg" {
                assert!(written.contains("# the person's own") && written.contains("Mine"));
            }
            set_offered(&path, "Claude", "claude", &["work".to_owned()], &[]).expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name).map(|each| each.1),
                Some(vec!["work".to_owned()]),
                "{name}: {written}"
            );
            set_offered(&path, "Claude", "claude", &[], &[]).expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name).map(|each| each.1),
                Some(Vec::new()),
                "{name}: {written}"
            );
        }
        fs::remove_dir_all(dir).expect("cleanup");
    }

    /// Accounts added and offers made at the same moment all reach canoa.cfg: each reads the file the one before it wrote.
    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn changes_made_at_the_same_moment_do_not_undo_one_another() {
        let dir = std::env::temp_dir().join(format!("canoa-writes-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        let path = dir.join("canoa.cfg");
        fs::write(
            &path,
            "[desktop]
start_hidden = true
",
        )
        .expect("write");
        std::thread::scope(|scope| {
            for index in 0..12 {
                let path = &path;
                scope.spawn(move || {
                    let mut made = account();
                    made.id = format!("id-{index}");
                    append_account(path, &format!("a{index}"), &made).expect("added");
                    set_offered(
                        path,
                        &format!("E{index}"),
                        "claude",
                        &[format!("a{index}")],
                        &[],
                    )
                    .expect("offered");
                });
            }
        });
        let written = fs::read_to_string(&path).expect("read");
        let config = Config::from_str_named(&written, "canoa.cfg", Path::new(".")).expect("reads");
        assert_eq!(config.accounts.len(), 12, "{written}");
        assert_eq!(config.entries.len(), 12, "{written}");
        assert!(written.contains("start_hidden"));
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
