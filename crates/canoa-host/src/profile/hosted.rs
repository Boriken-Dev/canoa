//! A `[host.profiles]` profile applied to the agent an entry starts for one connection: the environment when the process
//! starts, then context, skills and MCP servers with each session, each part where the agent's harness takes it, and a
//! report of what applied sent to the client. The client names a profile only among those its entry offers, and never
//! anything inside one.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_web::{Channel, Failure};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::overlay::{base_of, build, granted, resolve_path};
use super::{
    Context, Effective, EnvValue, Layer, LayerId, LayerKind, McpServer, OverlayInput, Secret,
    Sourced, merge, overlay_layer, resolve,
};
use crate::aos::{self, EntryKind, TreeEntry};
use crate::config::HostProfile;
use crate::scope::{Scope, Verdict};

/// The `_meta` key a client names a profile under in `initialize`, and the host's report under in its answers.
pub const PROFILE_META: &str = "dev.boriken.canoa/profile";

/// The longest initial context a session is given, in UTF-8 bytes, every layer's together.
const MAX_CONTEXT: usize = 1024 * 1024;
/// What one skill's directory may hold.
const MAX_SKILL_FILES: usize = 1024;
const MAX_SKILL_BYTES: u64 = 16 * 1024 * 1024;
/// What one installed overlay may hold.
const MAX_OVERLAY_FILES: usize = 4096;
const MAX_OVERLAY_BYTES: u64 = 64 * 1024 * 1024;
/// The plugin Claude Code is given a profile's skills in.
const CLAUDE_PLUGIN: &str = "canoa-profile";
/// Where Codex reads skills inside each directory it is given as `additionalDirectories`.
const CODEX_SKILLS: &str = ".agents/skills";
/// The session requests a profile's parts go with.
const SESSION_METHODS: [&str; 3] = ["session/new", "session/load", "session/resume"];
/// `initialize` and session requests a connection may have waiting for the agent's answer.
const MAX_PENDING: usize = 64;
const INVALID_REQUEST: i64 = -32600;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const REPLY_LIMIT: Duration = Duration::from_secs(30);
const REPLY_POLL: Duration = Duration::from_millis(10);

/// What became of one part of a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Applied,
    Unsupported,
    Off,
    Refused,
}

impl State {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Unsupported => "not supported",
            Self::Off => "off",
            Self::Refused => "refused",
        }
    }
}

/// One part of a profile (`context`, `env.NAME`, `skills.NAME`, `mcp.NAME`, `model`, ...), its layer, what became of
/// it and how, as the app's report has it. Never a value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    key: String,
    layer: String,
    state: State,
    how: String,
    /// An MCP server's definition digest, which `allow_mcp` names.
    digest: Option<String>,
}

impl Item {
    fn new(
        key: impl Into<String>,
        layer: impl ToString,
        state: State,
        how: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            layer: layer.to_string(),
            state,
            how: how.into(),
            digest: None,
        }
    }

    fn json(&self) -> Value {
        let mut out = json!({"key": self.key, "layer": self.layer, "state": self.state.as_str(), "how": self.how});
        if let Some(digest) = &self.digest {
            out["digest"] = json!(digest);
        }
        out
    }
}

/// What an entry offers its clients: its profiles, the first by default, and the program that launches its agent.
#[derive(Debug)]
pub(crate) struct Offer {
    pub(crate) profiles: Vec<(String, HostProfile)>,
    pub(crate) launcher: OsString,
}

/// A client's first message refused: the answer to send, when it was a request, and what to log.
#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) answer: Option<Vec<u8>>,
    pub(crate) log: &'static str,
}

fn error(id: &Value, code: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
    .unwrap_or_default()
}

/// The profile `first` names in `initialize`'s `_meta`, by its index in `offer`; the first when it names none. Anything
/// but the name of one the entry offers, or a choice in a message other than `initialize`, is refused.
pub(crate) fn choose(offer: &Offer, first: &[u8]) -> Result<usize, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let id = message.get("id").cloned();
    let choice = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(PROFILE_META));
    let Some(choice) = choice else {
        return Ok(0);
    };
    let refuse = |message: &str, log: &'static str| Refusal {
        answer: id.as_ref().map(|id| error(id, INVALID_PARAMS, message)),
        log,
    };
    if message.get("method").and_then(Value::as_str) != Some("initialize") {
        return Err(refuse(
            "a profile is chosen in initialize",
            "a client named a profile outside initialize, which was refused",
        ));
    }
    let Some(name) = choice.as_str() else {
        return Err(refuse(
            "a profile is chosen by its name, among those this entry offers",
            "a client sent something other than a profile's name as its choice, which was refused",
        ));
    };
    offer
        .profiles
        .iter()
        .position(|(offered, _)| offered == name)
        .ok_or_else(|| {
            refuse(
                "this entry does not offer that profile",
                "a client asked for a profile this entry does not offer, which was refused",
            )
        })
}

/// An installed overlay as read when a connection starts: its tree, never read again.
#[derive(Debug)]
struct ReadOverlay {
    identity: String,
    root: String,
    tree: Vec<TreeEntry>,
}

impl ReadOverlay {
    /// A member file's bytes, by `/`-separated path: what the digest covered.
    fn file(&self, path: &str) -> Option<&[u8]> {
        self.tree
            .iter()
            .find(|entry| {
                entry.kind == EntryKind::File
                    && !entry.is_excluded()
                    && entry.components.join(&b'/') == path.as_bytes()
            })
            .map(|entry| entry.data.as_slice())
    }

