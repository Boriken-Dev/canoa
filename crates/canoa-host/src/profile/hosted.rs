//! A `[host.profiles]` profile applied to the agent an entry starts for one connection: the environment when the process
//! starts, then context, skills and MCP servers with each session, each part where the agent's harness takes it, and a
//! report of what applied sent to the client. The client names a profile only among those its entry offers, and never
//! anything inside one.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_aos::{self as aos, Limit, OverlayTree, Settings, StdFs};
use coqui_transport::{Channel, Failure};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::overlay::{Built, build_each, granted, resolve_path};
use super::{
    Change, Context, Effective, EnvValue, Layer, LayerId, LayerKind, McpServer, OverlayInput,
    Secret, Skill, Sourced, merge, overlay_layer, resolve,
};
use crate::config::HostProfile;
use crate::scope::{Scope, Verdict};
use crate::usage_splice::{Client, Splice};
use hooks::{Book, Hooks, PreEnv, Seen};
use template::{Again, Templating};

/// The `_meta` key a client names a profile under in `initialize`, and the host's report under in its answers.
pub const PROFILE_META: &str = "dev.boriken.canoa/profile";

/// The `_meta` key a client names, in `initialize`, the overlays this connection starts with: an array of the names the
/// profile lists. It only narrows: a name the profile does not list adds nothing and is reported as ignored, and one the
/// client leaves out is reported as not chosen. Absent, the connection starts with every overlay the profile lists.
pub const OVERLAYS_META: &str = "dev.boriken.canoa/overlays";

/// The longest initial context a session is given, in UTF-8 bytes, every layer's together.
const MAX_CONTEXT: usize = 1024 * 1024;
pub(crate) mod fetched;

/// What one skill's directory may hold: 1024 files, 16 MiB.
fn skill_limits() -> Settings {
    Settings::default()
        .tree_files(Limit::at(1024))
        .tree_bytes(Limit::at(16 * 1024 * 1024))
}

/// Whether two digests are the same, comparing the hexadecimal digits without regard to case; false when either is not
/// `sha256:` and 64 of them.
fn same_digest(a: &str, b: &str) -> bool {
    matches!((aos::Digest::parse(a), aos::Digest::parse(b)), (Ok(a), Ok(b)) if a == b)
}
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
/// The method the host asks the client that opened a session which of the profile's MCP servers it may start.
pub const APPROVE_METHOD: &str = "_canoa/profile/approve";
/// The `_meta` key of `initialize` with which a client says it answers [`APPROVE_METHOD`]; one that does not is never asked.
pub const APPROVE_META: &str = "dev.boriken.canoa/approve";
/// How long the person has to answer an approval.
const APPROVE_WAIT: Duration = Duration::from_secs(120);

mod hooks;
mod template;

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
    /// What `model`, `effort` or `mode` is set to: the session's options take it, as the host cannot. Never a secret.
    value: Option<String>,
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
            value: None,
        }
    }

    fn json(&self) -> Value {
        let mut out = json!({"key": self.key, "layer": self.layer, "state": self.state.as_str(), "how": self.how});
        if let Some(digest) = &self.digest {
            out["digest"] = json!(digest);
        }
        if let Some(value) = &self.value {
            out["value"] = json!(value);
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

/// The profile `first` names in `initialize`'s `_meta`, by its index in `offer`; the first when it names none. A first
/// message other than `initialize`, and anything but the name of a profile the entry offers, is refused, before any
/// reference is read or any process starts.
pub(crate) fn choose(offer: &Offer, first: &[u8]) -> Result<usize, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let id = message.get("id").cloned();
    let choice = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(PROFILE_META));
    let refuse = |code: i64, message: &str, log: &'static str| Refusal {
        answer: id.as_ref().map(|id| error(id, code, message)),
        log,
    };
    if message.get("method").and_then(Value::as_str) != Some("initialize") {
        return Err(match choice {
            Some(_) => refuse(
                INVALID_PARAMS,
                "a profile is chosen in initialize",
                "a client named a profile outside initialize, which was refused",
            ),
            None => refuse(
                INVALID_REQUEST,
                "initialize comes first on an entry with a profile",
                "a client's first message was not initialize, which was refused",
            ),
        });
    }
    let Some(choice) = choice else {
        return Ok(0);
    };
    let refuse = |message: &str, log: &'static str| refuse(INVALID_PARAMS, message, log);
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

/// The overlays `first` chooses for the connection in `initialize`'s `_meta`; `None` when it chooses none, which is every
/// overlay the profile lists. Anything but an array of names is refused.
pub(crate) fn choose_overlays(first: &[u8]) -> Result<Option<Vec<String>>, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let Some(chosen) = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(OVERLAYS_META))
    else {
        return Ok(None);
    };
    let names: Option<Vec<String>> = chosen.as_array().and_then(|names| {
        names
            .iter()
            .map(|name| name.as_str().map(str::to_owned))
            .collect()
    });
    names.map(Some).ok_or_else(|| Refusal {
        answer: message
            .get("id")
            .map(|id| error(id, INVALID_PARAMS, "the overlays are chosen as a list of names")),
        log: "a client sent something other than a list of overlay names as its choice, which was refused",
    })
}

