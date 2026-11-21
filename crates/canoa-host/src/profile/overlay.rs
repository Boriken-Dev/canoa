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
    /// Its manifest.
    pub manifest: Map<String, Value>,
    /// Its files, relative to its root.
    pub files: BTreeSet<String>,
    /// Its env files' bytes, by relative path.
    pub texts: BTreeMap<String, Vec<u8>>,
    /// What it declares that Canoa does not apply yet, each as `<field>` or `<field>: <detail>`.
    pub not_applied: Vec<String>,
    /// The skills it names that it does not bundle: by source, or by a name for a catalog to resolve. Canoa fetches and
    /// resolves nothing itself; what the app fetched for them is read when a session starts.
    pub elsewhere: Vec<aos::SkillEntry>,
    /// Links `(skill_name, server_name)` from `[[mcp.skills]]` pairing a skill with an MCP server.
    pub mcp_skills: Vec<(String, String)>,
    /// The digest canoa.cfg allows its hooks and env scripts at, when it names one.
    pub allowed: Option<String>,
    /// Each runtime's packages folder (`python`, `node`) when the overlay was prepared at this digest: what goes on `PYTHONPATH`
    /// and `NODE_PATH` after its own `lib/`.
    pub packages: BTreeMap<String, String>,
    /// The overlay's state directory, once it was prepared at this digest.
    pub state: Option<String>,
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
    /// `name`, `digest`, `manifest`, `invalid`, `env-file`, `path` or `reserved`.
    pub code: &'static str,
    /// The name, the digest's or the reader's class and path, the pointer, the path that leaves the overlay, or the
    /// variable.
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
            "path" => "a directory it adds to a variable is not inside its own folder",
            _ => "it sets a variable the AOS specification reserves for the host",
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

/// An AOS overlay's report in the view: what Canoa does not apply, and its non-default settings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayReport {
    /// The layer.
    pub layer: LayerId,
    /// Its tree digest.
    pub digest: String,
    /// What it declares that Canoa does not apply yet.
    pub not_applied: Vec<String>,
    /// Its non-default settings, each time it is activated.
    pub non_defaults: Vec<NonDefaultSetting>,
    /// What the specification reports of it besides, as `(id, subject)`: `env-script-not-allowed`. Not in the view.
    pub reports: Vec<(String, String)>,
}