    /// The member files under `prefix/`, by path below it.
    fn under(&self, prefix: &str) -> BTreeMap<String, Vec<u8>> {
        let prefix = format!("{prefix}/");
        self.tree
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && !entry.is_excluded())
            .filter_map(|entry| {
                let path = String::from_utf8(entry.components.join(&b'/')).ok()?;
                Some((path.strip_prefix(&prefix)?.to_owned(), entry.data.clone()))
            })
            .collect()
    }
}

/// The operating system as AOS's `os` keys name it.
const fn os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// One profile, prepared for one connection when it starts: the overlays read, the environment resolved.
#[derive(Debug)]
pub(crate) struct Connection {
    name: String,
    entry: String,
    host: HostProfile,
    overlays: Vec<ReadOverlay>,
    /// What was left out before any harness is known: overlays, and entries that read a variable not shared.
    notes: Vec<Item>,
    env: Vec<(String, Secret)>,
    env_items: Vec<Item>,
}

impl Connection {
    /// Reads the overlays `offer`'s profile `index` names and resolves its environment through `lookup`.
    ///
    /// # Errors
    ///
    /// The environment cannot be built, as [`super::ProfileError`] words it: no agent starts with it.
    pub(crate) fn prepare(
        offer: &Offer,
        index: usize,
        entry: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let (name, host) = &offer.profiles[index];
        let mut notes = Vec::new();
        let overlays = read_overlays(host, &mut notes);
        let layers = layers_of(host, &overlays, None, &mut notes);
        let effective = merge(&layers);
        let env = effective
            .environment(lookup)
            .map_err(|error| error.to_string())?;
        let mut env_items = Vec::new();
        for (key, value) in &effective.env {
            if env.iter().any(|(name, _)| name == key) {
                env_items.push(Item::new(
                    format!("env.{key}"),
                    &value.layer,
                    State::Applied,
                    "the agent's process environment",
                ));
            } else {
                env_items.push(Item::new(
                    format!("env.{key}"),
                    &value.layer,
                    State::Off,
                    "an optional reference that is not set",
                ));
            }
        }
        for refused in &effective.refused {
            notes.push(Item::new(
                refused.key.clone(),
                &refused.layer,
                State::Refused,
                format!(
                    "it reads ${}, which this host's profile does not share",
                    refused.reads
                ),
            ));
        }
        for report in &effective.overlays {
            for what in &report.not_applied {
                notes.push(Item::new(
                    what.clone(),
                    &report.layer,
                    State::Unsupported,
                    "Canoa does not apply it",
                ));
            }
        }
        Ok(Self {
            name: name.clone(),
            entry: entry.to_owned(),
            host: host.clone(),
            overlays,
            notes,
            env,
            env_items,
        })
    }

    /// The line `canoa-host launch` reads first.
    pub(crate) fn launch_line(&self) -> Vec<u8> {
        super::launch::launch_line(&self.env)
    }
}

/// Each overlay `host` names, read from `overlays/NAME` beside Canoa.cfg and checked against the digest of the record
/// the app wrote when it installed it, `overlays/NAME.json`. One that cannot be read or does not match is left out.
fn read_overlays(host: &HostProfile, notes: &mut Vec<Item>) -> Vec<ReadOverlay> {
    let mut out = Vec::new();
    let store = host.directory.join("overlays");
    for identity in &host.overlays {
        let layer = LayerId::new(LayerKind::Overlay, identity.clone());
        let mut leave_out = |why: &str| {
            notes.push(Item::new(
                "overlay",
                &layer,
                State::Refused,
                format!("left out: {why}"),
            ))
        };
        let record = std::fs::read(store.join(format!("{identity}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(expected) = record
            .as_ref()
            .and_then(|record| record.get("digest"))
            .and_then(Value::as_str)
        else {
            leave_out(
                "it is not installed, or its record holds no digest: install or sync it in Canoa",
            );
            continue;
        };
        let root = store.join(identity);
        let tree = match aos::read_tree(&root, MAX_OVERLAY_FILES, MAX_OVERLAY_BYTES) {
            Ok(tree) => tree,
            Err(error) => {
                leave_out(&format!(
                    "its files cannot be read ({}: {})",
                    error.code, error.subject
                ));
                continue;
            }
        };
        if tree.iter().any(TreeEntry::is_excluded) {
            leave_out("its installed copy holds files that are no part of an overlay: sync it");
            continue;
        }
        match aos::digest_entries(&tree) {
            Ok(digest) if aos::same_digest(&digest.digest, expected) => {}
            _ => {
                leave_out("its files are not those its record's digest names: sync it");
                continue;
            }
        }
        out.push(ReadOverlay {
            identity: identity.clone(),
            root: root.to_string_lossy().into_owned(),
            tree,
        });
    }
    out
}

/// The layers of `host` for `harness`: its overlays in composition order, then its own.
fn layers_of(
    host: &HostProfile,
    overlays: &[ReadOverlay],
    harness: Option<&str>,
    notes: &mut Vec<Item>,
) -> Vec<Layer> {
    let mut found = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for overlay in overlays {
        let mut input = OverlayInput::new(&overlay.identity, &overlay.root, os(), &overlay.tree);
        input.harness = harness;
        input.pinned = true;
        match overlay_layer(&input) {
            Ok(layer) => {
                let key = aos::normalize_name(&overlay.identity);
                if let Some(aos) = &layer.aos {
                    manifests.insert(key.clone(), aos.manifest.clone());
                }
                found.insert(key, layer);
            }
            Err(refusal) => {
                // Read once per harness; said once.
                if harness.is_none() {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, overlay.identity.clone()),
                        State::Refused,
                        format!("left out: {refusal}"),
                    ));
                }
            }
        }
    }
    let active: Vec<String> = manifests.keys().cloned().collect();
    let mut layers: Vec<Layer> = aos::composition_order(&manifests, &active)
        .into_iter()
        .filter_map(|key| found.remove(&key))
        .collect();
    layers.push(Layer::new(
        LayerId::new(LayerKind::User, ""),
        Some(host.directory.clone()),
        host.profile.clone(),
    ));
    layers
}