/// An installed overlay as read when a connection starts: its tree, never read again.
#[derive(Debug)]
struct ReadOverlay {
    identity: String,
    root: String,
    tree: OverlayTree,
    /// The skills it names from elsewhere that the app fetched and that passed the checks, by the name it gives each:
    /// their files by path.
    fetched: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
}

impl ReadOverlay {
    /// A member file's bytes, by `/`-separated path: what the digest covered.
    fn file(&self, path: &str) -> Option<&[u8]> {
        self.tree.bytes(path)
    }

    /// The member files under `prefix/`, by path below it.
    /// What planning the skills needs of the tree: every path, and the text of each bundled skill's `SKILL.md`.
    fn skill_view(&self) -> OverlayTree {
        let mut view = OverlayTree::from_paths(self.tree.paths().map(str::to_owned));
        for path in self.tree.paths() {
            let skill_md = path
                .strip_prefix("skills/")
                .and_then(|rest| rest.strip_suffix("/SKILL.md"))
                .is_some_and(|name| !name.contains('/'));
            if let (true, Some(bytes)) = (skill_md, self.tree.bytes(path)) {
                view = view.with_bytes(path, bytes.to_vec());
            }
        }
        view
    }

    fn under(&self, prefix: &str) -> BTreeMap<String, Vec<u8>> {
        if let Some(name) = prefix
            .strip_prefix(fetched::SKILLS_DIR)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return self.fetched.get(name).cloned().unwrap_or_default();
        }
        let prefix = format!("{prefix}/");
        self.tree
            .paths()
            .filter_map(|path| {
                Some((
                    path.strip_prefix(&prefix)?.to_owned(),
                    self.tree.bytes(path)?.to_vec(),
                ))
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
    /// The overlays' hooks, and the means to run them.
    hooks: Hooks,
    /// The `pre-env` runs the environment was built with: the same for every environment built after.
    pre_env: PreEnv,
    /// What was left out before any harness is known: overlays, and entries that read a variable not shared.
    notes: Vec<Item>,
    env: Vec<(String, Secret)>,
    env_items: Vec<Item>,
    /// The directory every skill directory of this connection is written under, so the provider can be told to work in it.
    skills: PathBuf,
    /// Whether the client is this process's own link, the only one the host asks to approve servers.
    in_process: bool,
}

impl Connection {
    /// Reads the overlays `offer`'s profile `index` names and resolves its environment through `lookup`.
    ///
    /// # Errors
    ///
    /// The environment cannot be built, as [`super::ProfileError`] words it: no agent starts with it.
    #[cfg(test)]
    pub(crate) fn prepare(
        offer: &Offer,
        index: usize,
        entry: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        Self::prepare_with(offer, index, entry, lookup, None)
    }

    /// As [`Self::prepare`], with the overlays the client chose: only those of them the profile lists are read, and each
    /// other overlay the profile lists is reported as not chosen for this connection.
    ///
    /// # Errors
    ///
    /// As [`Self::prepare`].
    pub(crate) fn prepare_with(
        offer: &Offer,
        index: usize,
        entry: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
        only: Option<&[String]>,
    ) -> Result<Self, String> {
        let (name, listed) = &offer.profiles[index];
        let mut notes = Vec::new();
        let narrowed;
        let host = if let Some(only) = only {
            let mut host = listed.clone();
            host.overlays.retain(|identity| {
                let chosen = only.contains(identity);
                if !chosen {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, identity.clone()),
                        State::Off,
                        "not chosen for this session",
                    ));
                }
                chosen
            });
            for asked in only.iter().filter(|asked| !listed.overlays.contains(asked)) {
                notes.push(Item::new(
                    "overlay",
                    LayerId::new(LayerKind::Overlay, asked.clone()),
                    State::Unsupported,
                    "this profile does not list it, so a client cannot add it",
                ));
            }
            narrowed = host;
            &narrowed
        } else {
            listed
        };
        let overlays = read_overlays(host, &mut notes);
        let layers = layers_of(host, &overlays, None, &mut notes);
        let effective = merge(&layers);
        let hooks = Hooks::new(
            host,
            &effective.aos,
            &overlays,
            &effective.share_env,
            lookup,
        );
        // Before the agent starts nobody can be asked, so only what canoa.cfg allows runs.
        let directory = host.directory.clone();
        let harness = harness_of(None, entry).unwrap_or("canoa");
        let pre_env = hooks.pre_env(&overlays, harness, &directory);
        let resolved = effective
            .environment_with(lookup, &pre_env.runs, &pre_env.exported)
            .map_err(|error| error.to_string())?;
        notes.extend(resolved.hook_reports.iter().map(hooks::report_item));
        notes.extend(pre_env.refused.iter().cloned());
        let env = resolved.env;
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
            for (id, subject) in &report.reports {
                if id == "env-script-not-allowed" {
                    notes.push(Item::new(
                        format!("env_scripts.{subject}"),
                        &report.layer,
                        State::Off,
                        "env-script-not-allowed: its env scripts do not run until canoa.cfg names the overlay's digest for its hooks; the rest of the overlay still applies",
                    ));
                }
            }
            for what in &report.not_applied {
                notes.push(Item::new(
                    what.clone(),
                    &report.layer,
                    State::Unsupported,
                    "Canoa does not apply it",
                ));
            }
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| error.to_string())?;
        let skills = std::env::temp_dir().join(format!(
            "canoa-profile-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        Ok(Self {
            name: name.clone(),
            entry: entry.to_owned(),
            host: host.clone(),
            overlays,
            hooks,
            pre_env,
            notes,
            env,
            env_items,
            skills,
            in_process: false,
        })
    }

    /// This connection's client is (or is not) this process's own link.
    pub(crate) fn asking(mut self, in_process: bool) -> Self {
        self.in_process = in_process;
        self
    }

    /// The directory the connection's skills are written under: created when the first is written, removed with the last.
    pub(crate) fn skills(&self) -> &Path {
        &self.skills
    }

    /// The profile's environment, resolved: what the agent's process is given.
    pub(crate) fn env(&self) -> &[(String, Secret)] {
        &self.env
    }

    /// The line `canoa launch` reads first.
    pub(crate) fn launch_line(&self) -> Vec<u8> {
        super::launch::launch_line(&self.env)
    }
}