impl OverlayReport {
    /// The report as the view writes it.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "layer": self.layer.to_string(),
            "digest": self.digest,
            "not_applied": self.not_applied,
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

/// Makes the layer of an AOS overlay: its root variable, environment, skills, MCP servers, context, and the hints for
/// `input.harness`. The overlay's own `bin/` and `lib/` go first on `PATH`, `PYTHONPATH` and `NODE_PATH`, as the
/// specification's defaults and `paths` say. What it needs of the computer
/// (`runtime`), its dependencies, setup and commands are the host's to check and report when a session starts. Reads nothing
/// but `input.tree`.
///
/// # Errors
///
/// [`OverlayRefusal`] for a name that is not a valid identity, a tree that cannot be digested, a manifest (`OVERLAY.md`
/// or `overlay.cfg`, never both) that cannot be read or is not valid, an env file that is missing or cannot be read,
/// a path addition that leaves the overlay's own folder, and a variable the specification reserves for the consumer
/// (`AOS_*`, `*_OVERLAY_ROOT`, `*_OVERLAY_STATE`). Every other variable is applied; the report lists the sensitive ones.
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
    let mut env: IndexMap<String, EnvValue> = IndexMap::new();
    env.insert(
        aos::root_variable(input.identity),
        EnvValue::Value(input.root.to_owned()),
    );
    let set = |name: &str, value: EnvValue, env: &mut IndexMap<String, EnvValue>| {
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
    let applied = manifest.clone();
    let mut lists: IndexMap<String, Vec<(String, bool)>> = IndexMap::new();
    for addition in aos::paths_of(&applied, &paths) {
        if on_os(&addition, input.os) {
            let path = text_of(&addition, "path");
            // A directory is the overlay's own: a path that leaves its installed tree is refused, whoever wrote it.
            if !aos::valid_relative_path(path) {
                return Err(refused("path", format!("paths: {path}")));
            }
            lists
                .entry(text_of(&addition, "variable").to_owned())
                .or_default()
                .push((path.to_owned(), text_of(&addition, "position") == "append"));
        }
    }
    for (variable, paths) in lists {
        set(&variable, EnvValue::Paths(paths), &mut env);
    }
    // The names the specification reserves for the consumer are the only ones refused.
    if let Some(name) = env
        .keys()
        .skip(1)
        .find(|name| aos::is_reserved_variable(name))
    {
        return Err(refused("reserved", name.clone()));
    }
    profile.env = env
        .into_iter()
        .map(|(name, value)| (name, Change::Set(value)))
        .collect();

    // Only a bundled skill has a directory here; the others are the app's to fetch and are carried by origin.
    let mut elsewhere = Vec::new();
    for entry in aos::skill_entries(&manifest, &paths) {
        if entry.origin == aos::SkillOrigin::Bundled {
            profile.skills.insert(
                entry.name.clone(),
                Change::Set(Skill::new(format!("skills/{}", entry.name))),
            );
        } else {
            elsewhere.push(entry);
        }
    }

    for server in servers_of(&applied) {
        if !server.is_object() {
            continue;
        }
        let name = server
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if server.get("timeout").is_some() {
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
            if let Some(variable) = variables
                .keys()
                .find(|variable| aos::is_reserved_variable(variable))
            {
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
            for (header, variable) in entries(server.get("headers_env")) {
                headers.insert(
                    header.clone(),
                    EnvValue::Reference {
                        reference: format!("env:{}", variable.as_str().unwrap_or("")),
                        optional: true,
                    },
                );
            }
            McpServer::Http { url, headers }
        };
        profile.mcp.insert(name, Change::Set(mapped));
    }
    let mut mcp_skills = Vec::new();
    for link in manifest
        .get("mcp")
        .and_then(|mcp| mcp.get("skills"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(name), Some(server)) = (
            link.get("name").and_then(Value::as_str),
            link.get("server").and_then(Value::as_str),
        ) {
            mcp_skills.push((name.to_owned(), server.to_owned()));
        } else {
            not_applied.push(format!(
                "mcp.skills.{}",
                link.get("name").and_then(Value::as_str).unwrap_or("")
            ));
        }
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
                ("delegate", _) => {
                    // Suggestions only: the host applies each where its own settings allow.
                    if let Some(hint) = aos::harness_delegate_of(&manifest, id) {
                        profile.delegate = super::Delegate {
                            default: hint.default,
                            sources: hint.sources,
                            max_running: hint.max_running,
                        };
                    }
                }
                ("skills", Value::Bool(true)) => {}
                ("skills", _) => {
                    not_applied.push(format!("harness.{id}.skills: delivery by routing line"))
                }
                _ => not_applied.push(format!("harness.{id}.{key}: the harness's own setting")),
            }
        }
    }

    // Setup, dependencies and the commands `scripts` expose are the host's to prepare and report when a session starts, and so
    // are the env scripts, which run where the environment is built.
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
        not_applied,
        elsewhere,
        mcp_skills,
        allowed: None,
        packages: BTreeMap::new(),
        state: None,
    }));
    Ok(layer)
}

/// A reader's refusal as its class and line: the parsers disagree on columns, and the text of a document is never quoted.
fn reading(error: &aos::Error) -> String {
    let class = error.class.as_str();
    error.line.map_or_else(
        || class.to_owned(),
        |line| format!("{class} at line {line}"),
    )
}

/// A context entry as the profile holds it: the file, templated when the context is assembled, or for `OVERLAY.md` its
/// body as text, since the front matter is the manifest and no context. An empty body is no context.
fn context_entry(tree: &OverlayTree, path: &str, settings: &Settings) -> Option<Context> {
    if path != "OVERLAY.md" {
        return Some(Context::Template(path.to_owned()));
    }
    let body = aos::context_file_text(tree.bytes(path)?, settings).ok()?;
    let text = core::str::from_utf8(body).ok()?;
    (!text.trim().is_empty()).then(|| Context::TemplateBody(text.to_owned()))
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

/// What `coqui_aos::effective` says of an overlay for the view. Every variable its text names is the host's own and
/// granted, read as empty text: only the report is made from it, and a session reads the real values.
/// The parts of an overlay a probe reads.
struct Probed<'a> {
    manifest: &'a Map<String, Value>,
    files: &'a BTreeSet<String>,
    texts: &'a BTreeMap<String, Vec<u8>>,
    identity: &'a str,
    root: &'a str,
    os: &'a str,
    pinned: bool,
    /// Whether env scripts run only at the digest the person allowed: `allowed` is that digest, and `digest` the overlay's.
    gate: Option<(&'a str, Option<&'a str>)>,
}