/// What an agent's `initialize` answer says it takes with a session.
#[derive(Clone, Debug, Default)]
struct Takes {
    version: u64,
    capabilities: Value,
}

impl Takes {
    fn says(&self, path: &[&str]) -> bool {
        let mut at = &self.capabilities;
        for step in path {
            match at.get(step) {
                Some(next) => at = next,
                None => return false,
            }
        }
        at == &Value::Bool(true) || at.is_object()
    }

    /// Stdio is in v1's baseline and v2 says it; v2 has no SSE.
    fn mcp(&self, transport: &str) -> bool {
        match (self.version, transport) {
            (2, "stdio") => self.says(&["session", "mcp", "stdio"]),
            (2, "http") => self.says(&["session", "mcp", "http"]),
            (2, _) => false,
            (_, "stdio") => true,
            (_, other) => self.says(&["mcpCapabilities", other]),
        }
    }

    fn system_prompt(&self) -> bool {
        if self.version == 2 {
            self.says(&["session", "systemPrompt"])
        } else {
            self.says(&["sessionCapabilities", "systemPrompt"])
        }
    }

    fn additional_directories(&self) -> bool {
        if self.version == 2 {
            self.says(&["session", "additionalDirectories"])
        } else {
            self.says(&["sessionCapabilities", "additionalDirectories"])
        }
    }
}

/// The AOS harness id of what an agent reports as its name, else of an entry named as Canoa's presets are.
fn harness_of(reported: Option<&str>, entry: &str) -> Option<&'static str> {
    match reported {
        Some("@agentclientprotocol/claude-agent-acp") => Some("claude-code"),
        Some("@agentclientprotocol/codex-acp") => Some("codex"),
        Some("gemini-cli") => Some("gemini-cli"),
        Some("pi-acp") => Some("pi"),
        Some("OpenCode") => Some("opencode"),
        _ => match entry {
            "Claude" => Some("claude-code"),
            "Codex" => Some("codex"),
            "Gemini" => Some("gemini-cli"),
            "Pi" => Some("pi"),
            _ => None,
        },
    }
}

fn context_not_taken(harness: Option<&str>) -> &'static str {
    match harness {
        Some("pi") => {
            "pi-acp takes no system prompt; Pi reads SYSTEM.md and APPEND_SYSTEM.md only from its own directory"
        }
        Some("gemini-cli") => {
            "Gemini CLI takes no system prompt over ACP; GEMINI_SYSTEM_MD would replace its whole prompt"
        }
        Some("opencode") => {
            "OpenCode takes instructions only from its configuration, when it starts"
        }
        _ => "the agent does not say it takes a system prompt",
    }
}

fn skills_not_taken(harness: Option<&str>) -> &'static str {
    match harness {
        Some("pi") => {
            "pi-acp passes Pi no --skill; Pi reads skills only from its own and the project's directories"
        }
        Some("gemini-cli") => {
            "Gemini CLI reads skills only from its own, your home's and the project's skill directories"
        }
        Some("opencode") => {
            "OpenCode reads skills only from its configuration and the project's directories"
        }
        Some("codex") => {
            "the adapter does not say it takes more directories, where Codex would read them"
        }
        _ => "ACP has no field for skills, and Canoa knows no other way this agent takes them",
    }
}

/// A private directory a connection's skills are written to, removed with it.
#[derive(Debug)]
struct Written(PathBuf);

impl Written {
    fn create(files: &BTreeMap<String, Vec<u8>>) -> std::io::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = std::env::temp_dir().join(format!("canoa-host-profile-{name}"));
        #[cfg_attr(not(unix), expect(unused_mut, reason = "only Unix sets a mode"))]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&root)?;
        let written = Self(root);
        for (path, bytes) in files {
            let mut target = written.0.clone();
            for part in path.split('/') {
                if part.is_empty() || part == "." || part == ".." {
                    return Err(std::io::Error::other("a path that is not plain"));
                }
                target.push(part);
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::io::Write::write_all(
                &mut std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?,
                bytes,
            )?;
        }
        Ok(written)
    }

