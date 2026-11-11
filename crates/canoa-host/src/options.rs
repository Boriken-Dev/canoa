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
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use coqui_acp_api::{
    AccountKeeper as _, AcpHarness, ModelInfo, ModelListing, ModelsRequest, ProcessSpawner as _,
};
use coqui_acp_harness::LocalSpawner;
use serde_json::{Map, Value, json};

use crate::config::{Config, EntryKind, Settings};
use crate::harness::Harness;
use crate::accounts::NoAccounts;
use crate::kind;

/// How long the program's `--version` is waited for.
const VERSION_WAIT: Duration = Duration::from_secs(10);
/// The most of a version line kept, in bytes.
const VERSION_BYTES: usize = 128;

/// What one source offers before it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Choices {
    /// Whether the provider listed its models; when not, `models` is empty and `unknown_because` says why.
    pub known: bool,
    /// Why it could not list them.
    pub unknown_because: Option<String>,
    /// The models in the provider's order, each with its own efforts.
    pub models: Vec<ModelInfo>,
    /// The program the source runs, as the provider names it.
    pub program: Option<String>,
    /// The first line of the program's `--version`, when it answered: what a cache of these choices is keyed by.
    pub program_version: Option<String>,
}

impl Choices {
    fn unknown(why: impl Into<String>) -> Self {
        Self {
            known: false,
            unknown_because: Some(why.into()),
            models: Vec::new(),
            program: None,
            program_version: None,
        }
    }

    /// The choices as one JSON object: `known`, `unknown_because` when not known, `program`, `program_version` and
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
        Err(error) => return Choices::unknown(error.to_string()),
    };
    match kind::Harness::new(
        &registry,
        harness.as_str(),
        &Settings::new(map),
        false,
        Vec::new(),
    ) {
        Ok(configured) => ask(configured.connection(None).0.as_ref()),
        Err(error) => Choices::unknown(error.to_string()),
    }
}

/// What the entry `name` of `config` offers: a harness entry's provider lists its models; an entry that runs a program
/// speaking ACP, a provider or a tool says it lists none before it starts. `None` when `config` has no such entry.
#[must_use]
pub fn entry(config: &Config, name: &str) -> Option<Choices> {
    let entry = config.entry(name)?;
    Some(match &entry.kind {
        EntryKind::Harness {
            harness,
            origin,
            executable,
            options,
            settings,
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
                Err(error) => return Some(Choices::unknown(error.to_string())),
            };
            match kind::Harness::new(&registry, harness, &configured, false, Vec::new()) {
                Ok(built) => ask(built.connection(None).0.as_ref()),
                Err(error) => Choices::unknown(error.to_string()),
            }
        }
        EntryKind::Agent { .. } | EntryKind::Provider { .. } | EntryKind::Tool { .. } => {
            Choices::unknown("this source lists its models in its session, not before it")
        }
    })
}

/// Asks `harness` for its models with the machine's own login, and its program for its version.
fn ask(harness: &dyn AcpHarness) -> Choices {
    let listing = harness
        .models(
            ModelsRequest::new(),
            LocalSpawner::new().into_object(),
            NoAccounts.into_object(),
        )
        .unwrap_or_else(|error| ModelListing::unknown(error.message()));
    let program = harness.program().ok().flatten();
    let program_version = program.as_deref().and_then(version_of);
    Choices {
        known: listing.known,
        unknown_because: listing.unknown_because,
        models: listing.models,
        program,
        program_version,
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
                .stdin(Stdio::null())
                .stderr(Stdio::null())
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