/// Each overlay `host` names, read from `overlays/NAME` beside canoa.cfg and checked against the digest of the record
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
        let (tree, digest) = match aos::read_tree(&StdFs, &root, &Settings::default()) {
            Ok(read) => read,
            Err(error) => {
                leave_out(&format!(
                    "its files cannot be read ({}: {})",
                    error.class, error.subject
                ));
                continue;
            }
        };
        if !same_digest(&digest.digest.to_string(), expected) {
            leave_out("its files are not those its record's digest names: sync it");
            continue;
        }
        let fetched = fetched_skills(&store, identity, &tree, &layer, &host.skill_pins, notes);
        out.push(ReadOverlay {
            identity: identity.clone(),
            root: root.to_string_lossy().into_owned(),
            tree,
            fetched,
        });
    }
    out
}

/// The skills `tree`'s manifest names that it does not bundle, as the app fetched them: what passed the checks, and a
/// report of the rest.
fn fetched_skills(
    store: &Path,
    identity: &str,
    tree: &OverlayTree,
    layer: &LayerId,
    pins: &BTreeMap<String, String>,
    notes: &mut Vec<Item>,
) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
    let Ok(read) = aos::read_overlay(tree, &Settings::default()) else {
        return BTreeMap::new();
    };
    let elsewhere: Vec<aos::SkillEntry> = aos::skill_entries(&read.manifest, tree)
        .into_iter()
        .filter(|entry| entry.origin != aos::SkillOrigin::Bundled)
        .collect();
    let found = fetched::read(store, identity, &elsewhere, pins);
    for (name, state, how) in found.notes {
        notes.push(Item::new(format!("skills.{name}"), layer, state, how));
    }
    found.files
}

/// Plans the skills of all the overlays read together, as the specification has a consumer do before it delivers
/// any, and gives each layer the fetched skills it is to deliver: a skill from elsewhere never takes a name another
/// skill holds. Said once, with the first harness-less pass.
fn deliver_fetched(
    overlays: &[ReadOverlay],
    manifests: &BTreeMap<String, Map<String, Value>>,
    active: &[String],
    found: &mut BTreeMap<String, Layer>,
    harness: Option<&str>,
    notes: &mut Vec<Item>,
) {
    let views: BTreeMap<String, OverlayTree> = overlays
        .iter()
        .map(|overlay| (aos::normalize_name(&overlay.identity), overlay.skill_view()))
        .filter(|(key, _)| manifests.contains_key(key))
        .collect();
    let layer_of = |key: &str| LayerId::new(LayerKind::Overlay, key.to_owned());
    let plan = match aos::resolve_skills(
        manifests,
        active,
        &views,
        &[],
        &[],
        true,
        &Settings::default(),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            if harness.is_none() {
                notes.push(Item::new(
                    "skills",
                    LayerId::new(LayerKind::User, ""),
                    State::Refused,
                    format!(
                        "no skill from elsewhere is delivered: the overlays' skills cannot be planned ({})",
                        error.class
                    ),
                ));
            }
            return;
        }
    };
    let holder = |name: &str| {
        plan.delivered
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, overlay)| overlay.as_str())
    };
    for overlay in overlays {
        let key = aos::normalize_name(&overlay.identity);
        let Some(layer) = found.get_mut(&key) else {
            continue;
        };
        for name in overlay.fetched.keys() {
            if holder(name) == Some(key.as_str()) {
                layer.profile.skills.insert(
                    name.clone(),
                    Change::Set(Skill::new(format!("{}/{name}", fetched::SKILLS_DIR))),
                );
            }
        }
    }
    if harness.is_some() {
        return;
    }
    for (name, overlay) in &plan.shadowing {
        notes.push(Item::new(
            format!("skills.{name}"),
            layer_of(overlay),
            State::Refused,
            format!(
                "not delivered: a skill from elsewhere never takes a name another skill holds ({} holds it)",
                holder(name).unwrap_or("another overlay")
            ),
        ));
    }
    for name in &plan.collisions {
        if let Some(winner) = holder(name) {
            notes.push(Item::new(
                format!("skills.{name}.collision"),
                layer_of(winner),
                State::Applied,
                "more than one overlay offers a skill of this name; this overlay's is the one delivered",
            ));
        }
    }
    for (id, name, overlay) in &plan.non_defaults {
        let delivered = holder(name) == Some(overlay.as_str())
            && overlays.iter().any(|read| {
                aos::normalize_name(&read.identity) == *overlay && read.fetched.contains_key(name)
            });
        if delivered {
            let how = if id == "insecure-transport" {
                "its source is not over an encrypted transport"
            } else {
                "its source is on a private network address"
            };
            notes.push(Item::new(
                format!("skills.{name}.{id}"),
                layer_of(overlay),
                State::Applied,
                how,
            ));
        }
    }
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
            Ok(layer)
                if layer
                    .aos
                    .as_ref()
                    .is_some_and(|aos| !hooks::activates(host, aos, overlay)) =>
            {
                // A guard that is off would fail open, so the overlay waits for its hooks to be allowed.
                if harness.is_none() {
                    notes.push(Item::new(
                        "overlay",
                        LayerId::new(LayerKind::Overlay, overlay.identity.clone()),
                        State::Refused,
                        "left out: a hook of it fails closed and its hooks are not allowed; canoa.cfg's agents.NAME.hooks names the digest that allows them",
                    ));
                }
            }
            Ok(mut layer) => {
                let key = aos::normalize_name(&overlay.identity);
                if let Some(aos) = layer.aos.as_mut() {
                    aos.allowed = host.allow_hooks.get(&overlay.identity).cloned();
                }
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
    deliver_fetched(overlays, &manifests, &active, &mut found, harness, notes);
    let mut layers: Vec<Layer> = aos::composition_order(&manifests, &active)
        .into_iter()
        .filter_map(|key| found.remove(&key))
        .collect();
    layers.push(Layer::new(
        LayerId::new(LayerKind::User, ""),
        Some(host.directory.clone()),
        host.profile.clone(),
    ));
    if let Some((label, profile)) = &host.agent {
        layers.push(Layer::new(
            LayerId::new(LayerKind::Agent, label.clone()),
            Some(host.directory.clone()),
            profile.clone(),
        ));
    }
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

/// What an agent's `initialize` answer says it takes, and the name it gives itself: version 2 says them under
/// `capabilities` and `info`, version 1 under `agentCapabilities` and `agentInfo`.
fn described(result: &Value) -> (Takes, Option<&str>) {
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let (capabilities, info) = if version == 2 {
        ("capabilities", "info")
    } else {
        ("agentCapabilities", "agentInfo")
    };
    let takes = Takes {
        version,
        capabilities: result.get(capabilities).cloned().unwrap_or(Value::Null),
    };
    let reported = result
        .get(info)
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str);
    (takes, reported)
}