    fn path(&self, below: &str) -> String {
        let mut path = self.0.clone();
        path.extend(below.split('/'));
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Written {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What every session of a connection is given, worked out once the agent's `initialize` answer says what it takes.
#[derive(Debug, Default)]
struct Parts {
    system_prompt: Option<String>,
    servers: Vec<Value>,
    /// Every MCP server name the profile holds, started or not: a client's server may not take one.
    names: Vec<String>,
    directories: Vec<String>,
    meta: Option<Value>,
    report: Value,
    #[allow(
        dead_code,
        reason = "held so the directory is removed with the connection, not read"
    )]
    written: Option<Written>,
}

/// What ACP's `mcpServers` entry carries: the command or URL, the variables or headers, and a stdio server's arguments.
#[derive(Debug)]
struct Acp {
    target: String,
    pairs: Vec<(String, String)>,
    args: Value,
}

/// Resolves one variable or header: its key, its name, its value as written; `None` when it is left out.
type Resolve<'f> = dyn FnMut(&str, &str, &EnvValue) -> Result<Option<String>, String> + 'f;

/// `values` resolved, those left out dropped.
fn resolved(
    name: &str,
    table: &str,
    values: &IndexMap<String, EnvValue>,
    value: &mut Resolve<'_>,
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (key, raw) in values {
        if let Some(found) = value(&format!("mcp.{name}.{table}.{key}"), key, raw)? {
            out.push((key.clone(), found));
        }
    }
    Ok(out)
}

/// One MCP server of the profile, resolved.
#[derive(Debug)]
struct ReadyMcp {
    name: String,
    layer: LayerId,
    digest: String,
    transport: &'static str,
    acp: Option<Acp>,
    withheld: Vec<String>,
    refused: Option<String>,
}

/// The digest an `allow_mcp` entry names: of the layer and the server as the profile's view writes it.
fn mcp_digest(layer: &LayerId, server: &McpServer) -> String {
    let definition = json!({"layer": layer.to_string(), "server": super::mcp_json(server)});
    let hash = Sha256::digest(serde_json::to_vec(&definition).unwrap_or_default());
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// Each overlay's environment as it stood once that overlay applied, by identity: what its MCP servers' text reads.
fn environments_after<'a>(
    effective: &Effective,
    base: aos::Base<'a>,
) -> BTreeMap<String, Result<aos::Environment<'a>, String>> {
    let mut out = BTreeMap::new();
    for (index, layer) in effective.aos.iter().enumerate() {
        let built = build(&effective.aos[..=index], &effective.share_env, base)
            .map_err(|error| error.to_string());
        out.insert(layer.identity.clone(), built);
    }
    out
}

fn ready_mcp(
    name: &str,
    sourced: &Sourced<McpServer>,
    effective: &Effective,
    after: &BTreeMap<String, Result<aos::Environment<'_>, String>>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> ReadyMcp {
    let layer = sourced.layer.clone();
    let server = &sourced.value;
    let overlay = (layer.kind == LayerKind::Overlay)
        .then(|| effective.aos.iter().find(|aos| aos.identity == layer.name))
        .flatten();
    let windows = cfg!(windows);
    let mut withheld = Vec::new();
    let transport = match server {
        McpServer::Stdio { .. } => "stdio",
        McpServer::Http { .. } => "http",
        McpServer::Sse { .. } => "sse",
    };
    let mut value = |key: &str, label: &str, raw: &EnvValue| -> Result<Option<String>, String> {
        match (raw, overlay) {
            (EnvValue::Text(text), Some(aos)) => {
                let env = match after.get(&aos.identity) {
                    Some(Ok(env)) => env,
                    Some(Err(why)) => return Err(why.clone()),
                    None => return Err("its overlay's environment was not built".to_owned()),
                };
                let lookup = |variable: &str| -> Result<Option<String>, aos::AosError> {
                    let same = |name: &str| {
                        variable == name || windows && variable.eq_ignore_ascii_case(name)
                    };
                    if same("AOS_OVERLAY_ROOT") {
                        return Ok(Some(aos.root.clone()));
                    }
                    if same("AOS_OVERLAY_NAME") {
                        return Ok(Some(aos::normalize_name(&aos.identity)));
                    }
                    env.lookup(variable)
                };
                aos::substitute(text, &lookup).map(Some).map_err(|error| {
                    format!(
                        "its text cannot be substituted ({}: {})",
                        error.code, error.subject
                    )
                })
            }
            (EnvValue::Path(path), Some(aos)) => Ok(Some(resolve_path(&aos.root, path, windows))),
            (
                EnvValue::Reference {
                    reference,
                    optional,
                },
                _,
            ) => {
                let (provider, variable) = reference.split_once(':').unwrap_or((reference, ""));
                if provider != "env" {
                    withheld.push(format!(
                        "{label}: reads {reference}, a secret this host does not hold"
                    ));
                    return Ok(None);
                }
                if !granted(&effective.share_env, variable, windows) {
                    withheld.push(format!(
                        "{label}: reads ${variable}, which this host's profile does not share"
                    ));
                    return Ok(None);
                }
                match lookup(variable) {
                    Some(found) => Ok(Some(found)),
                    None if *optional => {
                        withheld.push(format!("{label}: ${variable} is not set"));
                        Ok(None)
                    }
                    None => Err(format!("{key} reads a variable that is not set")),
                }
            }
            _ => resolve(
                key,
                raw,
                sourced.dir.as_deref(),
                layer.kind.is_confined(),
                lookup,
            )
            .map(|secret| Some(secret.expose().to_owned()))
            .map_err(|error| error.to_string()),
        }
    };
    let acp = match server {
        McpServer::Stdio { command, args, env } => {
            resolved(name, "env", env, &mut value).map(|env| {
                let program = if Path::new(command).is_absolute() || !command.contains(['/', '\\'])
                {
                    command.clone()
                } else {
                    sourced.dir.as_ref().map_or_else(
                        || command.clone(),
                        |dir| dir.join(command).to_string_lossy().into_owned(),
                    )
                };
                Acp {
                    target: program,
                    pairs: env,
                    args: json!(args),
                }
            })
        }
        McpServer::Http { url, headers } | McpServer::Sse { url, headers } => {
            resolved(name, "headers", headers, &mut value).map(|headers| Acp {
                target: url.clone(),
                pairs: headers,
                args: Value::Null,
            })
        }
    };
    let (acp, refused) = match acp {
        Ok(acp) => (Some(acp), None),
        Err(why) => (None, Some(why)),
    };
    ReadyMcp {
        name: name.to_owned(),
        digest: mcp_digest(&layer, server),
        layer,
        transport,
        acp,
        withheld,
        refused,
    }
}

