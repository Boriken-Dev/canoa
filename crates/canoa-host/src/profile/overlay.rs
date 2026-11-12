//! An AOS overlay as a profile layer: what Canoa applies of it, what it reports as not applied yet, and its
//! environment, built when a session starts.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use std::path::PathBuf;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::{
    Change, Context, EnvValue, Layer, LayerId, LayerKind, McpServer, Profile, ProfileError, Skill,
    is_reserved_env,
};
use coqui_aos::{self as aos, Environment, Facts, NonDefaultSetting, OverlayTree, Settings};

/// An AOS overlay as Canoa applies it: what building its environment needs, and what Canoa does not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AosLayer {
    /// The name canoa.cfg gives it, its identity.
    pub identity: String,
    /// Its root, absolute, as processes see it.
    pub root: String,
    /// The OS its environment is built for, as `os` keys name it.
    pub os: String,
    /// Its tree digest.
    pub digest: String,
    /// Whether canoa.cfg pins it by digest.
    pub pinned: bool,
    /// Its manifest as Canoa applies it: the `bin` and `lib` defaults off, and no header sent over plain `http` to
    /// another host.
    pub manifest: Map<String, Value>,
    /// Its files, relative to its root.
    pub files: BTreeSet<String>,
    /// Its env files' bytes, by relative path.
    pub texts: BTreeMap<String, Vec<u8>>,
    /// Each secret reference of its environment: the variable, the reference and whether it is optional.
    pub references: Vec<(String, String, bool)>,
    /// What it declares that Canoa does not apply yet, each as `<field>` or `<field>: <detail>`.
    pub not_applied: Vec<String>,
}

/// What Canoa needs to make a layer of an AOS overlay.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct OverlayInput<'a> {
    /// The name canoa.cfg gives it.
    pub identity: &'a str,
    /// Its root, absolute.
    pub root: &'a str,
    /// The OS its environment is for: `windows`, `linux`, `macos`, ...
    pub os: &'a str,
    /// The AOS harness id of the agent it is for (`claude-code`, `codex`, ...), whose hints apply.
    pub harness: Option<&'a str>,
    /// Whether canoa.cfg pins it by digest; checking the digest is the caller's.
    pub pinned: bool,
    /// Its files and their bytes, as [`aos::read_tree`] reads them or a source's objects hold them.
    pub tree: &'a OverlayTree,
}

impl<'a> OverlayInput<'a> {
    /// The overlay `identity` at `root`, holding `tree`, for `os`.
    #[must_use]
    pub fn new(identity: &'a str, root: &'a str, os: &'a str, tree: &'a OverlayTree) -> Self {
        Self {
            identity,
            root,
            os,
            harness: None,
            pinned: false,
            tree,
        }
    }
}

/// Why an overlay is left out whole. Never quotes a value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayRefusal {
    /// `name`, `digest`, `manifest`, `invalid`, `env-file` or `reserved`.
    pub code: &'static str,
    /// The name, the digest's or the reader's class and path, the pointer, or the variable.
    pub subject: String,
}

impl fmt::Display for OverlayRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.code {
            "name" => "its name is not a valid overlay identity",
            "digest" => "its files cannot be digested",
            "manifest" => "its manifest cannot be read",
            "invalid" => "its manifest is not valid",
            "env-file" => "an env file of it cannot be read",
            _ => "it sets a variable a layer from elsewhere may not set",
        };
        write!(f, "{what} ({})", self.subject)
    }
}

impl core::error::Error for OverlayRefusal {}

fn refused(code: &'static str, subject: impl Into<String>) -> OverlayRefusal {
    OverlayRefusal {
        code,
        subject: subject.into(),
    }
}

/// An AOS overlay's report in the view: what Canoa does not apply, what is not granted, and its non-default settings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayReport {
    /// The layer.
    pub layer: LayerId,
    /// Its tree digest.
    pub digest: String,
    /// What it declares that Canoa does not apply yet.
    pub not_applied: Vec<String>,
    /// Optional references and MCP reads no layer of the person's own shares, which are left unset.
    pub ungranted: Vec<String>,
    /// Required references that cannot be resolved, as `<variable>: <reference>`: no session can start with it.
    pub unresolved: Vec<String>,
    /// Its non-default settings, each time it is activated.
    pub non_defaults: Vec<NonDefaultSetting>,
}

