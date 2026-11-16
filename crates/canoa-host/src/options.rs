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
    AccountGrant, AccountKeeper, AcpHarness, CacheLifetime, ModelInfo, ModelListing, ModelsRequest,
    ProcessSpawner, UnknownKind,
};
use serde_json::{Map, Value, json};

use crate::spawn::{Command, Stdio};

mod probe;

use crate::config::{Config, EntryKind, Settings, cache_lifetime_name};
use crate::harness::Harness;
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
    /// The account of the entry to ask as, with the host's accounts; `None` asks on the login the machine has.
    pub(crate) account: Option<(String, Arc<crate::accounts::Accounts>)>,
}

impl Default for Asking {
    fn default() -> Self {
        Self {
            wait: PROBE_WAIT,
            account: None,
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
}

impl Choices {
    fn unknown(why: impl Into<String>) -> Self {
        Self {
            known: false,
            unknown_because: Some(why.into()),
            unknown_kind: None,
            signed_out: false,
            models: Vec::new(),
            program: None,
            program_version: None,
            cache_lifetime: None,
            cache_choices: Vec::new(),
            configured_cache_lifetime: None,
        }
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
        out.insert(
            "models".into(),
            Value::Array(self.models.iter().map(model_json).collect()),
        );
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
pub fn preset(harness: Harness, executable: Option<&OsStr>) -> Choices {
    let variable = match harness {
        Harness::Claude => "COQUI_ACP_CLAUDE_EXECUTABLE",
        Harness::Codex => "COQUI_ACP_CODEX_EXECUTABLE",
    };
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
        harness.as_str(),
        &Settings::new(map),
        false,
        Vec::new(),
    ) {
        Ok(configured) => {
            let connection = configured.connection(None, None, CacheLifetime::Default);
            ask(
                connection.harness.as_ref(),
                connection.spawner,
                connection.keeper,
                None,
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
            let mut built =
                match kind::Harness::new(&registry, harness, &configured, false, Vec::new()) {
                    Ok(built) => built,
                    Err(error) => return Some(Choices::failed(error.to_string())),
                };
            let mut held = None;
            if let Some((account, store)) = &asking.account {
                if !accounts.contains(account) {
                    return Some(Choices::failed(format!(
                        "{name} does not offer the account {account}"
                    )));
                }
                let listed = match crate::serve::offered_accounts(config, harness, accounts, &built)
                {
                    Ok(listed) => listed,
                    Err(why) => return Some(Choices::failed(why)),
                };
                built = built.offering(store, listed, Vec::new());
                match built.hold_account(account, "choices", false) {
                    Ok(taken) => held = Some(taken),
                    Err(refusal) => return Some(Choices::failed(refusal.client_reason())),
                }
            }
            let connection = built.connection(None, held.as_ref(), CacheLifetime::Default);
            let mut choices = ask(
                connection.harness.as_ref(),
                connection.spawner,
                connection.keeper,
                held.as_ref().map(crate::accounts::Held::grant),
            );
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

/// Asks `harness` for its models, as `account` when one is named, and its program for its version.
fn ask(
    harness: &dyn AcpHarness,
    spawner: Object<dyn ProcessSpawner>,
    keeper: Object<dyn AccountKeeper>,
    account: Option<AccountGrant>,
) -> Choices {
    let mut request = ModelsRequest::new();
    if let Some(account) = account {
        request = request.account(account);
    }
    let listing = harness
        .models(request, spawner, keeper)
        .unwrap_or_else(|error| ModelListing::unavailable(UnknownKind::Failed, error.message()));
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