/// `server` as ACP `mcpServers` writes it in protocol `version`: v1's stdio form has no type.
fn acp_server(server: &ReadyMcp, version: u64) -> Option<Value> {
    let Acp {
        target: first,
        pairs,
        args,
    } = server.acp.as_ref()?;
    let pairs: Vec<Value> = pairs
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    Some(if server.transport == "stdio" {
        let mut out = json!({"name": server.name, "command": first, "args": args, "env": pairs});
        if version == 2 {
            out["type"] = json!("stdio");
        }
        out
    } else {
        json!({"type": server.transport, "name": server.name, "url": first, "headers": pairs})
    })
}

/// A context's text: an overlay's from the bytes its digest covered, the host's own from its file.
fn context_text(
    context: &Sourced<Context>,
    overlays: &[ReadOverlay],
) -> Result<(String, String), String> {
    match &context.value {
        Context::Text(text) => Ok((text.clone(), "text".to_owned())),
        Context::File(file) => {
            let text = if context.layer.kind == LayerKind::Overlay {
                overlays
                    .iter()
                    .find(|overlay| overlay.identity == context.layer.name)
                    .and_then(|overlay| overlay.file(file))
                    .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
                    .ok_or_else(|| format!("the file {file} cannot be read as UTF-8 text"))?
            } else {
                resolve(
                    "context",
                    &EnvValue::FromFile(file.clone()),
                    context.dir.as_deref(),
                    context.layer.kind.is_confined(),
                    |_| None,
                )
                .map_err(|_| {
                    format!("the file {file} cannot be read as UTF-8 text of at most 1 MiB")
                })?
                .expose()
                .to_owned()
            };
            Ok((text, file.clone()))
        }
    }
}

/// A skill's files: an overlay's from its digest-covered bytes, the host's own read once from its directory, which may
/// hold no link or special file.
fn skill_files(
    skill: &Sourced<super::Skill>,
    overlays: &[ReadOverlay],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let files = if skill.layer.kind == LayerKind::Overlay {
        overlays
            .iter()
            .find(|overlay| overlay.identity == skill.layer.name)
            .map(|overlay| overlay.under(&skill.value.path))
            .unwrap_or_default()
    } else {
        let path = Path::new(&skill.value.path);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            skill
                .dir
                .as_ref()
                .ok_or("its layer has no directory")?
                .join(path)
        };
        let tree = aos::read_tree(&path, MAX_SKILL_FILES, MAX_SKILL_BYTES).map_err(|error| {
            format!(
                "its files cannot be read ({}: {})",
                error.code, error.subject
            )
        })?;
        if tree
            .iter()
            .any(|entry| matches!(entry.kind, EntryKind::Symlink | EntryKind::Special))
        {
            return Err("its directory holds a link or a special file".to_owned());
        }
        tree.into_iter()
            .filter(|entry| entry.kind == EntryKind::File && !entry.is_excluded())
            .filter_map(|entry| {
                Some((
                    String::from_utf8(entry.components.join(&b'/')).ok()?,
                    entry.data,
                ))
            })
            .collect()
    };
    if files.contains_key("SKILL.md") {
        Ok(files)
    } else {
        Err("its directory holds no SKILL.md".to_owned())
    }
}