impl OverlayReport {
    /// The report as the view writes it.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "layer": self.layer.to_string(),
            "digest": self.digest,
            "not_applied": self.not_applied,
            "ungranted": self.ungranted,
            "unresolved": self.unresolved,
            "non_defaults": self
                .non_defaults
                .iter()
                .map(|non_default| json!([non_default.id.as_str(), non_default.subject]))
                .collect::<Vec<_>>(),
        })
    }
}

fn reference_of(entry: &Map<String, Value>) -> Option<(String, bool)> {
    let optional = entry.get("optional") == Some(&Value::Bool(true));
    if let Some(name) = entry.get("secret").and_then(Value::as_str) {
        return Some((format!("secret:{name}"), optional));
    }
    let provider = entry.get("provider").and_then(Value::as_str)?;
    let key = entry.get("key").and_then(Value::as_str).unwrap_or("");
    Some((format!("{provider}:{key}"), optional))
}

/// An AOS env value as the view shows it, as written.
fn env_value(value: &Value) -> EnvValue {
    match value {
        Value::String(text) => EnvValue::Text(text.clone()),
        Value::Object(entry) => {
            if let Some(text) = entry.get("value").and_then(Value::as_str) {
                EnvValue::Text(text.to_owned())
            } else if let Some(path) = entry.get("path").and_then(Value::as_str) {
                EnvValue::Path(path.to_owned())
            } else {
                let (reference, optional) = reference_of(entry).unwrap_or_default();
                EnvValue::Reference {
                    reference,
                    optional,
                }
            }
        }
        _ => EnvValue::Text(String::new()),
    }
}

pub(crate) fn resolve_path(root: &str, relative: &str, windows: bool) -> String {
    let separator = if windows { "\\" } else { "/" };
    format!(
        "{}{separator}{}",
        root.trim_end_matches(['/', '\\']),
        relative.replace('/', separator)
    )
}

