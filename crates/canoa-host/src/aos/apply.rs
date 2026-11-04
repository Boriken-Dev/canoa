//! Applying overlays: the defaults conventional files contribute, requirements and composition order, and the
//! environment, built in the order of application with each text value substituted as it is set.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::read::{Limits, parse_dotenv, read_document};
use super::subst::substitute;
use super::validate::validate;
use super::{AosError, is_relative_path, is_reserved_variable, is_sensitive, normalize_name};

/// The environment a build starts from, read one variable at a time: its value, and whether a placeholder may read it.
pub type Base<'a> = &'a dyn Fn(&str) -> Option<(String, bool)>;

/// An environment being built: variables in the order they were first set, over a base it was started from. On Windows
/// names compare without regard to ASCII case, and setting one keeps the name it already has. A variable whose value
/// came from a secret reference, or a base variable marked unreadable, is never read by a placeholder.
#[derive(Clone, Default)]
pub struct Environment<'a> {
    windows: bool,
    values: IndexMap<String, String>,
    secret: BTreeSet<String>,
    extra: Vec<(String, String)>,
    base: Option<Base<'a>>,
}

impl core::fmt::Debug for Environment<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Environment")
            .field("windows", &self.windows)
            .field("names", &self.values.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl<'a> Environment<'a> {
    /// An empty environment; `windows` compares names as Windows does.
    #[must_use]
    pub fn new(windows: bool) -> Self {
        Self {
            windows,
            ..Self::default()
        }
    }

    /// An environment over `base`, whose variables are seen until one is set and are never in [`Environment::values`].
    #[must_use]
    pub fn with_base(windows: bool, base: Base<'a>) -> Self {
        Self {
            windows,
            base: Some(base),
            ..Self::default()
        }
    }

    fn key(&self, name: &str) -> Option<String> {
        if self.values.contains_key(name) {
            return Some(name.to_owned());
        }
        if self.windows {
            return self
                .values
                .keys()
                .find(|key| key.eq_ignore_ascii_case(name))
                .cloned();
        }
        None
    }

    fn base_value(&self, name: &str) -> Option<(String, bool)> {
        self.base.and_then(|base| base(name))
    }

    /// A variable's value, set or from the base.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        match self.key(name) {
            Some(key) => self.values.get(&key).cloned(),
            None => self.base_value(name).map(|(value, _)| value),
        }
    }

    /// Whether a placeholder may not read the variable: its value came from a secret reference, or from a base
    /// variable marked so.
    #[must_use]
    pub fn is_secret(&self, name: &str) -> bool {
        match self.key(name) {
            Some(key) => self.secret.contains(&key),
            None => self.base_value(name).is_some_and(|(_, readable)| !readable),
        }
    }

    /// What a placeholder reads: an overlay's own `AOS_OVERLAY_*` first, then the variable.
    ///
    /// # Errors
    ///
    /// `secret-placeholder` for a variable that holds a secret.
    pub fn lookup(&self, name: &str) -> Result<Option<String>, AosError> {
        for (extra, value) in &self.extra {
            if extra == name || self.windows && extra.eq_ignore_ascii_case(name) {
                return Ok(Some(value.clone()));
            }
        }
        if self.is_secret(name) {
            return Err(AosError::new("secret-placeholder", name));
        }
        Ok(self.get(name))
    }

    /// Sets a variable; `secret` marks a value no placeholder may read, and setting it from anything else clears that.
    pub fn set(&mut self, name: &str, value: &str, secret: bool) {
        let key = self.key(name).unwrap_or_else(|| name.to_owned());
        self.values.insert(key.clone(), value.to_owned());
        if secret {
            self.secret.insert(key);
        } else {
            self.secret.remove(&key);
        }
    }

    /// Every variable set, in the order first set; none from the base.
    #[must_use]
    pub fn values(&self) -> &IndexMap<String, String> {
        &self.values
    }

    /// Whether names compare as Windows does.
    #[must_use]
    pub const fn windows(&self) -> bool {
        self.windows
    }
}