/// The AOS harness id of what an agent reports as its name, else of an entry named as Canoa's presets are.
fn harness_of(reported: Option<&str>, entry: &str) -> Option<&'static str> {
    match reported {
        Some("coqui-acp-claude" | "@agentclientprotocol/claude-agent-acp") => Some("claude-code"),
        Some("coqui-acp-codex" | "@agentclientprotocol/codex-acp") => Some("codex"),
        Some("gemini-cli") => Some("gemini-cli"),
        Some("pi-acp") => Some("pi"),
        // Pi's own translator says what it takes, as Claude's and Codex's do, so the entry's name does not decide.
        Some("coqui-acp-pi") => None,
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
    fn create(under: &Path, files: &BTreeMap<String, Vec<u8>>) -> std::io::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = under.join(name);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
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
        // The connection's directory goes with its last skills, if nothing else is in it.
        if let Some(under) = self.0.parent() {
            let _ = std::fs::remove_dir(under);
        }
    }
}

/// What every session of a connection is given, worked out once the agent's `initialize` answer says what it takes.
#[derive(Debug, Default)]
struct Parts {
    system_prompt: Option<String>,
    /// The system prompt's texts in order, each with what replacing it again needs when it names `AOS_PROJECT_ROOT`.
    pieces: Vec<(String, Option<Again>)>,
    /// Whether the agent takes a system prompt with a new session, whether the profile gives one or not.
    system_prompt_taken: bool,
    servers: Vec<Value>,
    /// Every MCP server name the profile holds, started or not: a client's server may not take one.
    names: Vec<String>,
    directories: Vec<String>,
    meta: Option<Value>,
    report: Value,
    /// Servers the configuration does not allow and the person may, each ready to be added to one session.
    approvable: Vec<Approvable>,
    #[allow(
        dead_code,
        reason = "held so the directory is removed with the connection, not read"
    )]
    written: Option<Written>,
}

impl Parts {
    /// The system prompt for a session opened in `project`: the texts that name `AOS_PROJECT_ROOT` replaced again with it.
    fn system_prompt_for(&self, project: Option<&str>) -> Result<Option<String>, String> {
        let Some(prompt) = &self.system_prompt else {
            return Ok(None);
        };
        let Some(project) =
            project.filter(|_| self.pieces.iter().any(|(_, again)| again.is_some()))
        else {
            return Ok(Some(prompt.clone()));
        };
        let mut texts = Vec::new();
        for (text, again) in &self.pieces {
            texts.push(match again {
                Some(again) => again.render(Some(project))?,
                None => text.clone(),
            });
        }
        let joined = texts.join("\n\n");
        if joined.len() > MAX_CONTEXT {
            return Err(format!(
                "the initial context passes {MAX_CONTEXT} bytes with this session's folder in it"
            ));
        }
        Ok(Some(joined))
    }
}