/// A mapping's entries by key bytes: mapping order carries no meaning in AOS, so every order Canoa shows is this one.
fn sorted(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

fn entries(value: Option<&Value>) -> Vec<(&String, &Value)> {
    value
        .and_then(Value::as_object)
        .map(sorted)
        .unwrap_or_default()
}

const DEFAULT_SCRIPTS: [&str; 7] = [".py", ".js", ".mjs", ".cjs", ".ps1", ".bash", ".sh"];

/// Makes the layer of an AOS overlay: its root variable, environment, skills, MCP servers, context, and the hints for
/// `input.harness`. The `bin` and `lib` defaults, which change `PATH`, `PYTHONPATH` and `NODE_PATH`, are not applied
/// and are reported; so is everything Canoa does not run: env scripts, setup, dependencies, runtime probes and
/// commands. Reads nothing but `input.tree`.
///
/// # Errors
///
/// [`OverlayRefusal`] for a name that is not a valid identity, a tree that cannot be digested, a manifest (`OVERLAY.md`
/// or `overlay.cfg`, never both) that cannot be read or is not valid, an env file that is missing or cannot be read,
/// and a variable Canoa does not let a layer from elsewhere set ([`is_reserved_env`]).
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the manifest, field by field"
)]
pub fn overlay_layer(input: &OverlayInput<'_>) -> Result<Layer, OverlayRefusal> {
    if !aos::valid_overlay_name(input.identity) {
        return Err(refused("name", input.identity));
    }
    let digest = tree_digest(input.tree).map_err(|error| refused("digest", error.to_string()))?;
    let files: BTreeSet<String> = input
        .tree
        .paths()
        .filter(|path| aos::valid_relative_path(path))
        .map(str::to_owned)
        .collect();
    let member = |path: &str| {
        files
            .contains(path)
            .then(|| input.tree.bytes(path))
            .flatten()
    };
    let paths = OverlayTree::from_paths(files.iter().cloned());
    let settings = Settings::default();
    let read = aos::read_overlay(input.tree, &settings)
        .map_err(|error| refused("manifest", reading(&error)))?;
    let manifest = read.manifest;
    let ignored = read.context_ignored;
    // `about` is store metadata: a problem in it never stops an overlay, and Canoa applies nothing of it.
    if let Some(issue) = aos::validate(&manifest, Some(&paths))
        .iter()
        .find(|issue| !aos::applies(core::slice::from_ref(issue)))
    {
        return Err(refused("invalid", format!("{} {}", issue.at, issue.code)));
    }
    let windows = input.os == "windows";
    let mut profile = Profile::default();
    let mut not_applied: Vec<String> = ignored
        .iter()
        .map(|file| format!("context: {file}, ignored while the body of OVERLAY.md is the context"))
        .collect();
    let mut texts = BTreeMap::new();
    let mut references = Vec::new();
    let mut env: IndexMap<String, EnvValue> = IndexMap::new();
    env.insert(
        aos::root_variable(input.identity),
        EnvValue::Value(input.root.to_owned()),
    );
    let mut set = |name: &str, value: EnvValue, env: &mut IndexMap<String, EnvValue>| {
        if let EnvValue::Reference {
            reference,
            optional,
        } = &value
        {
            references.push((name.to_owned(), reference.clone(), *optional));
        }
        env.insert(name.to_owned(), value);
    };
    for file in aos::env_files_of(&manifest, &paths) {
        if !on_os(&file, input.os) {
            continue;
        }
        let path = file.get("path").and_then(Value::as_str).unwrap_or("");
        let bytes =
            member(path).ok_or_else(|| refused("env-file", format!("{path}: missing-file")))?;
        let invalid = |code: &str| refused("env-file", format!("{path}: {code}"));
        if file.get("format").and_then(Value::as_str) == Some("dotenv") {
            for line in aos::parse_dotenv(bytes).map_err(|error| invalid(error.class.as_str()))? {
                let value = if line.literal {
                    EnvValue::Value(line.text)
                } else {
                    EnvValue::Text(line.text)
                };
                set(&line.name, value, &mut env);
            }
        } else {
            let mapping = aos::read_document(bytes, &settings)
                .map_err(|error| invalid(error.class.as_str()))?;
            let wrapped = Map::from_iter([("env".to_owned(), Value::Object(mapping.clone()))]);
            if !aos::validate(&wrapped, None).is_empty() {
                return Err(invalid("invalid-env-file"));
            }
            for (name, value) in sorted(&mapping) {
                set(name, env_value(value), &mut env);
            }
        }
        texts.insert(path.to_owned(), bytes.to_vec());
    }
    for (name, value) in entries(manifest.get("env")) {
        set(name, env_value(value), &mut env);
    }
    let mut applied = manifest.clone();
    let defaults = aos::default_paths(&manifest, &paths);
    for addition in &defaults {
        not_applied.push(format!(
            "paths: {}/ on {}, which a layer from elsewhere may not change",
            text_of(addition, "path"),
            text_of(addition, "variable")
        ));
    }
    if !defaults.is_empty() {
        let table = applied
            .entry("defaults")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(table) = table {
            table.insert("bin".to_owned(), Value::Bool(false));
            table.insert("lib".to_owned(), Value::Bool(false));
        }
    }
    let mut lists: IndexMap<String, Vec<(String, bool)>> = IndexMap::new();
    for addition in aos::paths_of(&applied, &paths) {
        if on_os(&addition, input.os) {
            lists
                .entry(text_of(&addition, "variable").to_owned())
                .or_default()
                .push((
                    text_of(&addition, "path").to_owned(),
                    text_of(&addition, "position") == "append",
                ));
        }
    }
    for (variable, paths) in lists {
        set(&variable, EnvValue::Paths(paths), &mut env);
    }
    if let Some(name) = env.keys().skip(1).find(|name| is_reserved_env(name)) {
        return Err(refused("reserved", name.clone()));
    }
    profile.env = env
        .into_iter()
        .map(|(name, value)| (name, Change::Set(value)))
        .collect();

    for name in aos::skills_of(&manifest, &paths) {
        profile.skills.insert(
            name.clone(),
            Change::Set(Skill::new(format!("skills/{name}"))),
        );
    }

    let insecure: BTreeSet<String> = match probe(
        &Probed {
            manifest: &applied,
            files: &files,
            texts: &texts,
            identity: input.identity,
            root: input.root,
            os: input.os,
            pinned: input.pinned,
        },
        &[],
    ) {
        Ok(found) => found
            .non_defaults
            .into_iter()
            .filter(|setting| setting.id == aos::NonDefaultId::InsecureTransport)
            .map(|setting| setting.subject)
            .collect(),
        // Without a report, every plain `http` server counts as one to another host.
        Err(_) => servers_of(&applied)
            .filter(|server| {
                text_of(server, "url")
                    .to_ascii_lowercase()
                    .starts_with("http://")
            })
            .map(|server| text_of(server, "name").to_owned())
            .collect(),
    };
    let servers = applied
        .get_mut("mcp")
        .and_then(|mcp| mcp.get_mut("servers"))
        .and_then(Value::as_array_mut);
    for server in servers.into_iter().flatten() {
        let Some(server) = server.as_object_mut() else {
            continue;
        };
        let name = server
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if server.contains_key("timeout") {
            not_applied.push(format!("mcp.{name}.timeout"));
        }
        let mapped = if server.get("transport").and_then(Value::as_str) == Some("stdio") {
            let command = server.get("command").and_then(Value::as_str).unwrap_or("");
            let command = if command.contains(['/', '\\']) {
                resolve_path(input.root, command, windows)
            } else {
                command.to_owned()
            };
            let args = server
                .get("args")
                .and_then(Value::as_array)
                .map(|args| {
                    args.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let mut variables = IndexMap::new();
            for (variable, value) in entries(server.get("env")) {
                variables.insert(variable.clone(), env_value(value));
            }
            for variable in server
                .get("env_passthrough")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let variable = variable.as_str().unwrap_or("");
                variables.insert(
                    variable.to_owned(),
                    EnvValue::Reference {
                        reference: format!("env:{variable}"),
                        optional: true,
                    },
                );
            }
            if let Some(variable) = variables.keys().find(|variable| is_reserved_env(variable)) {
                return Err(refused("reserved", variable.clone()));
            }
            not_applied.push(format!(
                "mcp.{name}: it runs in the overlay's root, which Canoa does not set yet"
            ));
            McpServer::Stdio {
                command,
                args,
                env: variables,
            }
        } else {
            let url = server
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut headers = IndexMap::new();
            let plain = insecure.contains(&name);
            for (header, variable) in entries(server.get("headers_env")) {
                if plain {
                    not_applied.push(format!(
                        "mcp.{name}.headers_env.{header}: never sent over plain http to another host"
                    ));
                    continue;
                }
                headers.insert(
                    header.clone(),
                    EnvValue::Reference {
                        reference: format!("env:{}", variable.as_str().unwrap_or("")),
                        optional: true,
                    },
                );
            }
            if plain {
                server.remove("headers_env");
            }
            McpServer::Http { url, headers }
        };
        profile.mcp.insert(name, Change::Set(mapped));
    }
    for link in manifest
        .get("mcp")
        .and_then(|mcp| mcp.get("skills"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        not_applied.push(format!(
            "mcp.skills.{}",
            link.get("name").and_then(Value::as_str).unwrap_or("")
        ));
    }

    profile.contexts.extend(
        aos::rules_of(&manifest, &paths)
            .into_iter()
            .map(Context::File),
    );
    let routing: Vec<&str> = manifest
        .get("routing")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|line| !line.trim().is_empty())
        .collect();
    if !routing.is_empty() {
        profile.contexts.push(Context::Text(routing.join("\n")));
    }
    profile.contexts.extend(
        aos::context_of(&manifest, input.tree)
            .iter()
            .filter_map(|path| context_entry(input.tree, path, &settings)),
    );
    for (id, hints) in entries(manifest.get("harness")) {
        if input.harness != Some(id.as_str()) {
            not_applied.push(format!("harness.{id}: hints for another harness"));
            continue;
        }
        for (key, value) in entries(Some(hints)) {
            match (key.as_str(), value) {
                ("model" | "effort", Value::String(text)) if !super::plain(text) => {
                    not_applied.push(format!("harness.{id}.{key}: not one line of plain text"));
                }
                ("model", Value::String(model)) => profile.model = Some(model.clone()),
                ("effort", Value::String(effort)) => profile.effort = Some(effort.clone()),
                ("context", Value::Array(paths)) => profile.contexts.extend(
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(|path| context_entry(input.tree, path, &settings)),
                ),
                ("skills", Value::Bool(true)) => {}
                ("skills", _) => {
                    not_applied.push(format!("harness.{id}.skills: delivery by routing line"))
                }
                _ => not_applied.push(format!("harness.{id}.{key}: the harness's own setting")),
            }
        }
    }

    let on = |key: &str| {
        manifest
            .get("defaults")
            .and_then(|defaults| defaults.get(key))
            != Some(&Value::Bool(false))
    };
    for (field, stem) in [("env_scripts", "env"), ("setup", "setup")] {
        if on(field) {
            for extension in DEFAULT_SCRIPTS {
                let path = format!("{stem}{extension}");
                if files.contains(&path) {
                    not_applied.push(format!("{field}: {path}"));
                }
            }
        }
        for entry in manifest
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
            if !not_applied.contains(&format!("{field}: {path}")) {
                not_applied.push(format!("{field}: {path}"));
            }
        }
    }
    if on("deps") {
        for path in ["lib/python/requirements.txt", "lib/node/package.json"] {
            if files.contains(path) {
                not_applied.push(format!("dependencies: {path}"));
            }
        }
    }
    for (key, _) in entries(manifest.get("runtime")) {
        not_applied.push(format!("runtime.{key}"));
    }
    for (name, target) in entries(manifest.get("scripts")) {
        if *target != Value::Bool(false) {
            not_applied.push(format!("scripts.{name}"));
        }
    }

    let mut layer = Layer::new(
        LayerId::new(LayerKind::Overlay, input.identity),
        Some(PathBuf::from(input.root)),
        profile,
    );
    layer.aos = Some(Box::new(AosLayer {
        identity: input.identity.to_owned(),
        root: input.root.to_owned(),
        os: input.os.to_owned(),
        digest: digest.digest.to_string(),
        pinned: input.pinned,
        manifest: applied,
        files,
        texts,
        references,
        not_applied,
    }));
    Ok(layer)
}