fn probe(overlay: &Probed<'_>) -> Result<aos::Effective, aos::Error> {
    let Probed {
        manifest,
        files,
        texts,
        identity,
        root,
        os,
        pinned,
        gate,
    } = *overlay;
    let mut named = BTreeSet::new();
    words(&Value::Object(manifest.clone()), &mut named);
    for text in texts.values() {
        words(
            &Value::String(String::from_utf8_lossy(text).into_owned()),
            &mut named,
        );
    }
    let shared: Vec<String> = named.into_iter().collect();
    let mut facts = Facts::new(os);
    facts.root = root.to_owned();
    facts.identity = identity.to_owned();
    facts.pinned = pinned;
    facts.grants.insert("env".to_owned(), shared.clone());
    if let Some((digest, allowed)) = gate {
        facts.gate_env_scripts = true;
        facts.digest = Some(digest.to_owned());
        facts.allowed = allowed.map(str::to_owned);
    }
    facts.host_env = shared
        .iter()
        .map(|name| (name.clone(), String::new()))
        .collect();
    facts.base_env = facts.host_env.clone();
    facts.tree = texts.iter().fold(
        OverlayTree::from_paths(files.iter().cloned()),
        |tree, (path, bytes)| tree.with_bytes(path, bytes.clone()),
    );
    aos::effective(manifest, &facts)
}

/// The folders an overlay adds to `PATH` on its OS, absolute, each with whether it goes after the host's own entries: its `bin/`
/// by default, and what its `paths` name. Where a program the overlay's text names (an MCP server's command) is looked for besides
/// the host's `PATH`.
pub(crate) fn path_additions(aos: &AosLayer) -> Vec<(PathBuf, bool)> {
    let tree = OverlayTree::from_paths(aos.files.iter().cloned());
    aos::paths_of(&aos.manifest, &tree)
        .iter()
        .filter(|addition| {
            on_os(addition, &aos.os)
                && text_of(addition, "variable").eq_ignore_ascii_case("PATH")
                && aos::valid_relative_path(text_of(addition, "path"))
        })
        .map(|addition| {
            (
                PathBuf::from(resolve_path(
                    &aos.root,
                    text_of(addition, "path"),
                    aos.os == "windows",
                )),
                text_of(addition, "position") == "append",
            )
        })
        .collect()
}

/// What the specification's functions say an overlay needs and has to prepare, on its OS.
pub(crate) struct Inspected {
    /// What it needs of the computer it runs on.
    pub(crate) required: super::needs::Required,
    /// The dependency files it installs.
    pub(crate) dependencies: Vec<aos::Dependency>,
    /// The setup it runs, if any.
    pub(crate) setup: Option<aos::SetupRun>,
    /// The commands its `scripts` expose.
    pub(crate) commands: Vec<super::commands::Declared>,
}