/// What the consumer has and grants, the inputs of resolving secret references.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Facts {
    /// The OS name, as `os` keys write it: `windows`, `linux`, `macos`, ...
    pub os: String,
    /// What the overlay may read, by provider (`env`, `file`, `secret` for named secrets): host variables, file keys
    /// and secret names.
    pub grants: BTreeMap<String, Vec<String>>,
    /// The consumer's own environment, which the `env` provider reads.
    pub host_env: BTreeMap<String, String>,
    /// Named secrets.
    pub secrets: BTreeMap<String, String>,
    /// The secrets directory's files by relative path, or none when there is no such directory.
    pub secret_files: Option<BTreeMap<String, String>>,
    /// The providers the consumer has.
    pub providers: Vec<String>,
}

impl Facts {
    /// Facts for `os` with nothing granted and only the `env` provider.
    #[must_use]
    pub fn new(os: &str) -> Self {
        Self {
            os: os.to_owned(),
            providers: alloc::vec!["env".to_owned()],
            ..Self::default()
        }
    }
}

/// One overlay to apply: its identity, manifest, files, the root it is exposed at, and how to read a file of it.
#[allow(missing_debug_implementations, reason = "it holds a callback")]
pub struct Overlay<'a> {
    /// The name it is installed under.
    pub identity: &'a str,
    /// Its manifest, read and validated.
    pub manifest: &'a Map<String, Value>,
    /// Its files' relative paths, from which the defaults follow.
    pub files: &'a BTreeSet<String>,
    /// The absolute root it is exposed at.
    pub root: &'a str,
    /// Its state directory, when it has one.
    pub state: Option<&'a str>,
    /// A file's bytes by relative path, for its env files.
    pub read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
}

/// A weakening: a choice looser than the default, reported with an identifier and a subject, never a value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Weakening {
    /// `unpinned`, `env-sensitive`, `env-passthrough`, `secret-read`, `insecure-transport`, ...
    pub id: &'static str,
    /// The variable, reference, server or path; empty for `unpinned`.
    pub subject: String,
}

impl Weakening {
    /// The weakening `id` about `subject`.
    #[must_use]
    pub fn new(id: &'static str, subject: &str) -> Self {
        Self {
            id,
            subject: subject.to_owned(),
        }
    }
}

/// An entry of `paths`, declared or by default.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathAddition {
    /// The list variable.
    pub variable: String,
    /// The directory, relative to the root.
    pub path: String,
    /// Appended rather than prepended.
    pub append: bool,
    /// The OSes it applies on; none for every OS.
    pub os: Option<Vec<String>>,
}

/// An entry of `env_files`, declared or by default.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnvFile {
    /// The file, relative to the root.
    pub path: String,
    /// Read as `dotenv` rather than an `aos` document.
    pub dotenv: bool,
    /// The OSes it applies on; none for every OS.
    pub os: Option<Vec<String>>,
}

fn default_on(manifest: &Map<String, Value>, key: &str) -> bool {
    manifest
        .get("defaults")
        .and_then(|defaults| defaults.get(key))
        != Some(&Value::Bool(false))
}