pub(crate) fn granted(shared: &IndexMap<String, LayerId>, name: &str, windows: bool) -> bool {
    shared
        .keys()
        .any(|shared| shared == name || windows && shared.eq_ignore_ascii_case(name))
}

/// A reader's refusal as its class and line: the parsers disagree on columns, and the text of a document is never quoted.
fn reading(error: &aos::Error) -> String {
    let class = error.class.as_str();
    error.line.map_or_else(
        || class.to_owned(),
        |line| format!("{class} at line {line}"),
    )
}

/// A context entry as the profile holds it: a file, or for `OVERLAY.md` its body as text, since the front matter is the
/// manifest and no context. An empty body is no context.
fn context_entry(tree: &OverlayTree, path: &str, settings: &Settings) -> Option<Context> {
    if path != "OVERLAY.md" {
        return Some(Context::File(path.to_owned()));
    }
    let body = aos::context_file_text(tree.bytes(path)?, settings).ok()?;
    let text = core::str::from_utf8(body).ok()?;
    (!text.trim().is_empty()).then(|| Context::Text(text.to_owned()))
}

/// A text field of a mapping, empty when it is absent or not text.
fn text_of<'a>(entry: &'a Value, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Whether a path addition or env file applies on `os`: its `os` list names it, or it has none.
fn on_os(entry: &Value, os: &str) -> bool {
    let list: Option<Vec<String>> = entry.get("os").map(|names| match names {
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(name) => vec![name.clone()],
        _ => Vec::new(),
    });
    aos::os_matches(list.as_deref(), os)
}