/// The profile as `harness` takes it, for an agent that says it `takes` what it does.
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the profile's parts, as the app's launchFor makes it"
)]
fn parts_for(
    connection: &Connection,
    harness: Option<&str>,
    takes: &Takes,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Parts {
    let mut notes = Vec::new();
    let layers = layers_of(&connection.host, &connection.overlays, harness, &mut notes);
    let effective = merge(&layers);
    let mut items = connection.notes.clone();
    items.extend(notes);
    items.extend(connection.env_items.iter().cloned());

    let mut texts = Vec::new();
    let mut bytes = 0;
    for context in effective.contexts.iter().chain(effective.context.as_ref()) {
        match context_text(context, &connection.overlays) {
            Ok((text, source)) => {
                bytes += text.len();
                if bytes > MAX_CONTEXT {
                    items.push(Item::new(
                        "context",
                        &context.layer,
                        State::Refused,
                        format!("the initial context passes {MAX_CONTEXT} bytes"),
                    ));
                } else {
                    texts.push((text, context.layer.clone(), source));
                }
            }
            Err(why) => items.push(Item::new("context", &context.layer, State::Refused, why)),
        }
    }
    let mut system_prompt = None;
    if !texts.is_empty() {
        let taken = takes.system_prompt();
        if taken {
            system_prompt = Some(
                texts
                    .iter()
                    .map(|(text, ..)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
        }
        for (_, layer, source) in &texts {
            items.push(if taken {
                Item::new(
                    "context",
                    layer,
                    State::Applied,
                    format!("ACP systemPrompt, appended ({source})"),
                )
            } else {
                Item::new(
                    "context",
                    layer,
                    State::Unsupported,
                    context_not_taken(harness),
                )
            });
        }
    }

    let mut written_files = BTreeMap::new();
    let mut usable = Vec::new();
    for (name, skill) in &effective.skills {
        match skill_files(skill, &connection.overlays) {
            Ok(files) => usable.push((name, &skill.layer, files)),
            Err(why) => items.push(Item::new(
                format!("skills.{name}"),
                &skill.layer,
                State::Refused,
                why,
            )),
        }
    }
    let mut written = None;
    let mut meta = None;
    let mut directories = Vec::new();
    if !usable.is_empty() {
        let prefix = match harness {
            Some("claude-code") => Some(format!("{CLAUDE_PLUGIN}/skills")),
            Some("codex") if takes.additional_directories() => Some(CODEX_SKILLS.to_owned()),
            _ => None,
        };
        match prefix {
            None => {
                for (name, layer, _) in &usable {
                    items.push(Item::new(
                        format!("skills.{name}"),
                        layer,
                        State::Unsupported,
                        skills_not_taken(harness),
                    ));
                }
            }
            Some(prefix) => {
                for (name, _, files) in &usable {
                    for (path, data) in files {
                        written_files.insert(format!("{prefix}/{name}/{path}"), data.clone());
                    }
                }
                let claude = harness == Some("claude-code");
                if claude {
                    written_files.insert(
                        format!("{CLAUDE_PLUGIN}/.claude-plugin/plugin.json"),
                        serde_json::to_vec(&json!({
                            "name": CLAUDE_PLUGIN,
                            "version": "0.0.0",
                            "description": "The skills of the host's profile for this session",
                        }))
                        .unwrap_or_default(),
                    );
                }
                match Written::create(&written_files) {
                    Ok(done) => {
                        for (name, layer, _) in &usable {
                            let how = if claude {
                                format!(
                                    "the Claude plugin {CLAUDE_PLUGIN}, through the adapter's _meta, as {CLAUDE_PLUGIN}:{name}"
                                )
                            } else {
                                "a directory of skills Codex reads, through ACP additionalDirectories".to_owned()
                            };
                            items.push(Item::new(
                                format!("skills.{name}"),
                                layer,
                                State::Applied,
                                how,
                            ));
                        }
                        if claude {
                            meta = Some(json!({"claudeCode": {"options": {"plugins": [
                                {"type": "local", "path": done.path(CLAUDE_PLUGIN)}
                            ]}}}));
                        } else {
                            directories.push(done.0.to_string_lossy().into_owned());
                        }
                        written = Some(done);
                    }
                    Err(error) => {
                        for (name, layer, _) in &usable {
                            items.push(Item::new(
                                format!("skills.{name}"),
                                layer,
                                State::Refused,
                                format!("its files could not be written for the agent ({error})"),
                            ));
                        }
                    }
                }
            }
        }
    }

    let windows = cfg!(windows);
    let base = base_of(&effective.share_env, windows, lookup);
    let after = environments_after(&effective, &base);
    let mut servers = Vec::new();
    let mut names = Vec::new();
    for (name, sourced) in &effective.mcp {
        names.push(name.clone());
        let ready = ready_mcp(name, sourced, &effective, &after, lookup);
        let key = format!("mcp.{name}");
        let allowed = !ready.layer.kind.is_confined()
            || connection
                .host
                .allow_mcp
                .get(name)
                .is_some_and(|digest| aos::same_digest(digest, &ready.digest));
        let mut item = if let Some(why) = &ready.refused {
            Item::new(key, &ready.layer, State::Refused, why.clone())
        } else if !allowed {
            tracing::info!(
                entry = connection.entry,
                "mcp.{name} of {} is off: host.profiles.{}.allow_mcp.{name} = \"{}\" allows it",
                ready.layer,
                connection.name,
                ready.digest
            );
            Item::new(
                key,
                &ready.layer,
                State::Off,
                "not allowed by the host's configuration: allow_mcp names it at this digest",
            )
        } else if harness == Some("pi") {
            Item::new(
                key,
                &ready.layer,
                State::Unsupported,
                "pi-acp accepts mcpServers and starts none of them",
            )
        } else if !takes.mcp(ready.transport) {
            Item::new(
                key,
                &ready.layer,
                State::Unsupported,
                format!(
                    "the agent does not say it starts MCP servers over {}",
                    ready.transport
                ),
            )
        } else if let Some(server) = acp_server(&ready, takes.version) {
            servers.push(server);
            let mut said = vec!["ACP mcpServers".to_owned()];
            if harness == Some("codex") {
                said.push(format!(
                    "Codex skips it if its own configuration names a server {name}"
                ));
            }
            if !ready.withheld.is_empty() {
                said.push(format!("left out: {}", ready.withheld.join(", ")));
            }
            Item::new(key, &ready.layer, State::Applied, said.join("; "))
        } else {
            Item::new(key, &ready.layer, State::Refused, "it cannot be resolved")
        };
        item.digest = Some(ready.digest);
        items.push(item);
    }

    for (key, value) in [
        ("model", &effective.model),
        ("effort", &effective.effort),
        ("mode", &effective.mode),
    ] {
        if let Some(value) = value {
            items.push(Item::new(
                key,
                &value.layer,
                State::Unsupported,
                "the host does not set it; the session's options do",
            ));
        }
    }

    let label = harness.unwrap_or(&connection.entry);
    let applied = items
        .iter()
        .filter(|item| item.state == State::Applied)
        .count();
    let missed: Vec<String> = items
        .iter()
        .filter(|item| item.state != State::Applied)
        .map(|item| format!("{} ({})", item.key, item.state.as_str()))
        .collect();
    let summary = format!(
        "Profile {} for {label}: {applied} applied{}",
        connection.name,
        if missed.is_empty() {
            String::new()
        } else {
            format!("; not applied: {}", missed.join(", "))
        }
    );
    Parts {
        system_prompt,
        servers,
        names,
        directories,
        meta,
        report: json!({
            "name": connection.name,
            "harness": label,
            "summary": summary,
            "items": items.iter().map(Item::json).collect::<Vec<_>>(),
        }),
        written,
    }
}

/// Adds `parts` to a session request's `params`. A client's MCP server may not take a name the profile holds.
fn add_parts(params: &mut Map<String, Value>, parts: &Parts) -> Result<(), &'static str> {
    let servers = params.entry("mcpServers").or_insert_with(|| json!([]));
    let servers = servers.as_array_mut().ok_or("mcpServers is not a list")?;
    if servers.iter().any(|server| {
        server
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| parts.names.iter().any(|held| held == name))
    }) {
        return Err("an MCP server of that name comes from the host's profile");
    }
    servers.extend(parts.servers.iter().cloned());
    if let Some(text) = &parts.system_prompt {
        let block = json!({"type": "text", "text": text});
        match params
            .get_mut("systemPrompt")
            .and_then(Value::as_object_mut)
        {
            Some(prompt) => match prompt.get_mut("content").and_then(Value::as_array_mut) {
                Some(content) => content.insert(0, block),
                None => {
                    prompt.insert("content".to_owned(), json!([block]));
                }
            },
            None => {
                params.insert(
                    "systemPrompt".to_owned(),
                    json!({"mode": "append", "content": [block]}),
                );
            }
        }
    }
    if !parts.directories.is_empty() {
        let more = params
            .entry("additionalDirectories")
            .or_insert_with(|| json!([]));
        more.as_array_mut()
            .ok_or("additionalDirectories is not a list")?
            .extend(parts.directories.iter().map(|directory| json!(directory)));
    }
    if let Some(meta) = &parts.meta {
        params.insert("_meta".to_owned(), meta.clone());
    }
    Ok(())
}

/// Puts the report under [`PROFILE_META`] in an answer's `_meta`.
fn add_report(result: &mut Value, report: &Value) {
    let Some(result) = result.as_object_mut() else {
        return;
    };
    let meta = result.entry("_meta").or_insert_with(|| json!({}));
    if !meta.is_object() {
        *meta = json!({});
    }
    meta[PROFILE_META] = report.clone();
}

/// Whether `message` names a profile under `params._meta`.
fn names_profile(message: &[u8]) -> bool {
    serde_json::from_slice::<Value>(message)
        .ok()
        .and_then(|value| {
            value
                .get("params")?
                .get("_meta")?
                .get(PROFILE_META)
                .cloned()
        })
        .is_some()
}

#[derive(Debug)]
enum Pending {
    Initialize,
    Session,
}

#[derive(Debug)]
struct Hold {
    launch: Option<Vec<u8>>,
    first: bool,
    pending: BTreeMap<String, Pending>,
    parts: Option<Parts>,
}

/// A connection's channel to its agent with a profile applied: the launch line first, then each message the client sends
/// as [`Scope`] passes it, with the profile's parts added to session requests; and the report added to the agent's
/// answers to `initialize` and to each session request.
pub(crate) struct Hosted<'a, C: Channel + ?Sized> {
    inner: &'a C,
    scope: &'a Scope,
    connection: &'a Connection,
    hold: Mutex<Hold>,
}