fn members(files: &BTreeSet<String>) -> impl Iterator<Item = &String> {
    files.iter().filter(|path| is_relative_path(path))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn os_of(entry: &Value) -> Option<Vec<String>> {
    entry.get("os").map(|os| strings(Some(os)))
}

/// Whether an entry with `os` applies on `current`: no list is every OS, and `unix` is every OS but Windows that the
/// specification names.
#[must_use]
pub fn os_matches(os: Option<&[String]>, current: &str) -> bool {
    os.is_none_or(|names| {
        names.iter().any(|name| name == current)
            || names.iter().any(|name| name == "unix")
                && current != "windows"
                && ["macos", "linux", "freebsd", "openbsd", "netbsd"].contains(&current)
    })
}

/// Default entries first, then declared ones; a declared entry naming the same file replaces the default one.
fn merged(defaults: Vec<String>, declared: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = defaults
        .into_iter()
        .filter(|path| !declared.contains(path))
        .collect();
    out.extend(declared);
    out
}

/// The `bin` and `lib` defaults that are on, present and not replaced by a declared entry.
pub(crate) fn default_paths(
    manifest: &Map<String, Value>,
    files: &BTreeSet<String>,
) -> Vec<PathAddition> {
    let declared: Vec<String> = manifest
        .get("paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str).map(str::to_owned))
        .collect();
    [
        ("bin", "PATH", "bin"),
        ("lib", "PYTHONPATH", "lib/python"),
        ("lib", "NODE_PATH", "lib/node"),
    ]
    .into_iter()
    .filter(|(key, _, directory)| {
        default_on(manifest, key)
            && members(files).any(|path| path.starts_with(&format!("{directory}/")))
            && !declared.iter().any(|path| path == directory)
    })
    .map(|(_, variable, directory)| PathAddition {
        variable: variable.to_owned(),
        path: directory.to_owned(),
        append: false,
        os: None,
    })
    .collect()
}

/// The defaults `bin` and `lib`, then `paths`.
#[must_use]
pub fn paths_of(manifest: &Map<String, Value>, files: &BTreeSet<String>) -> Vec<PathAddition> {
    let mut out = default_paths(manifest, files);
    for entry in manifest
        .get("paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        out.push(PathAddition {
            variable: entry
                .get("variable")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            path: entry
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            append: entry.get("position").and_then(Value::as_str) == Some("append"),
            os: os_of(entry),
        });
    }
    out
}

/// Every `rules/<name>.md` directly in `rules/`, by path bytes, then `rules`.
#[must_use]
pub fn rules_of(manifest: &Map<String, Value>, files: &BTreeSet<String>) -> Vec<String> {
    let found = if default_on(manifest, "rules") {
        members(files)
            .filter(|path| {
                path.strip_prefix("rules/").is_some_and(|name| {
                    !name.contains('/') && name.len() > 3 && name.ends_with(".md")
                })
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    merged(found, strings(manifest.get("rules")))
}

/// The root `CONTEXT.md`, then `context`.
#[must_use]
pub fn context_of(manifest: &Map<String, Value>, files: &BTreeSet<String>) -> Vec<String> {
    let found =
        if default_on(manifest, "context") && members(files).any(|path| path == "CONTEXT.md") {
            alloc::vec!["CONTEXT.md".to_owned()]
        } else {
            Vec::new()
        };
    merged(found, strings(manifest.get("context")))
}

/// Every `skills/<name>/SKILL.md` with a skill name, and the skills `skills` lists, by name bytes.
#[must_use]
pub fn skills_of(manifest: &Map<String, Value>, files: &BTreeSet<String>) -> Vec<String> {
    let mut found: BTreeSet<String> = strings(manifest.get("skills")).into_iter().collect();
    if default_on(manifest, "skills") {
        for path in members(files) {
            let parts: Vec<&str> = path.split('/').collect();
            if let ["skills", name, "SKILL.md"] = parts.as_slice() {
                let bytes = name.as_bytes();
                if bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                    && bytes.iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
                    })
                {
                    found.insert((*name).to_owned());
                }
            }
        }
    }
    found.into_iter().collect()
}

/// The root file `env`, read as `dotenv`, then `env_files`.
#[must_use]
pub fn env_files_of(manifest: &Map<String, Value>, files: &BTreeSet<String>) -> Vec<EnvFile> {
    let declared: Vec<EnvFile> = manifest
        .get("env_files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| match entry {
            Value::String(path) => EnvFile {
                path: path.clone(),
                dotenv: false,
                os: None,
            },
            _ => EnvFile {
                path: entry
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                dotenv: entry.get("format").and_then(Value::as_str) == Some("dotenv"),
                os: os_of(entry),
            },
        })
        .collect();
    let mut out = Vec::new();
    if default_on(manifest, "env")
        && members(files).any(|path| path == "env")
        && !declared.iter().any(|entry| entry.path == "env")
    {
        out.push(EnvFile {
            path: "env".to_owned(),
            dotenv: true,
            os: None,
        });
    }
    out.extend(declared);
    out
}