/// The entries of `mcp.servers`.
fn servers_of(manifest: &Map<String, Value>) -> impl Iterator<Item = &Value> {
    manifest
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The digest of a tree whose every file's bytes it holds.
fn tree_digest(tree: &OverlayTree) -> Result<aos::TreeDigest, aos::Error> {
    let mut entries = Vec::new();
    for path in tree.paths() {
        let bytes = tree
            .bytes(path)
            .ok_or_else(|| aos::Error::new(aos::Class::Io, path))?;
        entries.push((path, aos::TreeEntry::File(bytes.to_vec())));
    }
    aos::digest_entries(
        entries
            .iter()
            .map(|(path, entry)| (path.split('/').map(str::as_bytes).collect(), entry)),
    )
}

/// What `coqui_aos::effective` says of an overlay for the view. The manifest is read less the references it must
/// have and nothing shares (the view lists those as unresolved, and they would fail the call). `shared` are the
/// variables the person's own layers share, each granted and read as empty text.
/// The parts of an overlay a probe reads.
struct Probed<'a> {
    manifest: &'a Map<String, Value>,
    files: &'a BTreeSet<String>,
    texts: &'a BTreeMap<String, Vec<u8>>,
    identity: &'a str,
    root: &'a str,
    os: &'a str,
    pinned: bool,
}