impl<C: Channel + ?Sized> core::fmt::Debug for Hosted<'_, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hosted").finish_non_exhaustive()
    }
}

impl<'a, C: Channel + ?Sized> Hosted<'a, C> {
    pub(crate) fn new(inner: &'a C, scope: &'a Scope, connection: &'a Connection) -> Self {
        Self {
            inner,
            scope,
            connection,
            hold: Mutex::new(Hold {
                launch: Some(connection.launch_line()),
                first: true,
                pending: BTreeMap::new(),
                parts: None,
            }),
        }
    }

    fn hold(&self) -> std::sync::MutexGuard<'_, Hold> {
        self.hold.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reply(&self, message: &[u8]) -> Result<(), Failure> {
        let until = Instant::now() + REPLY_LIMIT;
        loop {
            match self.inner.send(message) {
                Err(Failure::NoProgress) if Instant::now() < until => {
                    std::thread::sleep(REPLY_POLL)
                }
                other => return other,
            }
        }
    }

    /// Records the request, and adds the profile's parts to a session request; `Err` is the answer refusing it.
    fn forward(&self, bytes: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(bytes);
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(id) = value.get("id").cloned() else {
            return Ok(bytes);
        };
        let mut hold = self.hold();
        let tracked = method == "initialize" || SESSION_METHODS.contains(&method.as_str());
        if tracked && hold.pending.len() >= MAX_PENDING {
            return Err(error(
                &id,
                INVALID_REQUEST,
                "too many requests wait for the agent's answer",
            ));
        }
        if method == "initialize" {
            hold.pending.insert(id.to_string(), Pending::Initialize);
            return Ok(bytes);
        }
        if !tracked {
            return Ok(bytes);
        }
        let Some(parts) = &hold.parts else {
            return Err(error(
                &id,
                INVALID_REQUEST,
                "initialize comes first on an entry with a profile",
            ));
        };
        let params = value
            .as_object_mut()
            .map(|object| object.entry("params").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut);
        let Some(params) = params else {
            return Err(error(&id, INVALID_PARAMS, "params must be an object"));
        };
        add_parts(params, parts).map_err(|why| error(&id, INVALID_PARAMS, why))?;
        hold.pending.insert(id.to_string(), Pending::Session);
        serde_json::to_vec(&value)
            .map_err(|_| error(&id, INTERNAL_ERROR, "the request could not be written"))
    }
}