/// An MCP server of an overlay or a project that canoa.cfg does not allow, and what the client is shown of it.
#[derive(Debug)]
struct Approvable {
    name: String,
    layer: String,
    digest: String,
    server: Value,
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
/// A stdio server's `program` as found is part of it, so another program found by the same name is off again.
fn mcp_digest(layer: &LayerId, server: &McpServer, program: Option<&str>) -> String {
    let mut definition = json!({"layer": layer.to_string(), "server": super::mcp_json(server)});
    if let Some(program) = program {
        definition["program"] = json!(program);
    }
    aos::Digest::of(&serde_json::to_vec(&definition).unwrap_or_default()).to_string()
}

/// The program a stdio server's `command` names, absolute: as written when absolute, from its layer's directory when it
/// holds a separator, else the first match on the host's `PATH` (with `PATHEXT` on Windows), where only absolute entries
/// count, since a relative one would name whatever directory the agent runs in.
fn program_of(
    command: &str,
    dir: Option<&Path>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if Path::new(command).is_absolute() {
        return Ok(command.to_owned());
    }
    if command.contains(['/', '\\']) {
        return dir
            .map(|dir| dir.join(command).to_string_lossy().into_owned())
            .ok_or_else(|| format!("{command} is relative, and its layer has no directory"));
    }
    let not_found = || format!("{command} is not found on the host's PATH");
    let path = lookup("PATH").ok_or_else(not_found)?;
    let extensions: Vec<String> = if cfg!(windows) && Path::new(command).extension().is_none() {
        lookup("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    for directory in std::env::split_paths(&path).filter(|directory| directory.is_absolute()) {
        for extension in &extensions {
            let candidate = directory.join(format!("{command}{extension}"));
            if runnable(&candidate) {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }
    Err(not_found())
}

/// A regular file, executable by someone on Unix.
fn runnable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| {
        #[cfg(unix)]
        let executable = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
        #[cfg(not(unix))]
        let executable = true;
        meta.is_file() && executable
    })
}

/// Each overlay's environment as it stood once that overlay applied, by identity: what its MCP servers' text reads.
fn environments_after(
    effective: &Effective,
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
) -> BTreeMap<String, Result<Built, String>> {
    build_each(&effective.aos, &effective.share_env, lookup, pre_env)
        .into_iter()
        .zip(&effective.aos)
        .map(|(built, layer)| {
            (
                layer.identity.clone(),
                built.map_err(|error| error.to_string()),
            )
        })
        .collect()
}

fn ready_mcp(
    name: &str,
    sourced: &Sourced<McpServer>,
    effective: &Effective,
    after: &BTreeMap<String, Result<Built, String>>,
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
                let lookup = |variable: &str| -> Result<Option<String>, aos::Error> {
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
                        error.class, error.subject
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
            program_of(command, sourced.dir.as_deref(), lookup).and_then(|program| {
                resolved(name, "env", env, &mut value).map(|env| Acp {
                    target: program,
                    pairs: env,
                    args: json!(args),
                })
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
        digest: mcp_digest(
            &layer,
            server,
            acp.as_ref()
                .filter(|_| transport == "stdio")
                .map(|acp| acp.target.as_str()),
        ),
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

/// A context's text: an overlay's from the bytes its digest covered, the host's own from its file. An overlay's context
/// file has its `${NAME}` names replaced.
fn context_text(
    context: &Sourced<Context>,
    overlays: &[ReadOverlay],
    templating: &Templating<'_>,
) -> Result<(String, String, Option<Again>), String> {
    match &context.value {
        Context::Text(text) => Ok((text.clone(), "text".to_owned(), None)),
        Context::TemplateBody(body) => {
            let rendered = templating.body(&context.layer.name, body)?;
            Ok((rendered.text, "text".to_owned(), rendered.again))
        }
        Context::Template(file) => {
            let rendered = templating.context(&context.layer.name, file)?;
            Ok((rendered.text, file.clone(), rendered.again))
        }
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
            Ok((text, file.clone(), None))
        }
    }
}

/// A skill's files: an overlay's from its digest-covered bytes, the host's own read once from its directory, which may
/// hold no link or special file.
fn skill_files(
    skill: &Sourced<super::Skill>,
    overlays: &[ReadOverlay],
    templating: &Templating<'_>,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = if skill.layer.kind == LayerKind::Overlay {
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
        let (tree, _) =
            aos::read_tree(&StdFs, &path, &skill_limits()).map_err(|error| match error.class {
                aos::Class::Symlink | aos::Class::SpecialFile => {
                    "its directory holds a link or a special file".to_owned()
                }
                _ => format!(
                    "its files cannot be read ({}: {})",
                    error.class, error.subject
                ),
            })?;
        tree.paths()
            .filter_map(|path| Some((path.to_owned(), tree.bytes(path)?.to_vec())))
            .collect()
    };

    // A skill the app fetched is delivered as it was fetched; a bundled one has its body replaced.
    let bundled = !skill
        .value
        .path
        .starts_with(&format!("{}/", fetched::SKILLS_DIR));
    if skill.layer.kind == LayerKind::Overlay
        && bundled
        && let Some(bytes) = files.get("SKILL.md")
    {
        let replaced = templating.skill(&skill.layer.name, bytes)?;
        files.insert("SKILL.md".to_owned(), replaced);
    }
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

    let templating = Templating {
        overlays: &connection.overlays,
        hooks: &connection.hooks,
        layers: &effective.aos,
        options: &connection.host.options,
    };
    let mut texts = Vec::new();
    let mut bytes = 0;
    for context in effective.contexts.iter().chain(effective.context.as_ref()) {
        match context_text(context, &connection.overlays, &templating) {
            Ok((text, source, again)) => {
                bytes += text.len();
                if bytes > MAX_CONTEXT {
                    items.push(Item::new(
                        "context",
                        &context.layer,
                        State::Refused,
                        format!("the initial context passes {MAX_CONTEXT} bytes"),
                    ));
                } else {
                    texts.push((text, context.layer.clone(), source, again));
                }
            }
            Err(why) => items.push(Item::new("context", &context.layer, State::Refused, why)),
        }
    }
    let mut system_prompt = None;
    let mut pieces = Vec::new();
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
            pieces = texts
                .iter()
                .map(|(text, _, _, again)| (text.clone(), again.clone()))
                .collect();
        }
        for (_, layer, source, _) in &texts {
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
        match skill_files(skill, &connection.overlays, &templating) {
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
                match Written::create(connection.skills(), &written_files) {
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

    let after = environments_after(&effective, lookup, &connection.pre_env.runs);
    let mut servers = Vec::new();
    let mut approvable = Vec::new();
    let mut names = Vec::new();
    for (name, sourced) in &effective.mcp {
        names.push(name.clone());
        let ready = ready_mcp(name, sourced, &effective, &after, lookup);
        let key = format!("mcp.{name}");
        let allowed = !connection.host.off_mcp.contains(name)
            && (!ready.layer.kind.is_confined()
                || connection
                    .host
                    .allow_mcp
                    .get(name)
                    .is_some_and(|digest| same_digest(digest, &ready.digest)));
        let mut item = if let Some(why) = &ready.refused {
            Item::new(key, &ready.layer, State::Refused, why.clone())
        } else if !allowed {
            if !connection.host.off_mcp.contains(name)
                && harness != Some("pi")
                && takes.mcp(ready.transport)
                && let Some(server) = acp_server(&ready, takes.version)
            {
                approvable.push(Approvable {
                    name: name.clone(),
                    layer: ready.layer.to_string(),
                    digest: ready.digest.clone(),
                    server,
                });
            }
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
            let mut item = Item::new(
                key,
                &value.layer,
                State::Unsupported,
                "the host does not set it; the session's options do",
            );
            item.value = Some(value.value.to_string());
            items.push(item);
        }
    }

    items.extend(connection.hooks.items(&BTreeSet::new()));

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
        pieces,
        system_prompt_taken: takes.system_prompt(),
        servers,
        names,
        directories,
        meta,
        approvable,
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
fn add_parts(params: &mut Map<String, Value>, parts: &Parts) -> Result<(), String> {
    let servers = params.entry("mcpServers").or_insert_with(|| json!([]));
    let servers = servers
        .as_array_mut()
        .ok_or_else(|| "mcpServers is not a list".to_owned())?;
    if servers.iter().any(|server| {
        server
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| parts.names.iter().any(|held| held == name))
    }) {
        return Err("an MCP server of that name comes from the host's profile".to_owned());
    }
    servers.extend(parts.servers.iter().cloned());
    let project = params.get("cwd").and_then(Value::as_str).map(str::to_owned);
    if let Some(text) = parts.system_prompt_for(project.as_deref())? {
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
            .ok_or_else(|| "additionalDirectories is not a list".to_owned())?
            .extend(parts.directories.iter().map(|directory| json!(directory)));
    }
    if let Some(meta) = &parts.meta {
        params.insert("_meta".to_owned(), meta.clone());
    }
    Ok(())
}

/// The report with the servers the person allowed for this session shown as applied.
fn mark_allowed(report: &mut Value, allowed: &[String]) {
    let Some(items) = report.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    for item in items {
        let named = item
            .get("key")
            .and_then(Value::as_str)
            .and_then(|key| key.strip_prefix("mcp."))
            .is_some_and(|name| allowed.iter().any(|each| each == name));
        if named {
            item["state"] = json!(State::Applied.as_str());
            item["how"] = json!("ACP mcpServers; allowed for this session when the host asked");
        }
    }
}

/// The report with what the hooks of this session did in place of what the profile said of them before any was asked.
fn replace_hook_items(report: &mut Value, notes: &[Item]) {
    let Some(items) = report.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    if notes.is_empty() {
        return;
    }
    items.retain(|item| {
        !item
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| key.starts_with("hooks."))
    });
    items.extend(notes.iter().map(Item::json));
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
    /// A session request, with the servers the client allowed for it.
    Session(Vec<String>),
}

#[derive(Debug)]
struct Hold {
    launch: Option<Vec<u8>>,
    first: bool,
    pending: BTreeMap<String, Pending>,
    parts: Option<Parts>,
    /// Messages the client sent while the host waited for an approval, in order, to be read before any other.
    queued: alloc::collections::VecDeque<Vec<u8>>,
    asked: u64,
    /// Whether the client said in `initialize` that it answers approvals.
    approves: bool,
    /// What the overlays' hooks remember between messages.
    book: Book,
}

/// What the person allowed when the host asked: the MCP servers, and the overlays whose hooks run in this session.
#[derive(Debug, Default)]
struct Approved {
    servers: Vec<String>,
    hooks: BTreeSet<String>,
}

/// A connection's channel to its agent with a profile applied: the launch line first, then each message the client sends
/// as [`Scope`] passes it, with the profile's parts added to session requests; and the report added to the agent's
/// answers to `initialize` and to each session request.
pub(crate) struct Hosted<'a, C: Channel + ?Sized> {
    inner: &'a C,
    scope: &'a Scope,
    connection: &'a Connection,
    hold: Mutex<Hold>,
    splice: Splice,
    sites: crate::site::Sites,
    /// An unreadable line has been logged.
    noted: core::sync::atomic::AtomicBool,
}

impl<C: Channel + ?Sized> core::fmt::Debug for Hosted<'_, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hosted").finish_non_exhaustive()
    }
}

impl<'a, C: Channel + ?Sized> Hosted<'a, C> {
    /// This channel serves the agent running as process `pid`.
    #[must_use]
    pub(crate) fn agent(mut self, pid: u32) -> Self {
        self.splice.set_agent(pid);
        self
    }

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
                queued: alloc::collections::VecDeque::new(),
                asked: 0,
                approves: false,
                book: Book::default(),
            }),
            splice: scope.splice(),
            sites: scope.sites(),
            noted: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The same, for an agent in this process, which takes its environment some other way than a first line.
    pub(crate) fn in_process(inner: &'a C, scope: &'a Scope, connection: &'a Connection) -> Self {
        let hosted = Self::new(inner, scope, connection);
        hosted.hold().launch = None;
        hosted
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

    /// For a session request, asks the client which of the servers canoa.cfg does not allow it lets this session start, with
    /// [`APPROVE_METHOD`]: what the person chose, or nothing when the client does not answer it, answers with an error, takes too
    /// long or goes away. The client's other messages meanwhile wait their turn.
    fn approvals(&self, bytes: &[u8]) -> Approved {
        let Ok(request) = serde_json::from_slice::<Value>(bytes) else {
            return Approved::default();
        };
        let session = request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| SESSION_METHODS.contains(&method));
        if !session || request.get("id").is_none() {
            return Approved::default();
        }
        let hooks = self.connection.hooks.approvable();
        let (ask_id, servers) = {
            let mut hold = self.hold();
            if !hold.approves || !self.connection.in_process {
                return Approved::default();
            }
            let Some(parts) = hold
                .parts
                .as_ref()
                .filter(|parts| !parts.approvable.is_empty() || !hooks.is_empty())
            else {
                return Approved::default();
            };
            let servers: Vec<Value> = parts
                .approvable
                .iter()
                .map(|each| json!({"name": each.name, "layer": each.layer, "digest": each.digest}))
                .collect();
            hold.asked += 1;
            // Unguessable, so nothing else can answer for the person.
            let mut nonce = [0_u8; 12];
            let _ = getrandom::fill(&mut nonce);
            let hex: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
            (format!("canoa-approve-{}-{hex}", hold.asked), servers)
        };
        let ask = json!({
            "jsonrpc": "2.0",
            "id": ask_id,
            "method": APPROVE_METHOD,
            "params": {
                "profile": self.connection.name,
                "harness": self.connection.entry,
                "servers": servers,
                "hooks": hooks,
            },
        });
        let Ok(ask) = serde_json::to_vec(&ask) else {
            return Approved::default();
        };
        if self.reply(&ask).is_err() {
            return Approved::default();
        }
        let until = Instant::now() + APPROVE_WAIT;
        while Instant::now() < until {
            match self.inner.receive(Some(Duration::from_millis(50))) {
                Ok(Some(message)) => {
                    let value: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
                    if value.get("method").is_none() && value.get("id") == Some(&json!(ask_id)) {
                        let held: Vec<String> = self
                            .hold()
                            .parts
                            .as_ref()
                            .map(|parts| {
                                parts
                                    .approvable
                                    .iter()
                                    .map(|each| each.name.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let result = value.get("result");
                        return Approved {
                            servers: result
                                .and_then(|result| result.get("allow"))
                                .and_then(Value::as_array)
                                .map(|names| {
                                    names
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .filter(|name| held.iter().any(|each| each == name))
                                        .map(str::to_owned)
                                        .collect()
                                })
                                .unwrap_or_default(),
                            // Only an overlay the question listed can be allowed by the answer.
                            hooks: self.connection.hooks.offered(&hooks::names(
                                result.and_then(|result| result.get("allowHooks")),
                            )),
                        };
                    }
                    self.hold().queued.push_back(message);
                }
                Err(Failure::Timeout) => {}
                Ok(None) | Err(_) => return Approved::default(),
            }
        }
        Approved::default()
    }

    /// Records the request, and adds the profile's parts to a session request; `Err` is the answer refusing it.
    fn forward(&self, bytes: Vec<u8>, allowed: Vec<String>) -> Result<Vec<u8>, Vec<u8>> {
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
        add_parts(params, parts).map_err(|why| error(&id, INVALID_PARAMS, &why))?;
        if let Some(servers) = params.get_mut("mcpServers").and_then(Value::as_array_mut) {
            servers.extend(
                parts
                    .approvable
                    .iter()
                    .filter(|each| allowed.contains(&each.name))
                    .map(|each| each.server.clone()),
            );
        }
        hold.pending
            .insert(id.to_string(), Pending::Session(allowed));
        serde_json::to_vec(&value)
            .map_err(|_| error(&id, INTERNAL_ERROR, "the request could not be written"))
    }
}

impl<C: Channel + ?Sized> Hosted<'_, C> {
    /// An answer to `initialize` or a session request gets the report; the first also settles what each session is given.
    fn send_profiled(&self, message: &[u8]) -> Result<(), Failure> {
        // `_canoa/` and the secret extension are the host's own to ask of its client: an agent's request of that name is never
        // passed on, or the client could be made to show the host's questions, or to answer the host's real ones, for the agent.
        let screened;
        let message = match crate::scope::screen(message) {
            crate::scope::Screened::Drop(why, answer) => {
                crate::scope::note_dropped(why, &self.noted);
                // A client waiting for this answer is told it will not come.
                return answer.map_or(Ok(()), |answer| self.inner.send(&answer));
            }
            crate::scope::Screened::Pass(Some(changed)) => {
                screened = changed;
                &screened[..]
            }
            crate::scope::Screened::Pass(None) => message,
        };
        let stamped = self.scope.stamp(message);
        let annotated = self.sites.annotate(&stamped);
        let message = &*annotated;
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
                    let (takes, reported) = described(result);
                    let harness = harness_of(reported, &self.connection.entry);
                    let lookup = |name: &str| std::env::var(name).ok();
                    let parts = parts_for(self.connection, harness, &takes, &lookup);
                    if let Some(result) = value.get_mut("result") {
                        add_report(result, &parts.report);
                    }
                    hold.parts = Some(parts);
                }
            }
            Some(Pending::Session(allowed)) => {
                let key = value.get("id").map(ToString::to_string).unwrap_or_default();
                let notes = hold.book.notes_for(&key);
                if let (Some(result), Some(parts)) = (value.get_mut("result"), &hold.parts) {
                    let mut report = parts.report.clone();
                    mark_allowed(&mut report, &allowed);
                    replace_hook_items(&mut report, &notes);
                    add_report(result, &report);
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
}

impl<C: Channel + ?Sized> Drop for Hosted<'_, C> {
    /// A connection that ends without closing its sessions ends them: best effort, for the hooks that listen.
    fn drop(&mut self) {
        self.end_all();
    }
}

impl<C: Channel + ?Sized> Channel for Hosted<'_, C> {
    /// The agent's message to the client: profiled, and with this host's say on usage when the agent reports none.
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.splice.relay(message, |sent| {
            let later = match self.on_agent(sent) {
                Seen::Dropped => return Ok(()),
                Seen::Passed(later) => later,
            };
            let sent_on = self.send_profiled(sent);
            if let Some(later) = later {
                self.later(later);
            }
            sent_on
        })
    }