fn resolve_path(root: &str, relative: &str, windows: bool) -> String {
    let separator = if windows { "\\" } else { "/" };
    format!(
        "{}{separator}{}",
        root.trim_end_matches(['/', '\\']),
        relative.replace('/', separator)
    )
}

/// How a report names a reference: `secret:<name>` or `<provider>:<key>`, never a value or a provider's other fields.
fn reference_name(entry: &Map<String, Value>) -> String {
    if let Some(name) = entry.get("secret").and_then(Value::as_str) {
        return format!("secret:{name}");
    }
    format!(
        "{}:{}",
        entry.get("provider").and_then(Value::as_str).unwrap_or(""),
        entry.get("key").and_then(Value::as_str).unwrap_or("")
    )
}

fn is_reference(entry: &Map<String, Value>) -> bool {
    entry.contains_key("provider") || entry.contains_key("secret")
}

fn granted(entry: &Map<String, Value>, facts: &Facts) -> bool {
    let (provider, key) = match entry.get("secret").and_then(Value::as_str) {
        Some(name) => ("secret", name),
        None => (
            entry.get("provider").and_then(Value::as_str).unwrap_or(""),
            entry.get("key").and_then(Value::as_str).unwrap_or(""),
        ),
    };
    facts
        .grants
        .get(provider)
        .is_some_and(|keys| keys.iter().any(|granted| granted == key))
}

/// A granted reference's value, or none when it cannot be resolved.
fn secret_value(
    entry: &Map<String, Value>,
    facts: &Facts,
    windows: bool,
) -> Result<Option<String>, AosError> {
    let provider = entry.get("provider").and_then(Value::as_str);
    let key = entry.get("key").and_then(Value::as_str).unwrap_or("");
    if provider == Some("file") && !is_relative_path(key) {
        return Err(AosError::new("invalid-reference", key));
    }
    if !granted(entry, facts) {
        return Ok(None);
    }
    if let Some(name) = entry.get("secret").and_then(Value::as_str) {
        return Ok(facts.secrets.get(name).cloned());
    }
    let provider = provider.unwrap_or("");
    if !facts.providers.iter().any(|known| known == provider) {
        return Ok(None);
    }
    Ok(match provider {
        "env" => {
            let mut host = Environment::new(windows);
            for (name, value) in &facts.host_env {
                host.set(name, value, false);
            }
            host.get(key)
        }
        "file" => facts
            .secret_files
            .as_ref()
            .and_then(|files| files.get(key))
            .map(|value| {
                value
                    .strip_suffix("\r\n")
                    .or_else(|| value.strip_suffix('\n'))
                    .unwrap_or(value)
                    .to_owned()
            }),
        _ => None,
    })
}

fn refuse(error: AosError, name: &str) -> AosError {
    AosError::new(error.code, name)
}