impl<C: Channel + ?Sized> Channel for Hosted<'_, C> {
    /// An answer to `initialize` or a session request gets the report; the first also settles what each session is given.
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let mut hold = self.hold();
        if hold.pending.is_empty() {
            drop(hold);
            return self.inner.send(message);
        }
        let parsed = serde_json::from_slice::<Value>(message)
            .ok()
            .filter(|value| value.get("method").is_none());
        let Some(mut value) = parsed else {
            drop(hold);
            return self.inner.send(message);
        };
        let pending = value
            .get("id")
            .and_then(|id| hold.pending.remove(&id.to_string()));
        match pending {
            // What sessions are given is settled by the first answer; a later `initialize` changes nothing.
            Some(Pending::Initialize) if hold.parts.is_some() => {
                if let (Some(result), Some(parts)) = (value.get_mut("result"), &hold.parts) {
                    add_report(result, &parts.report);
                }
            }
            Some(Pending::Initialize) => {
                if let Some(result) = value.get("result") {
                    let takes = Takes {
                        version: result
                            .get("protocolVersion")
                            .and_then(Value::as_u64)
                            .unwrap_or(1),
                        capabilities: result
                            .get("agentCapabilities")
                            .cloned()
                            .unwrap_or(Value::Null),
                    };
                    let reported = result
                        .get("agentInfo")
                        .and_then(|info| info.get("name"))
                        .and_then(Value::as_str);
                    let harness = harness_of(reported, &self.connection.entry);
                    let lookup = |name: &str| std::env::var(name).ok();
                    let parts = parts_for(self.connection, harness, &takes, &lookup);
                    if let Some(result) = value.get_mut("result") {
                        add_report(result, &parts.report);
                    }
                    hold.parts = Some(parts);
                }
            }
            Some(Pending::Session) => {
                if let (Some(result), Some(parts)) = (value.get_mut("result"), &hold.parts) {
                    add_report(result, &parts.report);
                }
            }
            None => {
                drop(hold);
                return self.inner.send(message);
            }
        }
        drop(hold);
        match serde_json::to_vec(&value) {
            Ok(bytes) => self.inner.send(&bytes),
            Err(_) => self.inner.send(message),
        }
    }

    /// The launch line first; then the client's messages, a refused one reading as a timeout as [`crate::scope`] has it.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let first = {
            let mut hold = self.hold();
            if let Some(line) = hold.launch.take() {
                return Ok(Some(line));
            }
            core::mem::replace(&mut hold.first, false)
        };
        let Some(message) = self.inner.receive(timeout)? else {
            return Ok(None);
        };
        if !first && names_profile(&message) {
            if let Some(id) = serde_json::from_slice::<Value>(&message)
                .ok()
                .and_then(|value| value.get("id").cloned())
            {
                self.reply(&error(
                    &id,
                    INVALID_PARAMS,
                    "a profile is chosen in initialize, once",
                ))?;
            }
            tracing::info!(
                entry = self.connection.entry,
                "a client named a profile after initialize, which was refused"
            );
            return Err(Failure::Timeout);
        }
        let forwarded = match self.scope.check(&message) {
            Verdict::Forward(bytes) => self.forward(bytes),
            Verdict::Refuse(answer) => Err(answer.unwrap_or_default()),
        };
        match forwarded {
            Ok(bytes) => Ok(Some(bytes)),
            Err(answer) => {
                if !answer.is_empty() {
                    self.reply(&answer)?;
                }
                tracing::info!(entry = self.connection.entry, "a message was refused");
                Err(Failure::Timeout)
            }
        }
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// An entry's offer, from its profile names and the configuration's `[host.profiles]`.
pub(crate) fn offer(
    names: &[String],
    profiles: &IndexMap<String, HostProfile>,
    launcher: OsString,
) -> Option<Arc<Offer>> {
    if names.is_empty() {
        return None;
    }
    Some(Arc::new(Offer {
        profiles: names
            .iter()
            .filter_map(|name| Some((name.clone(), profiles.get(name)?.clone())))
            .collect(),
        launcher,
    }))
}

#[cfg(test)]
mod tests;