fn probe(overlay: &Probed<'_>, shared: &[String]) -> Result<aos::Effective, aos::Error> {
    let Probed {
        manifest,
        files,
        texts,
        identity,
        root,
        os,
        pinned,
    } = *overlay;
    let mut manifest = manifest.clone();
    if let Some(Value::Object(env)) = manifest.get_mut("env") {
        env.retain(|_, entry| {
            let Some(entry) = entry.as_object() else {
                return true;
            };
            let reference = entry.contains_key("provider") || entry.contains_key("secret");
            let optional = entry.get("optional") == Some(&Value::Bool(true));
            let is_shared = entry.get("provider").and_then(Value::as_str) == Some("env")
                && entry
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| shared.iter().any(|name| name == key));
            !reference || optional || is_shared
        });
    }
    let mut facts = Facts::new(os);
    facts.root = root.to_owned();
    facts.identity = identity.to_owned();
    facts.pinned = pinned;
    facts.grants.insert("env".to_owned(), shared.to_vec());
    facts.host_env = shared
        .iter()
        .map(|name| (name.clone(), String::new()))
        .collect();
    facts.base_env = facts.host_env.clone();
    facts.tree = texts.iter().fold(
        OverlayTree::from_paths(files.iter().cloned()),
        |tree, (path, bytes)| tree.with_bytes(path, bytes.clone()),
    );
    aos::effective(&manifest, &facts)
}

/// An overlay's report, with what the person's own layers share as its grants.
pub(crate) fn report(
    layer: &LayerId,
    aos: &AosLayer,
    shared: &IndexMap<String, LayerId>,
) -> OverlayReport {
    let windows = aos.os == "windows";
    let names: Vec<String> = shared.keys().cloned().collect();
    // An overlay whose environment cannot be built starts no session; its view then lists no grants.
    let (ungranted, non_defaults) = probe(
        &Probed {
            manifest: &aos.manifest,
            files: &aos.files,
            texts: &aos.texts,
            identity: &aos.identity,
            root: &aos.root,
            os: &aos.os,
            pinned: aos.pinned,
        },
        &names,
    )
    .map(|found| (found.ungranted, found.non_defaults))
    .unwrap_or_default();
    // Canoa runs no script, setup or install, so what only those would make non-default is not Canoa's to report.
    let non_defaults = non_defaults
        .into_iter()
        .filter(|setting| {
            matches!(
                setting.id,
                aos::NonDefaultId::Unpinned
                    | aos::NonDefaultId::EnvPassthrough
                    | aos::NonDefaultId::SecretRead
                    | aos::NonDefaultId::EnvSensitive
                    | aos::NonDefaultId::InsecureTransport
            )
        })
        .collect();
    let unresolved = aos
        .references
        .iter()
        .filter(|(_, reference, optional)| {
            !optional
                && !reference
                    .strip_prefix("env:")
                    .is_some_and(|name| granted(shared, name, windows))
        })
        .map(|(variable, reference, _)| format!("{variable}: {reference}"))
        .collect();
    OverlayReport {
        layer: layer.clone(),
        digest: aos.digest.clone(),
        not_applied: aos.not_applied.clone(),
        ungranted,
        unresolved,
        non_defaults,
    }
}

fn reason(class: aos::Class) -> &'static str {
    match class {
        aos::Class::UndefinedVariable => {
            "an overlay's placeholder names a variable that is not set"
        }
        aos::Class::SecretPlaceholder => {
            "an overlay's placeholder reads a secret, or a variable of this computer none of your layers shares"
        }
        aos::Class::UnresolvedReference => {
            "an overlay reads a variable none of your layers shares, or one that is not set, or a secret Canoa does not hold"
        }
        aos::Class::InvalidPattern => "an overlay's placeholder has a pattern that is not valid",
        aos::Class::InvalidReference => {
            "an overlay names a secret file that is not a relative path"
        }
        aos::Class::MissingFile | aos::Class::InvalidEnvFile => {
            "an overlay's env file cannot be read"
        }
        aos::Class::EnvPathConflict => {
            "an overlay sets a variable both by env and by a path addition"
        }
        aos::Class::ReservedVariable => "an overlay sets a variable the consumer sets",
        aos::Class::HookRefused => "a hook of an overlay that guards its session failed",
        _ => "an overlay's environment cannot be built",
    }
}

/// An environment built from the overlays, and the variables of this process it started from that no layer of the
/// person's shares.
#[derive(Debug)]
pub(crate) struct Built {
    env: Environment,
    base: BTreeMap<String, String>,
    /// What the `pre-env` hooks' runs reported.
    pub(crate) hook_reports: Vec<aos::HookReport>,
}