fn apply_mapping(
    env: &mut Environment,
    mapping: &Map<String, Value>,
    facts: &Facts,
    root: &str,
    weakenings: &mut Vec<Weakening>,
) -> Result<(), AosError> {
    let windows = env.windows();
    let before = env.clone();
    for (name, entry) in mapping {
        if is_reserved_variable(name) {
            return Err(AosError::new("reserved-variable", name));
        }
        let text_entry;
        let entry = match entry {
            Value::String(text) => {
                text_entry = Map::from_iter([("value".to_owned(), Value::String(text.clone()))]);
                &text_entry
            }
            Value::Object(map) => map,
            _ => return Err(AosError::new("invalid-env-file", name)),
        };
        if entry.get("if_unset") == Some(&Value::Bool(true)) && before.get(name).is_some() {
            continue;
        }
        let mut secret = false;
        let value = if let Some(text) = entry.get("value").and_then(Value::as_str) {
            substitute(text, &|name| before.lookup(name)).map_err(|error| refuse(error, name))?
        } else if let Some(path) = entry.get("path").and_then(Value::as_str) {
            resolve_path(root, path, windows)
        } else {
            let Some(value) = secret_value(entry, facts, windows)? else {
                if entry.get("optional") == Some(&Value::Bool(true)) {
                    continue;
                }
                return Err(AosError::new("unresolved-reference", name));
            };
            secret = true;
            match entry.get("provider").and_then(Value::as_str) {
                Some("env") => weakenings.push(Weakening::new(
                    "env-passthrough",
                    entry.get("key").and_then(Value::as_str).unwrap_or(""),
                )),
                _ => weakenings.push(Weakening::new("secret-read", &reference_name(entry))),
            }
            value
        };
        if is_sensitive(name) {
            weakenings.push(Weakening::new("env-sensitive", name));
        }
        env.set(name, &value, secret);
    }
    Ok(())
}