    /// The launch line first; then the client's messages, a refused one reading as a timeout as [`crate::scope`] has it.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let first = {
            let mut hold = self.hold();
            if let Some(line) = hold.launch.take() {
                return Ok(Some(line));
            }
            // Canoa's own answers to the agent, made by a hook's denial, go before anything the client sent.
            if let Some(answer) = hold.book.next_injected() {
                return Ok(Some(answer));
            }
            core::mem::replace(&mut hold.first, false)
        };
        self.splice.tick(|sent| self.send_profiled(sent));
        let queued = self.hold().queued.pop_front();
        let message = match queued {
            Some(message) => message,
            None => match self.inner.receive(timeout)? {
                Some(message) => message,
                None => return Ok(None),
            },
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
        // The scope removes every `_meta` of the client's, so its opt-in is read before.
        if let Ok(value) = serde_json::from_slice::<Value>(&message)
            && value.get("method") == Some(&json!("initialize"))
        {
            self.hold().approves = value
                .pointer("/params/_meta")
                .and_then(|meta| meta.get(APPROVE_META))
                == Some(&json!(true));
        }
        let forwarded = match self.sites.check(self.scope, &message) {
            Verdict::Forward(bytes) => {
                let approved = self.approvals(&bytes);
                match self.on_client(bytes, &approved.hooks) {
                    Ok(bytes) => self.forward(bytes, approved.servers),
                    Err(answer) => Err(answer),
                }
            }
            Verdict::Refuse(answer) => Err(answer.unwrap_or_default()),
        };
        match forwarded {
            Ok(bytes) => match self.splice.read_client(&bytes) {
                Client::Answer(answer) => {
                    self.reply(&answer)?;
                    Err(Failure::Timeout)
                }
                Client::Held => Err(Failure::Timeout),
                Client::Forward => Ok(Some(bytes)),
            },
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
        self.end_all();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// The name of the one profile a source of the host's own is offered, built from canoa.cfg's top level.
pub(crate) const LOCAL_PROFILE: &str = "default";

/// An entry's offer, from its profile names and the configuration's `[host.profiles]`.
pub(crate) fn offer(
    names: &[String],
    profiles: &IndexMap<String, HostProfile>,
    local: Option<&HostProfile>,
    launcher: OsString,
) -> Option<Arc<Offer>> {
    if let Some(local) = local {
        return Some(Arc::new(Offer {
            profiles: vec![(LOCAL_PROFILE.to_owned(), local.clone())],
            launcher,
        }));
    }
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
