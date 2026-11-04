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
use crate::aos::{self, Environment, Facts, Limits, Overlay, TreeEntry, Weakening};

/// An AOS overlay as Canoa applies it: what building its environment needs, and what Canoa does not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AosLayer {
    /// The name Canoa.cfg gives it, its identity.
    pub identity: String,
    /// Its root, absolute, as processes see it.
    pub root: String,
    /// The OS its environment is built for, as `os` keys name it.
    pub os: String,
    /// Its tree digest.
    pub digest: String,
    /// Whether Canoa.cfg pins it by digest.
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
    /// The name Canoa.cfg gives it.
    pub identity: &'a str,
    /// Its root, absolute.
    pub root: &'a str,
    /// The OS its environment is for: `windows`, `linux`, `macos`, ...
    pub os: &'a str,
    /// The AOS harness id of the agent it is for (`claude-code`, `codex`, ...), whose hints apply.
    pub harness: Option<&'a str>,
    /// Whether Canoa.cfg pins it by digest; checking the digest is the caller's.
    pub pinned: bool,
    /// Its tree, as [`aos::read_tree`] reads it or a source's objects hold it.
    pub tree: &'a [TreeEntry],
}

impl<'a> OverlayInput<'a> {
    /// The overlay `identity` at `root`, holding `tree`, for `os`.
    #[must_use]
    pub fn new(identity: &'a str, root: &'a str, os: &'a str, tree: &'a [TreeEntry]) -> Self {
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
            "manifest" => "its overlay.cfg cannot be read",
            "invalid" => "its overlay.cfg is not valid",
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

/// An AOS overlay's report in the view: what Canoa does not apply, what is not granted, and its weakenings.
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
    /// Its weakenings, each time it is activated.
    pub weakenings: Vec<Weakening>,
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
            "weakenings": self
                .weakenings
                .iter()
                .map(|weakening| json!([weakening.id, weakening.subject]))
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

fn resolve_path(root: &str, relative: &str, windows: bool) -> String {
    let separator = if windows { "\\" } else { "/" };
    format!(
        "{}{separator}{}",
        root.trim_end_matches(['/', '\\']),
        relative.replace('/', separator)
    )
}

fn insecure(url: &str) -> bool {
    if !url
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
    {
        return false;
    }
    let host = url[7..]
        .split('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let host = host
        .rsplit_once(':')
        .map_or(host, |(host, _)| host)
        .to_ascii_lowercase();
    host != "localhost" && host != "[::1]" && !host.starts_with("127.")
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
/// [`OverlayRefusal`] for a name that is not a valid identity, a tree that cannot be digested, an `overlay.cfg` that
/// cannot be read or is not valid, an env file that is missing or cannot be read, and a variable Canoa does not let a
/// layer from elsewhere set ([`is_reserved_env`]).
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the manifest, field by field"
)]
pub fn overlay_layer(input: &OverlayInput<'_>) -> Result<Layer, OverlayRefusal> {
    if !aos::valid_overlay_name(input.identity) {
        return Err(refused("name", input.identity));
    }
    let digest =
        aos::digest_entries(input.tree).map_err(|error| refused("digest", error.to_string()))?;
    let members: BTreeMap<String, &[u8]> = input
        .tree
        .iter()
        .filter(|entry| entry.kind == aos::EntryKind::File && !entry.is_excluded())
        .map(|entry| {
            let path = entry.components.join(&b'/');
            (
                String::from_utf8_lossy(&path).into_owned(),
                entry.data.as_slice(),
            )
        })
        .collect();
    let files: BTreeSet<String> = members.keys().cloned().collect();
    let manifest = match members.get("overlay.cfg") {
        Some(bytes) => aos::read_document(bytes, Limits::default())
            .map_err(|error| refused("manifest", error.to_string()))?,
        None => Map::new(),
    };
    // `about` is store metadata: a problem in it never stops an overlay, and Canoa applies nothing of it.
    if let Some(issue) = aos::validate(&manifest, Some(&files))
        .iter()
        .find(|issue| !aos::applies(core::slice::from_ref(issue)))
    {
        return Err(refused("invalid", format!("{} {}", issue.at, issue.code)));
    }
    let windows = input.os == "windows";
    let mut profile = Profile::default();
    let mut not_applied = Vec::new();
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
    for file in aos::env_files_of(&manifest, &files) {
        if !aos::os_matches(file.os.as_deref(), input.os) {
            continue;
        }
        let bytes = *members
            .get(&file.path)
            .ok_or_else(|| refused("env-file", format!("{}: missing-file", file.path)))?;
        let invalid = |code: &str| refused("env-file", format!("{}: {code}", file.path));
        if file.dotenv {
            for line in aos::parse_dotenv(bytes).map_err(|error| invalid(error.code))? {
                let value = if line.literal {
                    EnvValue::Value(line.text)
                } else {
                    EnvValue::Text(line.text)
                };
                set(&line.name, value, &mut env);
            }
        } else {
            let mapping = aos::read_document(bytes, Limits::default())
                .map_err(|error| invalid(error.code))?;
            let wrapped = Map::from_iter([("env".to_owned(), Value::Object(mapping.clone()))]);
            if !aos::validate(&wrapped, None).is_empty() {
                return Err(invalid("invalid-env-file"));
            }
            for (name, value) in sorted(&mapping) {
                set(name, env_value(value), &mut env);
            }
        }
        texts.insert(file.path.clone(), bytes.to_vec());
    }
    for (name, value) in entries(manifest.get("env")) {
        set(name, env_value(value), &mut env);
    }
    let mut applied = manifest.clone();
    let defaults = aos::default_paths(&manifest, &files);
    for addition in &defaults {
        not_applied.push(format!(
            "paths: {}/ on {}, which a layer from elsewhere may not change",
            addition.path, addition.variable
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
    for addition in aos::paths_of(&applied, &files) {
        if aos::os_matches(addition.os.as_deref(), input.os) {
            lists
                .entry(addition.variable.clone())
                .or_default()
                .push((addition.path.clone(), addition.append));
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

    for name in aos::skills_of(&manifest, &files) {
        profile.skills.insert(
            name.clone(),
            Change::Set(Skill::new(format!("skills/{name}"))),
        );
    }

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
            let plain = insecure(&url);
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
        aos::rules_of(&manifest, &files)
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
        aos::context_of(&manifest, &files)
            .into_iter()
            .map(Context::File),
    );
    for (id, hints) in entries(manifest.get("harness")) {
        if input.harness != Some(id.as_str()) {
            not_applied.push(format!("harness.{id}: hints for another harness"));
            continue;
        }
        for (key, value) in entries(Some(hints)) {
            match (key.as_str(), value) {
                ("model", Value::String(model)) => profile.model = Some(model.clone()),
                ("effort", Value::String(effort)) => profile.effort = Some(effort.clone()),
                ("context", Value::Array(paths)) => profile.contexts.extend(
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|path| Context::File(path.to_owned())),
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
        digest: digest.digest,
        pinned: input.pinned,
        manifest: applied,
        files,
        texts,
        references,
        not_applied,
    }));
    Ok(layer)
}

fn granted(shared: &IndexMap<String, LayerId>, name: &str, windows: bool) -> bool {
    shared
        .keys()
        .any(|shared| shared == name || windows && shared.eq_ignore_ascii_case(name))
}

/// The facts an overlay's references resolve against: the `env` provider alone, and only the variables the person's
/// own layers share.
fn facts(
    os: &str,
    shared: &IndexMap<String, LayerId>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Facts {
    let mut facts = Facts::new(os);
    facts
        .grants
        .insert("env".to_owned(), shared.keys().cloned().collect());
    facts.host_env = shared
        .keys()
        .filter_map(|name| lookup(name).map(|value| (name.clone(), value)))
        .collect();
    facts
}

/// An overlay's report, with what the person's own layers share as its grants.
pub(crate) fn report(
    layer: &LayerId,
    aos: &AosLayer,
    shared: &IndexMap<String, LayerId>,
) -> OverlayReport {
    let windows = aos.os == "windows";
    let mut weakenings = Vec::new();
    if !aos.pinned {
        weakenings.push(Weakening::new("unpinned", ""));
    }
    let facts = facts(&aos.os, shared, &|_| None);
    let ungranted = aos::grants_report(&aos.manifest, &facts, &mut weakenings);
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
    weakenings.sort();
    weakenings.dedup();
    OverlayReport {
        layer: layer.clone(),
        digest: aos.digest.clone(),
        not_applied: aos.not_applied.clone(),
        ungranted,
        unresolved,
        weakenings,
    }
}

fn reason(code: &str) -> &'static str {
    match code {
        "undefined-variable" => "an overlay's placeholder names a variable that is not set",
        "secret-placeholder" => {
            "an overlay's placeholder reads a secret, or a variable of this computer none of your layers shares"
        }
        "unresolved-reference" => {
            "an overlay reads a variable none of your layers shares, or one that is not set, or a secret Canoa does not hold"
        }
        "invalid-pattern" => "an overlay's placeholder has a pattern that is not valid",
        "invalid-reference" => "an overlay names a secret file that is not a relative path",
        "missing-file" | "invalid-env-file" => "an overlay's env file cannot be read",
        "env-path-conflict" => "an overlay sets a variable both by env and by a path addition",
        "reserved-variable" => "an overlay sets a variable the consumer sets",
        _ => "an overlay's environment cannot be built",
    }
}

/// Builds the AOS overlays' environment, lowest first, over this process's environment as `lookup` reads it: every
/// root variable first, then each overlay's env files, `env` and path additions. A placeholder reads only what the
/// overlays set and the variables the person's own layers share; any other variable of this process is seen by a path
/// addition and `if_unset`, never by a placeholder.
pub(crate) fn build<'a>(
    layers: &[AosLayer],
    shared: &IndexMap<String, LayerId>,
    base: aos::Base<'a>,
) -> Result<Environment<'a>, ProfileError> {
    let Some(first) = layers.first() else {
        return Ok(Environment::new(cfg!(windows)));
    };
    let windows = first.os == "windows";
    let mut env = Environment::with_base(windows, base);
    for layer in layers {
        env.set(&aos::root_variable(&layer.identity), &layer.root, false);
    }
    let lookup = |name: &str| base(name).map(|(value, _)| value);
    let facts = facts(&first.os, shared, &lookup);
    for layer in layers {
        let read = |path: &str| layer.texts.get(path).cloned();
        let overlay = Overlay {
            identity: &layer.identity,
            manifest: &layer.manifest,
            files: &layer.files,
            root: &layer.root,
            state: None,
            read: &read,
        };
        aos::apply_overlay(&mut env, &overlay, &facts, &mut Vec::new()).map_err(|error| {
            ProfileError::Resolve {
                key: format!("overlay:{}.env.{}", layer.identity, error.subject),
                what: reason(error.code),
            }
        })?;
    }
    Ok(env)
}

/// The base a build reads this process's environment through: each variable's value, readable by a placeholder only
/// when the person's own layers share it.
pub(crate) fn base_of<'a>(
    shared: &'a IndexMap<String, LayerId>,
    windows: bool,
    lookup: &'a dyn Fn(&str) -> Option<String>,
) -> impl Fn(&str) -> Option<(String, bool)> + 'a {
    move |name: &str| lookup(name).map(|value| (value, granted(shared, name, windows)))
}