fn apply_paths(
    env: &mut Environment,
    paths: &[PathAddition],
    root: &str,
    os: &str,
    weakenings: &mut Vec<Weakening>,
) {
    let windows = env.windows();
    let separator = if windows { ";" } else { ":" };
    let mut grouped: IndexMap<String, (String, Vec<&PathAddition>)> = IndexMap::new();
    for entry in paths
        .iter()
        .filter(|entry| os_matches(entry.os.as_deref(), os))
    {
        let key = if windows {
            entry.variable.to_ascii_uppercase()
        } else {
            entry.variable.clone()
        };
        grouped
            .entry(key)
            .or_insert_with(|| (entry.variable.clone(), Vec::new()))
            .1
            .push(entry);
    }
    for (variable, entries) in grouped.into_values() {
        if is_sensitive(&variable)
            && !["PATH", "PYTHONPATH", "NODE_PATH"]
                .contains(&variable.to_ascii_uppercase().as_str())
        {
            weakenings.push(Weakening::new("env-sensitive", &variable));
        }
        let (mut prepend, mut append, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
        for entry in entries {
            let resolved = resolve_path(root, &entry.path, windows);
            if !seen.insert(resolved.clone()) {
                continue;
            }
            if entry.append {
                append.push(resolved);
            } else {
                prepend.push(resolved);
            }
        }
        let current = env.get(&variable).unwrap_or_default();
        let kept = current
            .split(separator)
            .filter(|component| !component.is_empty() && !seen.contains(*component))
            .map(str::to_owned);
        let joined: Vec<String> = prepend.into_iter().chain(kept).chain(append).collect();
        let secret = env.is_secret(&variable);
        env.set(&variable, &joined.join(separator), secret);
    }
}

/// Applies one overlay to `env`: its env files that apply on this OS, then `env`, then `paths`, each with its defaults
/// first, every text value substituted against the environment as it stands. The overlay's root variable is the
/// caller's to set first, with every other active overlay's. Env scripts are not run. Weakenings found are added to
/// `weakenings`.
///
/// # Errors
///
/// The applying classes, `missing-file`, `invalid-env-file`, `env-path-conflict`, `reserved-variable`,
/// `unresolved-reference` and `invalid-reference`, and the substitution classes; the subject is the variable being set
/// or the env file. Nothing is applied after the first.
pub fn apply_overlay(
    env: &mut Environment,
    overlay: &Overlay<'_>,
    facts: &Facts,
    weakenings: &mut Vec<Weakening>,
) -> Result<(), AosError> {
    env.extra = alloc::vec![
        ("AOS_OVERLAY_ROOT".to_owned(), overlay.root.to_owned()),
        (
            "AOS_OVERLAY_NAME".to_owned(),
            normalize_name(overlay.identity)
        ),
    ];
    if let Some(state) = overlay.state {
        env.extra
            .push(("AOS_OVERLAY_STATE".to_owned(), state.to_owned()));
    }
    let result = apply_inner(env, overlay, facts, weakenings);
    env.extra.clear();
    result
}

fn apply_inner(
    env: &mut Environment,
    overlay: &Overlay<'_>,
    facts: &Facts,
    weakenings: &mut Vec<Weakening>,
) -> Result<(), AosError> {
    let manifest = overlay.manifest;
    let paths = paths_of(manifest, overlay.files);
    let listed: BTreeSet<String> = paths
        .iter()
        .map(|entry| entry.variable.to_ascii_uppercase())
        .collect();
    let empty = Map::new();
    let own = manifest
        .get("env")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if let Some(name) = own
        .keys()
        .find(|name| listed.contains(&name.to_ascii_uppercase()))
    {
        return Err(AosError::new("env-path-conflict", name));
    }
    for file in env_files_of(manifest, overlay.files) {
        if !os_matches(file.os.as_deref(), &facts.os) {
            continue;
        }
        let bytes =
            (overlay.read)(&file.path).ok_or_else(|| AosError::new("missing-file", &file.path))?;
        let invalid = || AosError::new("invalid-env-file", &file.path);
        if file.dotenv {
            let lines = parse_dotenv(&bytes).map_err(|_| invalid())?;
            if let Some(line) = lines
                .iter()
                .find(|line| listed.contains(&line.name.to_ascii_uppercase()))
            {
                return Err(AosError::new("env-path-conflict", &line.name));
            }
            for line in lines {
                if is_reserved_variable(&line.name) {
                    return Err(AosError::new("reserved-variable", &line.name));
                }
                let value = if line.literal {
                    line.text
                } else {
                    substitute(&line.text, &|name| env.lookup(name))
                        .map_err(|error| refuse(error, &line.name))?
                };
                if is_sensitive(&line.name) {
                    weakenings.push(Weakening::new("env-sensitive", &line.name));
                }
                env.set(&line.name, &value, false);
            }
        } else {
            let mapping = read_document(&bytes, Limits::default()).map_err(|_| invalid())?;
            let wrapped = Map::from_iter([("env".to_owned(), Value::Object(mapping.clone()))]);
            if !validate(&wrapped, None).is_empty() {
                return Err(invalid());
            }
            if let Some(name) = mapping
                .keys()
                .find(|name| listed.contains(&name.to_ascii_uppercase()))
            {
                return Err(AosError::new("env-path-conflict", name));
            }
            apply_mapping(env, &mapping, facts, overlay.root, weakenings)?;
        }
    }
    apply_mapping(env, own, facts, overlay.root, weakenings)?;
    apply_paths(env, &paths, overlay.root, &facts.os, weakenings);
    Ok(())
}

/// What an overlay's MCP servers and optional references read that is not granted, as `<provider>:<key>`,
/// `secret:<name>` or `env:<variable>`, sorted; the granted reads and plain `http` to a remote host are added to
/// `weakenings`.
#[must_use]
pub fn grants_report(
    manifest: &Map<String, Value>,
    facts: &Facts,
    weakenings: &mut Vec<Weakening>,
) -> Vec<String> {
    let mut ungranted = BTreeSet::new();
    let host_grants = facts.grants.get("env").cloned().unwrap_or_default();
    let servers = manifest
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for server in &servers {
        for value in server
            .get("env")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(_, value)| value)
        {
            let Some(entry) = value.as_object().filter(|entry| is_reference(entry)) else {
                continue;
            };
            if !granted(entry, facts) {
                ungranted.insert(reference_name(entry));
            } else if entry.get("provider").and_then(Value::as_str) == Some("env") {
                weakenings.push(Weakening::new(
                    "env-passthrough",
                    entry.get("key").and_then(Value::as_str).unwrap_or(""),
                ));
            } else {
                weakenings.push(Weakening::new("secret-read", &reference_name(entry)));
            }
        }
        let headers = server
            .get("headers_env")
            .and_then(Value::as_object)
            .map(|headers| {
                headers
                    .values()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(Vec::new);
        for name in strings(server.get("env_passthrough"))
            .into_iter()
            .chain(headers)
        {
            if host_grants.contains(&name) {
                weakenings.push(Weakening::new("env-passthrough", &name));
            } else {
                ungranted.insert(format!("env:{name}"));
            }
        }
        if server
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(super::is_insecure_url)
        {
            weakenings.push(Weakening::new(
                "insecure-transport",
                server.get("name").and_then(Value::as_str).unwrap_or(""),
            ));
        }
    }
    for entry in manifest
        .get("env")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(_, entry)| entry)
    {
        if let Some(entry) = entry.as_object()
            && is_reference(entry)
            && entry.get("optional") == Some(&Value::Bool(true))
            && !granted(entry, facts)
        {
            ungranted.insert(reference_name(entry));
        }
    }
    ungranted.into_iter().collect()
}

/// A requirement no overlay offers: the overlay that requires it (empty for one requested), and the name.
pub type Missing = (String, String);

/// The visiting order of the active overlays, and the missing requirements.
pub type Resolved = (Vec<String>, Vec<Missing>);

/// Resolves `requires`: the visiting order of the active overlays and each missing requirement, or a cycle as its
/// chain, `a -> b -> a`. `overlays` maps a normalized identity to its manifest.
///
/// # Errors
///
/// The chain of the first cycle found.
pub fn resolve(
    overlays: &BTreeMap<String, Map<String, Value>>,
    requested: &[String],
) -> Result<Resolved, Vec<String>> {
    fn visit(
        name: String,
        chain: &mut Vec<String>,
        overlays: &BTreeMap<String, Map<String, Value>>,
        order: &mut Vec<String>,
        missing: &mut Vec<(String, String)>,
    ) -> Result<(), Vec<String>> {
        if order.contains(&name) {
            return Ok(());
        }
        if let Some(at) = chain.iter().position(|seen| *seen == name) {
            let mut cycle = chain[at..].to_vec();
            cycle.push(name);
            return Err(cycle);
        }
        let Some(manifest) = overlays.get(&name) else {
            missing.push((chain.last().cloned().unwrap_or_default(), name));
            return Ok(());
        };
        chain.push(name.clone());
        for required in strings(manifest.get("requires")) {
            visit(normalize_name(&required), chain, overlays, order, missing)?;
        }
        chain.pop();
        order.push(name);
        Ok(())
    }
    let (mut order, mut missing) = (Vec::new(), Vec::new());
    for name in requested {
        visit(
            normalize_name(name),
            &mut Vec::new(),
            overlays,
            &mut order,
            &mut missing,
        )?;
    }
    Ok((order, missing))
}

/// The composition order: ascending `(priority, name, identity)`, where `name` is the manifest's `name` or the identity
/// when it is absent or empty, and strings compare by code point.
#[must_use]
pub fn composition_order(
    overlays: &BTreeMap<String, Map<String, Value>>,
    active: &[String],
) -> Vec<String> {
    let key = |identity: &String| {
        let manifest = overlays.get(identity);
        let priority = manifest
            .and_then(|manifest| manifest.get("priority"))
            .and_then(Value::as_i64)
            .unwrap_or(500);
        let name = manifest
            .and_then(|manifest| manifest.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or(identity)
            .to_owned();
        (priority, name, identity.clone())
    };
    let mut sorted = active.to_vec();
    sorted.sort_by_cached_key(key);
    sorted
}