impl Built {
    /// A variable an overlay set, or one the person shares; none for one only this process holds.
    pub(crate) fn owned(&self, name: &str) -> Option<&str> {
        let value = self.env.get(name)?;
        (self.base.get(name).map(String::as_str) != Some(value)).then_some(value)
    }

    /// What a placeholder reads, as the environment answers one.
    pub(crate) fn lookup(&self, name: &str) -> Result<Option<String>, aos::Error> {
        if self.env.is_secret(name) {
            return Err(aos::Error::new(aos::Class::SecretPlaceholder, name));
        }
        Ok(self.env.get(name).map(str::to_owned))
    }
}

/// Every word of a document's text: the names of the variables it can read or set.
fn words(value: &Value, out: &mut BTreeSet<String>) {
    fn add(text: &str, out: &mut BTreeSet<String>) {
        out.extend(
            text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|word| !word.is_empty() && !word.starts_with(|c: char| c.is_ascii_digit()))
                .map(str::to_owned),
        );
    }
    match value {
        Value::String(text) => add(text, out),
        Value::Array(items) => {
            for item in items {
                words(item, out);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                add(key, out);
                words(item, out);
            }
        }
        _ => {}
    }
}

/// Builds the AOS overlays' environment after each overlay in turn, lowest first, over this process's environment as
/// `lookup` reads it. A placeholder reads only what the overlays set and the variables the person's own layers share;
/// any other variable of this process is seen by a path addition and `if_unset`, never by a placeholder. Only the
/// variables an overlay's text names are read from the process.
pub(crate) fn build_each(
    layers: &[AosLayer],
    shared: &IndexMap<String, LayerId>,
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
) -> Vec<Result<Built, ProfileError>> {
    let windows = layers.first().is_some_and(|first| first.os == "windows");
    let mut named = BTreeSet::new();
    for layer in layers {
        words(&Value::Object(layer.manifest.clone()), &mut named);
        for text in layer.texts.values() {
            let text = String::from_utf8_lossy(text).into_owned();
            words(&Value::String(text), &mut named);
        }
    }
    let mut base = BTreeMap::new();
    let mut process = Environment::new(Vec::new(), windows);
    for name in named {
        if let Some(value) = lookup(&name) {
            let unshared = !granted(shared, &name, windows);
            process.set(&name, value.clone(), unshared);
            if unshared {
                base.insert(name, value);
            }
        }
    }
    let names: Vec<String> = shared.keys().cloned().collect();
    let host_env: BTreeMap<String, String> = names
        .iter()
        .filter_map(|name| lookup(name).map(|value| (name.clone(), value)))
        .collect();
    let active: Vec<(&Map<String, Value>, Facts)> = layers
        .iter()
        .map(|layer| {
            let mut facts = Facts::new(layer.os.clone());
            facts.root = layer.root.clone();
            facts.identity = layer.identity.clone();
            facts.pinned = layer.pinned;
            facts.grants.insert("env".to_owned(), names.clone());
            facts.host_env = host_env.clone();
            let paths = OverlayTree::from_paths(layer.files.iter().cloned());
            facts.tree = layer.texts.iter().fold(paths, |tree, (path, bytes)| {
                tree.with_bytes(path, bytes.clone())
            });
            (&layer.manifest, facts)
        })
        .collect();
    (0..layers.len())
        .map(|last| {
            let composed = aos::composed_environment(
                &active[..=last],
                process.clone(),
                pre_env,
                &mut Vec::new(),
            );
            match composed.env {
                Ok(env) => Ok(Built {
                    env,
                    base: base.clone(),
                    hook_reports: composed.hook_reports,
                }),
                Err(error) => Err(ProfileError::Resolve {
                    key: format!("overlay:{}.env.{}", layers[last].identity, error.subject),
                    what: reason(error.class),
                }),
            }
        })
        .collect()
}

/// The environment after every overlay: what a session starts with.
pub(crate) fn build(
    layers: &[AosLayer],
    shared: &IndexMap<String, LayerId>,
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
) -> Result<Built, ProfileError> {
    match build_each(layers, shared, lookup, pre_env).pop() {
        Some(built) => built,
        None => Ok(Built {
            env: Environment::new(Vec::new(), cfg!(windows)),
            base: BTreeMap::new(),
            hook_reports: Vec::new(),
        }),
    }
}