/// What an overlay needs of the computer it runs on (its `runtime`, and the interpreters and commands the rest of its manifest
/// names), and what preparing it runs. None when its environment cannot be built, since no session would start with it.
pub(crate) fn inspect(aos: &AosLayer) -> Option<Inspected> {
    let found = probe(&Probed {
        manifest: &aos.manifest,
        files: &aos.files,
        texts: &aos.texts,
        identity: &aos.identity,
        root: &aos.root,
        os: &aos.os,
        pinned: aos.pinned,
        gate: Some((&aos.digest, aos.allowed.as_deref())),
    })
    .ok()?;
    let version = |variant: &Option<Map<String, Value>>| {
        variant
            .as_ref()
            .and_then(|variant| variant.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let asks = |name: &str| found.needs.interpreters.iter().any(|each| each == name);
    let other_os = (!found.supported).then(|| {
        aos.manifest
            .get("runtime")
            .and_then(|runtime| runtime.get("os"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|each| each.as_str().map(str::to_owned))
            .collect()
    });
    let required = super::needs::Required {
        other_os,
        python: (found.python.is_some() || asks("python")).then(|| version(&found.python)),
        node: (found.node.is_some() || asks("node")).then(|| version(&found.node)),
        interpreters: found
            .needs
            .interpreters
            .iter()
            .filter(|each| !matches!(each.as_str(), "python" | "node"))
            .cloned()
            .collect(),
        commands: found.needs.commands.clone(),
    };
    Some(Inspected {
        required,
        dependencies: found.dependencies.clone(),
        setup: found.setup.clone(),
        commands: super::commands::declared(&aos.manifest, &aos.os),
    })
}

/// An overlay's report.
pub(crate) fn report(layer: &LayerId, aos: &AosLayer) -> OverlayReport {
    // An overlay whose environment cannot be built starts no session; its view then lists no settings.
    let (non_defaults, reports) = probe(&Probed {
        manifest: &aos.manifest,
        files: &aos.files,
        texts: &aos.texts,
        identity: &aos.identity,
        root: &aos.root,
        os: &aos.os,
        pinned: aos.pinned,
        gate: Some((&aos.digest, aos.allowed.as_deref())),
    })
    .map(|found| (found.non_defaults, found.reports))
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
    OverlayReport {
        layer: layer.clone(),
        digest: aos.digest.clone(),
        not_applied: aos.not_applied.clone(),
        non_defaults,
        reports,
    }
}

fn reason(class: aos::Class) -> &'static str {
    match class {
        aos::Class::UndefinedVariable => {
            "an overlay's placeholder names a variable that is not set"
        }
        aos::Class::SecretPlaceholder => "an overlay's placeholder reads a secret",
        aos::Class::UnresolvedReference => {
            "an overlay reads a variable that is not set, or a secret Canoa does not hold"
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

/// An environment built from the overlays, and the variables of this process it started from.
#[derive(Debug)]
pub(crate) struct Built {
    env: Environment,
    base: BTreeMap<String, String>,
    /// The same composition over a base whose every value carries [`MARK`]: a variable still holding only a marked value
    /// was left as this process gave it, whatever the overlays would have written.
    traced: Option<Environment>,
    /// What the `pre-env` hooks' runs reported.
    pub(crate) hook_reports: Vec<aos::HookReport>,
}

impl Built {
    /// A variable an overlay set, even to the value this process holds; none for one only this process holds.
    pub(crate) fn owned(&self, name: &str) -> Option<&str> {
        let value = self.env.get(name)?;
        let Some(held) = self.base.get(name) else {
            return Some(value);
        };
        if held != value {
            return Some(value);
        }
        let traced = self.traced.as_ref()?;
        (traced.get(name) != Some(format!("{MARK}{value}").as_str())).then_some(value)
    }

    /// What a placeholder reads, as the environment answers one.
    pub(crate) fn lookup(&self, name: &str) -> Result<Option<String>, aos::Error> {
        if self.env.is_secret(name) {
            return Err(aos::Error::new(aos::Class::SecretPlaceholder, name));
        }
        Ok(self.env.get(name).map(str::to_owned))
    }
}

/// A character no environment value of this process starts with in practice: a Unicode noncharacter.
const MARK: char = '\u{FFFF}';

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

/// The variable names an overlay's text can read or set, and its path additions' variables: the default `bin/` and `lib/`
/// additions are in no text, yet they extend this process's `PATH`, `PYTHONPATH` and `NODE_PATH`, and without them read,
/// each would start empty and replace the process's own.
fn names_of(layer: &AosLayer) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    words(&Value::Object(layer.manifest.clone()), &mut named);
    for text in layer.texts.values() {
        words(
            &Value::String(String::from_utf8_lossy(text).into_owned()),
            &mut named,
        );
    }
    let paths = OverlayTree::from_paths(layer.files.iter().cloned());
    named.extend(
        aos::paths_of(&layer.manifest, &paths)
            .iter()
            .map(|addition| text_of(addition, "variable").to_owned()),
    );
    named
}

/// What each overlay's facts are to the specification's functions: its root, the digest canoa.cfg allowed its hooks and
/// env scripts at, its files, the interpreters the host can run (`interpreters`; none is every one the specification names)
/// and the variables its text names, each granted.
pub(crate) fn active_facts<'a>(
    layers: &'a [AosLayer],
    lookup: &dyn Fn(&str) -> Option<String>,
    interpreters: Option<&[String]>,
) -> Vec<(&'a Map<String, Value>, Facts)> {
    layers
        .iter()
        .map(|layer| {
            // Every variable the overlay's text names is this process's own to read.
            let host_env: BTreeMap<String, String> = names_of(layer)
                .into_iter()
                .filter_map(|name| lookup(&name).map(|value| (name, value)))
                .collect();
            let names: Vec<String> = host_env.keys().cloned().collect();
            let mut facts = Facts::new(layer.os.clone());
            facts.root = layer.root.clone();
            facts.identity = layer.identity.clone();
            facts.pinned = layer.pinned;
            facts.gate_env_scripts = true;
            facts.digest = Some(layer.digest.clone());
            facts.allowed.clone_from(&layer.allowed);
            facts.interpreters = interpreters.map(<[String]>::to_vec);
            facts.packages.clone_from(&layer.packages);
            facts.state.clone_from(&layer.state);
            facts.grants.insert("env".to_owned(), names.clone());
            facts.host_env = host_env.clone();
            let paths = OverlayTree::from_paths(layer.files.iter().cloned());
            facts.tree = layer.texts.iter().fold(paths, |tree, (path, bytes)| {
                tree.with_bytes(path, bytes.clone())
            });
            (&layer.manifest, facts)
        })
        .collect()
}

/// What the overlays' env scripts gave when the connection ran them, by overlay and place among its scripts. An environment
/// built again from these takes each script's output as it was, so a script runs once however many environments are built.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScriptRuns {
    /// The interpreters the host had when they ran, which decide which default script an overlay has; none is every one.
    pub(crate) interpreters: Option<Vec<String>>,
    /// What each gave: its standard output, or why it gave none.
    pub(crate) results: BTreeMap<(String, usize), aos::ScriptResult>,
}

/// Builds the AOS overlays' environment after each overlay in turn, lowest first, over this process's environment as
/// `lookup` reads it. A placeholder reads what the overlays set and any variable of this process. Only the variables an
/// overlay's text names are read from the process. The env scripts that ran (`scripts`) apply where the order of
/// application puts them.
pub(crate) fn build_each(
    layers: &[AosLayer],
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
    scripts: &ScriptRuns,
) -> Vec<Result<Built, ProfileError>> {
    let windows = layers.first().is_some_and(|first| first.os == "windows");
    let named: BTreeSet<String> = layers.iter().flat_map(names_of).collect();
    let mut base = BTreeMap::new();
    let mut process = Environment::new(Vec::new(), windows);
    let mut marked = Environment::new(Vec::new(), windows);
    for name in named {
        if let Some(value) = lookup(&name) {
            process.set(&name, value.clone(), false);
            marked.set(&name, format!("{MARK}{value}"), false);
            base.insert(name, value);
        }
    }
    let active = active_facts(layers, lookup, scripts.interpreters.as_deref());
    // A script the connection did not run gives nothing.
    let replay = |call: &aos::ScriptCall<'_>| -> aos::ScriptResult {
        scripts
            .results
            .get(&(call.identity.to_owned(), call.index))
            .cloned()
            .unwrap_or(Err(aos::ScriptFailure::NotRun))
    };
    (0..layers.len())
        .map(|last| {
            let composed = aos::composed_environment_running(
                &active[..=last],
                process.clone(),
                pre_env,
                &mut Vec::new(),
                &mut |call| replay(call),
            );
            match composed.env {
                Ok(env) => Ok(Built {
                    env,
                    base: base.clone(),
                    traced: aos::composed_environment_running(
                        &active[..=last],
                        marked.clone(),
                        pre_env,
                        &mut Vec::new(),
                        &mut |call| replay(call),
                    )
                    .env
                    .ok(),
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
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
    scripts: &ScriptRuns,
) -> Result<Built, ProfileError> {
    match build_each(layers, lookup, pre_env, scripts).pop() {
        Some(built) => built,
        None => Ok(Built {
            env: Environment::new(Vec::new(), cfg!(windows)),
            base: BTreeMap::new(),
            traced: None,
            hook_reports: Vec::new(),
        }),
    }
}
