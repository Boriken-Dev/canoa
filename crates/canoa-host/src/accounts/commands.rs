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

/// What a sign-in asks of its front, passed to the thread that owns it.
enum Event {
    Say(String),
    Open(OpenUrl),
    Ask(LoginForm, mpsc::Sender<Result<LoginAnswer, String>>),
}

/// The sign-in's side of the conversation: it queues what the provider says and waits for the answer to a question.
struct Relay {
    events: Mutex<mpsc::Sender<Event>>,
    cancelled: Arc<AtomicBool>,
}

impl Relay {
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
const NO_TYPING: &str = "this way of running canoa cannot take a typed secret: sealed prompts are not available to it yet, so run `canoa accounts add` in a terminal";

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
    let relay = Arc::new(Relay {
        events: Mutex::new(events),
        cancelled: Arc::clone(&cancel.flag),
    });
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
        _ if BUILT_IN_HARNESSES.contains(&harness) => (None, Settings::default()),
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

fn accounts_of(env: &Env<'_>) -> Result<Arc<Accounts>, Failed> {
    Accounts::open(env.config.account_store, env.home.dir())
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

/// `canoa accounts list`.
pub(crate) fn list(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    let accounts = accounts_of(env)?;
    for (name, account) in &env.config.accounts {
        let (state, revision, expires_at) = state_of(&accounts, account);
        let line = match format {
            Format::Json => serde_json::json!({
                "name": name, "harness": account.harness, "kind": account.kind, "id": account.id,
                "state": state, "revision": revision, "expires_at": expires_at,
            })
            .to_string(),
            Format::Text => format!("{name} {} {} {state}", account.harness, account.kind),
        };
        env.front.print(&line).map_err(Failed::run)?;
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

/// `canoa accounts add NAME`.
pub(crate) fn add(
    env: &Env<'_>,
    name: &str,
    harness: &str,
    method: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    (format, until_stdin_closes): (Format, bool),
) -> Result<(), Failed> {
    check_name(name).map_err(Failed::usage)?;
    if env.config.accounts.contains_key(name) {
        return Err(Failed::usage(format!("an account called {name} exists")));
    }
    let provider = provider_for(&env.config, harness)?;
    let methods = methods_of(&provider)?;
    let Some(method) = method.and_then(|id| methods.iter().find(|method| method.id == id)) else {
        return Err(Failed::usage(format!(
            "--method is one of the ways {harness} signs in: {}",
            describe_methods(&methods)
        )));
    };
    check_inputs(method, inputs)?;
    let accounts = accounts_of(env)?;
    let cancel = cancel_on_input_closing(env.front, until_stdin_closes)?;
    let outcome = sign_in(
        env, &accounts, &provider, &method.id, inputs, format, &cancel,
    )?;
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
    if let Err(failed) = append_account(&env.config_path, name, &account) {
        let _ = accounts.forget_credentials(&id);
        let _ = accounts.forget_directory(&id);
        return Err(failed);
    }
    let line = match format {
        Format::Json => serde_json::json!({"account": {
            "name": name, "harness": harness, "kind": outcome.kind, "id": id,
            "label": outcome.label, "checked": outcome.checked}})
        .to_string(),
        Format::Text => format!("added {name} ({})", outcome.kind),
    };
    env.front.print(&line).map_err(Failed::run)
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
    let account = env
        .config
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no account called {name}")))?;
    let provider = provider_for(&env.config, &account.harness)?;
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
    let accounts = accounts_of(env)?;
    let _lease = accounts.exclusive(&account.id).map_err(refused)?;
    let cancel = cancel_on_input_closing(env.front, until_stdin_closes)?;
    let outcome = sign_in(
        env, &accounts, &provider, &method.id, inputs, format, &cancel,
    )?;
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
    let line = match format {
        Format::Json => {
            serde_json::json!({"account": {"name": name, "id": account.id}}).to_string()
        }
        Format::Text => format!("signed in {name}"),
    };
    env.front.print(&line).map_err(Failed::run)
}

fn refused(refusal: Refusal) -> Failed {
    Failed::run(refusal.to_string())
}

/// `canoa accounts remove NAME`: forgets the credential and the account's state, and the account from canoa.cfg. Nothing is
/// signed out at the provider.
pub(crate) fn remove(env: &Env<'_>, name: &str) -> Result<(), Failed> {
    let account = env
        .config
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no account called {name}")))?;
    if let Some(entry) = env.config.entries.iter().find(|entry| {
        matches!(&entry.kind, EntryKind::Harness { accounts, .. } if accounts.iter().any(|offered| offered == name))
    }) {
        return Err(Failed::usage(format!(
            "entry {} offers {name}: take it out of that entry's accounts first",
            entry.name
        )));
    }
    let accounts = accounts_of(env)?;
    let lease = accounts.exclusive(&account.id).map_err(refused)?;
    let before = read_cfg(&env.config_path)?;
    remove_account(&env.config_path, name)?;
    // The stored credential goes with the account; were it to stay, nothing would name it any more.
    if let Err(error) = accounts.forget_credentials(&account.id) {
        let _ = write_cfg(&env.config_path, &before);
        return Err(Failed::run(error.to_string()));
    }
    if let Err(error) = accounts.forget_directory(&account.id) {
        tracing::warn!("the directory of the removed account {name} could not be removed: {error}");
    }
    drop(lease);
    env.front
        .print(&format!("removed {name}"))
        .map_err(Failed::run)
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
    Config::from_str_in(text, path.parent().unwrap_or_else(|| Path::new("")))
        .map_err(|error| Failed::run(format!("the edited canoa.cfg would not be read: {error}")))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| Failed::run(format!("{}: {error}", dir.display())))?;
    }
    let beside = path.with_extension(format!("cfg.new{}", std::process::id()));
    fs::write(&beside, text)
        .and_then(|()| fs::rename(&beside, path))
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

fn append_account(path: &Path, name: &str, account: &AccountConfig) -> Result<(), Failed> {
    let mut text = read_cfg(path)?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&block_of(name, account));
    write_cfg(path, &text)
}

/// Takes the block `add` wrote for `name` out of the file. An account written some other way is for its writer to remove.
fn remove_account(path: &Path, name: &str) -> Result<(), Failed> {
    let text = read_cfg(path)?;
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
