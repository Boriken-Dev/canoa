//! Profiles: the environment, initial context, skills, MCP servers and defaults an agent is launched with, in layers.
//!
//! A profile document is TOML, the same in a file and under a table of Canoa.cfg. Layers merge lowest first: a later layer
//! wins per key, named entries merge by name, and a layer can remove what a lower one set. The effective profile keeps the
//! layer of every value. The app reads the same schema; `tests/vectors/profile.json` holds both to it.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The longest skill, MCP server, header or overlay name.
pub const MAX_NAME: usize = 64;
/// The longest environment variable name.
pub const MAX_ENV_NAME: usize = 128;

/// Where a layer comes from, lowest first. The order of a merge is the caller's; this is the order Canoa uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LayerKind {
    /// Canoa's own defaults.
    BuiltIn,
    /// A named overlay, from a git source or a local directory.
    Overlay,
    /// The person's own, in Canoa.cfg.
    User,
    /// A file in the working directory, used only once the person trusts that directory.
    Project,
    /// The agent's entry.
    Agent,
    /// The session being started.
    Session,
}

impl LayerKind {
    /// The kind as the view writes it: `built-in`, `overlay`, `user`, `project`, `agent` or `session`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Overlay => "overlay",
            Self::User => "user",
            Self::Project => "project",
            Self::Agent => "agent",
            Self::Session => "session",
        }
    }

    /// Whether the layer comes from elsewhere, so its paths must stay inside its own directory.
    #[must_use]
    pub const fn is_confined(self) -> bool {
        matches!(self, Self::Overlay | Self::Project)
    }
}

/// One layer's name in the view: its kind, then `:` and its name for an overlay or an agent (`overlay:team`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerId {
    /// Its kind.
    pub kind: LayerKind,
    /// The overlay's or agent's name; empty for the other kinds.
    pub name: String,
}

impl LayerId {
    /// A layer of `kind` called `name`.
    #[must_use]
    pub fn new(kind: LayerKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.name.is_empty() {
            f.write_str(self.kind.as_str())
        } else {
            write!(f, "{}:{}", self.kind.as_str(), self.name)
        }
    }
}

/// A value an environment variable or a header takes. `from_env` and `from_file` are references, resolved only when a
/// session starts; a literal `value` is for what is not secret.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvValue {
    /// Written in the profile.
    Value(String),
    /// The value of this process's environment variable of that name.
    FromEnv(String),
    /// The contents of a file, relative to its layer's directory, without one final line break.
    FromFile(String),
}

impl fmt::Debug for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(_) => f.write_str("Value(..)"),
            Self::FromEnv(name) => write!(f, "FromEnv({name:?})"),
            Self::FromFile(path) => write!(f, "FromFile({path:?})"),
        }
    }
}

/// The initial context: text, or a file relative to its layer's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Context {
    /// The text itself.
    Text(String),
    /// A file holding it.
    File(String),
}

/// A skill: a directory holding a `SKILL.md`, relative to its layer's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Skill {
    /// The directory.
    pub path: String,
}

impl Skill {
    /// The skill in `path`.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// An MCP server, in the forms ACP's `mcpServers` takes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpServer {
    /// A program the agent starts, speaking MCP on its standard streams.
    Stdio {
        /// The program.
        command: String,
        /// Its arguments.
        args: Vec<String>,
        /// Its environment, beyond what the agent passes on.
        env: IndexMap<String, EnvValue>,
    },
    /// Streamable HTTP.
    Http {
        /// `http://` or `https://`.
        url: String,
        /// Headers sent with each request.
        headers: IndexMap<String, EnvValue>,
    },
    /// HTTP with server-sent events.
    Sse {
        /// `http://` or `https://`.
        url: String,
        /// Headers sent with each request.
        headers: IndexMap<String, EnvValue>,
    },
}

/// What a layer does to a named entry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change<T> {
    /// Sets it, replacing a lower layer's whole.
    Set(T),
    /// Removes a lower layer's.
    Remove,
}

/// A value a layer can take back with `unset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Scalar {
    /// `model`.
    Model,
    /// `effort`.
    Effort,
    /// `mode`.
    Mode,
    /// `context`.
    Context,
}

impl Scalar {
    const ALL: [Self; 4] = [Self::Model, Self::Effort, Self::Mode, Self::Context];

    /// Its key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Effort => "effort",
            Self::Mode => "mode",
            Self::Context => "context",
        }
    }
}

/// One profile document: what one layer sets and removes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
    /// The model the agent starts with.
    pub model: Option<String>,
    /// The effort, or thought level, it starts with.
    pub effort: Option<String>,
    /// The mode it starts in.
    pub mode: Option<String>,
    /// The initial context.
    pub context: Option<Context>,
    /// Values of lower layers this one takes back.
    pub unset: Vec<Scalar>,
    /// Environment variables, by name.
    pub env: IndexMap<String, Change<EnvValue>>,
    /// Skills, by name.
    pub skills: IndexMap<String, Change<Skill>>,
    /// MCP servers, by name.
    pub mcp: IndexMap<String, Change<McpServer>>,
}

/// One layer: a profile, where it comes from, and the directory its relative paths are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Layer {
    /// Its name in the view.
    pub id: LayerId,
    /// The directory its relative paths are read from; `None` reads them from nowhere, and resolving one fails.
    pub dir: Option<PathBuf>,
    /// What it sets.
    pub profile: Profile,
}

impl Layer {
    /// A layer holding `profile`.
    #[must_use]
    pub fn new(id: LayerId, dir: Option<PathBuf>, profile: Profile) -> Self {
        Self { id, dir, profile }
    }
}

/// Why a profile document was refused, or a reference could not be resolved. Nothing here quotes a value: a profile may
/// hold one that should not be shown, and a reference resolves to a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// Not TOML, or not of the profile's shape: a key no profile has, a key twice, a value of the wrong type.
    Parse {
        /// 1-based line, when the parser gives one.
        line: usize,
        /// 1-based column.
        column: usize,
    },
    /// A setting the schema refuses.
    Invalid {
        /// The key, such as `env.GITHUB_TOKEN` or `mcp.files`; for a name the table holding it, since a refused name may
        /// hold anything.
        key: String,
        /// Which rule.
        rule: Rule,
    },
    /// A reference could not be resolved when a session started.
    Resolve {
        /// The variable or header.
        key: String,
        /// Why, never with the value.
        what: &'static str,
    },
}

/// The rule a setting broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rule {
    /// A name outside its characters or length.
    Name,
    /// A path that is empty or holds NUL, or in an overlay or project layer one that is absolute or steps out of its
    /// directory.
    Path,
    /// Not exactly one of the forms a setting takes, or a value both set and unset.
    Form,
    /// A value that is empty or holds a control character, or an MCP URL that is not HTTP.
    Value,
}

impl ProfileError {
    /// A stable code: `parse`, `name`, `path`, `form`, `value` or `resolve`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Parse { .. } => "parse",
            Self::Invalid { rule, .. } => match rule {
                Rule::Name => "name",
                Rule::Path => "path",
                Rule::Form => "form",
                Rule::Value => "value",
            },
            Self::Resolve { .. } => "resolve",
        }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, column } => write!(
                f,
                "the profile is not TOML of a profile's shape at line {line}, column {column}"
            ),
            Self::Invalid { key, rule } => {
                let what = match rule {
                    Rule::Name => "a name has characters or a length the schema refuses",
                    Rule::Path => {
                        "a path is empty, or leaves the directory of a layer that comes from elsewhere"
                    }
                    Rule::Form => "takes exactly one of its forms",
                    Rule::Value => {
                        "a value is empty, holds a control character, or is not an HTTP URL"
                    }
                };
                write!(f, "profile {key}: {what}")
            }
            Self::Resolve { key, what } => write!(f, "profile {key}: {what}"),
        }
    }
}

impl core::error::Error for ProfileError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    model: Option<String>,
    effort: Option<String>,
    mode: Option<String>,
    context: Option<RawContext>,
    #[serde(default)]
    unset: Vec<String>,
    #[serde(default)]
    env: IndexMap<String, RawEnv>,
    #[serde(default)]
    skills: IndexMap<String, RawSkill>,
    #[serde(default)]
    mcp: IndexMap<String, RawMcp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContext {
    text: Option<String>,
    file: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    value: Option<String>,
    from_env: Option<String>,
    from_file: Option<String>,
    remove: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSkill {
    path: Option<String>,
    remove: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    #[serde(rename = "type")]
    kind: Option<String>,
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<IndexMap<String, RawEnv>>,
    url: Option<String>,
    headers: Option<IndexMap<String, RawEnv>>,
    remove: Option<bool>,
}

impl Profile {
    /// Parses a profile document for a layer of `kind`, whose kind decides whether its paths must stay in its directory.
    /// Touches no file and reads no variable.
    ///
    /// # Errors
    ///
    /// [`ProfileError::Parse`] for text that is not TOML of a profile's shape; [`ProfileError::Invalid`] for a setting the
    /// schema refuses.
    pub fn from_str(text: &str, kind: LayerKind) -> Result<Self, ProfileError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawProfile = toml::from_str(text).map_err(|error| {
            let (line, column) = error
                .span()
                .map_or((1, 1), |span| position(text, span.start));
            ProfileError::Parse { line, column }
        })?;
        Self::checked(raw, kind)
    }

    /// Reads a profile from a TOML table already parsed, such as Canoa.cfg's `[profile]`.
    ///
    /// # Errors
    ///
    /// As [`Profile::from_str`]; a parse error has no position.
    pub fn from_table(table: toml::Table, kind: LayerKind) -> Result<Self, ProfileError> {
        let raw = RawProfile::deserialize(toml::Value::Table(table))
            .map_err(|_| ProfileError::Parse { line: 1, column: 1 })?;
        Self::checked(raw, kind)
    }

    fn checked(raw: RawProfile, kind: LayerKind) -> Result<Self, ProfileError> {
        let confined = kind.is_confined();
        let mut profile = Self::default();
        for (key, value, slot) in [
            ("model", raw.model, &mut profile.model),
            ("effort", raw.effort, &mut profile.effort),
            ("mode", raw.mode, &mut profile.mode),
        ] {
            if let Some(value) = value {
                if !plain(&value) {
                    return Err(invalid(key, Rule::Value));
                }
                *slot = Some(value);
            }
        }
        if let Some(context) = raw.context {
            profile.context = Some(match (context.text, context.file) {
                (Some(text), None) => {
                    if text.is_empty() || text.contains('\0') {
                        return Err(invalid("context", Rule::Value));
                    }
                    Context::Text(text)
                }
                (None, Some(file)) => Context::File(checked_path("context", file, confined)?),
                _ => return Err(invalid("context", Rule::Form)),
            });
        }
        for name in raw.unset {
            let scalar = Scalar::ALL
                .into_iter()
                .find(|scalar| scalar.as_str() == name)
                .ok_or_else(|| invalid("unset", Rule::Form))?;
            let set = match scalar {
                Scalar::Model => profile.model.is_some(),
                Scalar::Effort => profile.effort.is_some(),
                Scalar::Mode => profile.mode.is_some(),
                Scalar::Context => profile.context.is_some(),
            };
            if set {
                return Err(invalid(scalar.as_str(), Rule::Form));
            }
            if !profile.unset.contains(&scalar) {
                profile.unset.push(scalar);
            }
        }
        for (name, raw) in raw.env {
            if !env_name(&name) {
                return Err(invalid("env", Rule::Name));
            }
            let key = format!("env.{name}");
            let change = if raw.remove.is_some() {
                removal(
                    &key,
                    raw.remove,
                    [
                        raw.value.is_some(),
                        raw.from_env.is_some(),
                        raw.from_file.is_some(),
                    ],
                )?
            } else {
                Change::Set(env_value(&key, raw, confined)?)
            };
            profile.env.insert(name, change);
        }
        for (name, raw) in raw.skills {
            if !entry_name(&name) {
                return Err(invalid("skills", Rule::Name));
            }
            let key = format!("skills.{name}");
            let change = match (raw.remove, raw.path) {
                (Some(_), path) => removal(&key, raw.remove, [path.is_some()])?,
                (None, Some(path)) => Change::Set(Skill::new(checked_path(&key, path, confined)?)),
                (None, None) => return Err(invalid(&key, Rule::Form)),
            };
            profile.skills.insert(name, change);
        }
        for (name, raw) in raw.mcp {
            if !entry_name(&name) {
                return Err(invalid("mcp", Rule::Name));
            }
            let key = format!("mcp.{name}");
            let change = if raw.remove.is_some() {
                removal(
                    &key,
                    raw.remove,
                    [
                        raw.kind.is_some(),
                        raw.command.is_some(),
                        raw.args.is_some(),
                        raw.env.is_some(),
                        raw.url.is_some(),
                        raw.headers.is_some(),
                    ],
                )?
            } else {
                Change::Set(mcp_server(&key, raw, confined)?)
            };
            profile.mcp.insert(name, change);
        }
        Ok(profile)
    }
}

fn invalid(key: &str, rule: Rule) -> ProfileError {
    ProfileError::Invalid {
        key: key.to_owned(),
        rule,
    }
}

/// `remove = true` alone; `others` says which other settings the entry has.
fn removal<T, const N: usize>(
    key: &str,
    remove: Option<bool>,
    others: [bool; N],
) -> Result<Change<T>, ProfileError> {
    if remove == Some(true) && !others.contains(&true) {
        Ok(Change::Remove)
    } else {
        Err(invalid(key, Rule::Form))
    }
}

fn env_value(key: &str, raw: RawEnv, confined: bool) -> Result<EnvValue, ProfileError> {
    match (raw.value, raw.from_env, raw.from_file) {
        (Some(value), None, None) => {
            if value.contains('\0') {
                return Err(invalid(key, Rule::Value));
            }
            Ok(EnvValue::Value(value))
        }
        (None, Some(name), None) => {
            if !env_name(&name) {
                return Err(invalid(key, Rule::Name));
            }
            Ok(EnvValue::FromEnv(name))
        }
        (None, None, Some(path)) => Ok(EnvValue::FromFile(checked_path(key, path, confined)?)),
        _ => Err(invalid(key, Rule::Form)),
    }
}

fn values(
    key: &str,
    table: Option<IndexMap<String, RawEnv>>,
    names: fn(&str) -> bool,
    confined: bool,
) -> Result<IndexMap<String, EnvValue>, ProfileError> {
    let mut out = IndexMap::new();
    for (name, raw) in table.unwrap_or_default() {
        if !names(&name) {
            return Err(invalid(key, Rule::Name));
        }
        let entry = format!("{key}.{name}");
        if raw.remove.is_some() {
            return Err(invalid(&entry, Rule::Form));
        }
        out.insert(name, env_value(&entry, raw, confined)?);
    }
    Ok(out)
}

fn mcp_server(key: &str, raw: RawMcp, confined: bool) -> Result<McpServer, ProfileError> {
    let form = || invalid(key, Rule::Form);
    match raw.kind.as_deref() {
        Some("stdio") => {
            if raw.url.is_some() || raw.headers.is_some() {
                return Err(form());
            }
            let command = raw.command.ok_or_else(form)?;
            let args = raw.args.unwrap_or_default();
            if !plain(&command) || args.iter().any(|arg| arg.contains('\0')) {
                return Err(invalid(key, Rule::Value));
            }
            let env = values(&format!("{key}.env"), raw.env, env_name, confined)?;
            Ok(McpServer::Stdio { command, args, env })
        }
        Some(kind @ ("http" | "sse")) => {
            if raw.command.is_some() || raw.args.is_some() || raw.env.is_some() {
                return Err(form());
            }
            let url = raw.url.ok_or_else(form)?;
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || url.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(invalid(key, Rule::Value));
            }
            let headers = values(&format!("{key}.headers"), raw.headers, entry_name, confined)?;
            Ok(if kind == "http" {
                McpServer::Http { url, headers }
            } else {
                McpServer::Sse { url, headers }
            })
        }
        _ => Err(form()),
    }
}

/// Not empty, with no control character.
fn plain(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(char::is_control)
}

/// 1 to [`MAX_NAME`] letters, digits, `-` or `_`.
fn entry_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// A letter or `_`, then letters, digits or `_`, up to [`MAX_ENV_NAME`].
fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && name.len() <= MAX_ENV_NAME
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn checked_path(key: &str, path: String, confined: bool) -> Result<String, ProfileError> {
    if path_allowed(&path, confined) {
        Ok(path)
    } else {
        Err(invalid(key, Rule::Path))
    }
}

/// Judged on the text, the same on every platform. A confined path is relative and stays below its directory: no root,
/// drive, home or `..`, and no `:`, which on Windows names a drive or a file's stream.
pub(crate) fn path_allowed(path: &str, confined: bool) -> bool {
    if path.is_empty() || path.contains('\0') {
        return false;
    }
    if !confined {
        return true;
    }
    !path.starts_with(['/', '\\', '~'])
        && !path.contains(':')
        && !path.split(['/', '\\']).any(|segment| segment == "..")
}

/// A value and the layer it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Sourced<T> {
    /// The value.
    pub value: T,
    /// The layer that set it.
    pub layer: LayerId,
    /// That layer's directory, which its relative paths are read from.
    pub dir: Option<PathBuf>,
}

/// An entry a layer removed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Removed {
    /// `model`, `context`, `env.NAME`, `skills.NAME` or `mcp.NAME`.
    pub key: String,
    /// The layer that removed it.
    pub layer: LayerId,
    /// The layer that had set it.
    pub was: LayerId,
}

/// The profile every layer adds up to, each value with its layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Effective {
    /// The model.
    pub model: Option<Sourced<String>>,
    /// The effort.
    pub effort: Option<Sourced<String>>,
    /// The mode.
    pub mode: Option<Sourced<String>>,
    /// The initial context.
    pub context: Option<Sourced<Context>>,
    /// Environment variables, in the order they were first set.
    pub env: IndexMap<String, Sourced<EnvValue>>,
    /// Skills, likewise.
    pub skills: IndexMap<String, Sourced<Skill>>,
    /// MCP servers, likewise.
    pub mcp: IndexMap<String, Sourced<McpServer>>,
    /// What layers removed, in the order they did.
    pub removed: Vec<Removed>,
}

/// Merges `layers`, lowest first. A later layer's value replaces a lower one's; a named entry is replaced whole and keeps
/// its place; an entry removed and set again later comes last.
#[must_use]
pub fn merge(layers: &[Layer]) -> Effective {
    let mut out = Effective::default();
    for layer in layers {
        let sourced = |value| Sourced {
            value,
            layer: layer.id.clone(),
            dir: layer.dir.clone(),
        };
        let profile = &layer.profile;
        for scalar in &profile.unset {
            let was = match scalar {
                Scalar::Model => out.model.take().map(|value| value.layer),
                Scalar::Effort => out.effort.take().map(|value| value.layer),
                Scalar::Mode => out.mode.take().map(|value| value.layer),
                Scalar::Context => out.context.take().map(|value| value.layer),
            };
            if let Some(was) = was {
                out.removed.push(Removed {
                    key: scalar.as_str().to_owned(),
                    layer: layer.id.clone(),
                    was,
                });
            }
        }
        if let Some(model) = &profile.model {
            out.model = Some(sourced(model.clone()));
        }
        if let Some(effort) = &profile.effort {
            out.effort = Some(sourced(effort.clone()));
        }
        if let Some(mode) = &profile.mode {
            out.mode = Some(sourced(mode.clone()));
        }
        if let Some(context) = &profile.context {
            out.context = Some(Sourced {
                value: context.clone(),
                layer: layer.id.clone(),
                dir: layer.dir.clone(),
            });
        }
        merge_entries(&mut out.env, &profile.env, layer, "env", &mut out.removed);
        merge_entries(
            &mut out.skills,
            &profile.skills,
            layer,
            "skills",
            &mut out.removed,
        );
        merge_entries(&mut out.mcp, &profile.mcp, layer, "mcp", &mut out.removed);
    }
    out
}

fn merge_entries<T: Clone>(
    into: &mut IndexMap<String, Sourced<T>>,
    changes: &IndexMap<String, Change<T>>,
    layer: &Layer,
    table: &str,
    removed: &mut Vec<Removed>,
) {
    for (name, change) in changes {
        match change {
            Change::Set(value) => {
                let value = Sourced {
                    value: value.clone(),
                    layer: layer.id.clone(),
                    dir: layer.dir.clone(),
                };
                if let Some(slot) = into.get_mut(name) {
                    *slot = value;
                } else {
                    into.insert(name.clone(), value);
                }
            }
            Change::Remove => {
                if let Some(was) = into.shift_remove(name) {
                    removed.push(Removed {
                        key: format!("{table}.{name}"),
                        layer: layer.id.clone(),
                        was: was.layer,
                    });
                }
            }
        }
    }
}

impl Effective {
    /// The view the app shows and `tests/vectors/profile.json` holds: every value with its layer, references as written
    /// and never resolved.
    #[must_use]
    pub fn view(&self) -> Value {
        let mut out = Map::new();
        for (key, value) in [
            ("model", &self.model),
            ("effort", &self.effort),
            ("mode", &self.mode),
        ] {
            if let Some(value) = value {
                out.insert(
                    key.to_owned(),
                    json!({"value": value.value, "layer": value.layer.to_string()}),
                );
            }
        }
        if let Some(context) = &self.context {
            let value = match &context.value {
                Context::Text(text) => json!({"text": text}),
                Context::File(file) => json!({"file": file}),
            };
            out.insert(
                "context".to_owned(),
                json!({"value": value, "layer": context.layer.to_string()}),
            );
        }
        out.insert("env".to_owned(), entries(&self.env, env_json));
        out.insert(
            "skills".to_owned(),
            entries(&self.skills, |skill| json!({"path": skill.path})),
        );
        out.insert("mcp".to_owned(), entries(&self.mcp, mcp_json));
        out.insert(
            "removed".to_owned(),
            Value::Array(
                self.removed
                    .iter()
                    .map(|removed| {
                        json!({"key": removed.key, "layer": removed.layer.to_string(), "was": removed.was.to_string()})
                    })
                    .collect(),
            ),
        );
        Value::Object(out)
    }

    /// The environment variables, each reference resolved: what a session starts with. `lookup` reads this process's
    /// environment, or what stands for it. Call it only when a session starts, and keep the result out of every log.
    ///
    /// # Errors
    ///
    /// [`ProfileError::Resolve`] for a variable that is not set, or a file that cannot be read as UTF-8 or whose layer
    /// has no directory for a relative path.
    pub fn environment(
        &self,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<(String, Secret)>, ProfileError> {
        self.env
            .iter()
            .map(|(name, value)| {
                let key = format!("env.{name}");
                resolve(
                    &key,
                    &value.value,
                    value.dir.as_deref(),
                    value.layer.kind.is_confined(),
                    &lookup,
                )
                .map(|secret| (name.clone(), secret))
            })
            .collect()
    }
}

fn entries<T>(map: &IndexMap<String, Sourced<T>>, value: impl Fn(&T) -> Value) -> Value {
    Value::Array(
        map.iter()
            .map(|(name, entry)| json!({"name": name, "value": value(&entry.value), "layer": entry.layer.to_string()}))
            .collect(),
    )
}

fn env_json(value: &EnvValue) -> Value {
    match value {
        EnvValue::Value(text) => json!({"value": text}),
        EnvValue::FromEnv(name) => json!({"from_env": name}),
        EnvValue::FromFile(path) => json!({"from_file": path}),
    }
}

fn env_map(map: &IndexMap<String, EnvValue>) -> Value {
    Value::Object(
        map.iter()
            .map(|(name, value)| (name.clone(), env_json(value)))
            .collect(),
    )
}

fn mcp_json(server: &McpServer) -> Value {
    match server {
        McpServer::Stdio { command, args, env } => {
            json!({"type": "stdio", "command": command, "args": args, "env": env_map(env)})
        }
        McpServer::Http { url, headers } => {
            json!({"type": "http", "url": url, "headers": env_map(headers)})
        }
        McpServer::Sse { url, headers } => {
            json!({"type": "sse", "url": url, "headers": env_map(headers)})
        }
    }
}

/// A resolved value. `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// The value, for the process being started and nothing else.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(hidden)")
    }
}

/// Resolves one value: a literal as written, a variable through `lookup`, a file from `dir` without one final line break.
/// A `confined` layer's file must stay inside `dir` once its links are followed.
///
/// # Errors
///
/// [`ProfileError::Resolve`], naming `key` and never the value.
pub fn resolve(
    key: &str,
    value: &EnvValue,
    dir: Option<&Path>,
    confined: bool,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Secret, ProfileError> {
    let fail = |what| ProfileError::Resolve {
        key: key.to_owned(),
        what,
    };
    match value {
        EnvValue::Value(text) => Ok(Secret(text.clone())),
        EnvValue::FromEnv(name) => lookup(name)
            .map(Secret)
            .ok_or_else(|| fail("the variable it names is not set")),
        EnvValue::FromFile(path) => {
            let path = Path::new(path);
            let path = if path.is_absolute() && !confined {
                path.to_owned()
            } else {
                dir.ok_or_else(|| fail("a relative file in a layer with no directory"))?
                    .join(path)
            };
            if confined {
                let inside = dir
                    .and_then(|dir| dir.canonicalize().ok())
                    .zip(path.canonicalize().ok())
                    .is_some_and(|(dir, path)| path.starts_with(dir));
                if !inside {
                    return Err(fail(
                        "the file it names is outside its layer's directory once links are followed",
                    ));
                }
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|_| fail("the file it names cannot be read as UTF-8 text"))?;
            let text = text.strip_suffix('\n').map_or(text.as_str(), |rest| {
                rest.strip_suffix('\r').unwrap_or(rest)
            });
            Ok(Secret(text.to_owned()))
        }
    }
}

/// Where a named overlay comes from: `<repository>@<ref>#<path>`, as dotagents writes it, or a local directory.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverlaySource {
    /// A git repository at a ref, and the overlay's directory inside it (empty for its root).
    Git {
        /// A URL (`https`, `http`, `ssh`, `git`, `file`), an scp-like `user@host:path`, or a local path.
        repo: String,
        /// A branch, a tag or a commit.
        reference: String,
        /// The directory, `/`-separated, relative.
        path: String,
    },
    /// A directory on this computer, read in place.
    Dir(PathBuf),
}

impl OverlaySource {
    /// Parses `<repository>@<ref>[#<path>]`. The ref is what follows the last `@`, the path what follows the last `#`.
    ///
    /// # Errors
    ///
    /// [`ProfileError::Invalid`] with key `overlays` for a source with no ref; a repository that opens with `-`, holds a
    /// space or control character, names a transport helper (`ext::`) or a scheme other than those above; a ref git would
    /// refuse; or a path that is empty after `#`, absolute, or steps out with `..`.
    pub fn parse_git(source: &str) -> Result<Self, ProfileError> {
        let refused = || invalid("overlays", Rule::Value);
        let (rest, path) = match source.rsplit_once('#') {
            Some((rest, path)) => {
                if path.is_empty()
                    || path.starts_with('/')
                    || path.contains(['\\', ':', '\0'])
                    || path.split('/').any(|segment| segment == "..")
                {
                    return Err(refused());
                }
                (rest, path)
            }
            None => (source, ""),
        };
        let (repo, reference) = rest.rsplit_once('@').ok_or_else(refused)?;
        if repo.is_empty()
            || repo.starts_with('-')
            || repo.contains("::")
            || repo.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(refused());
        }
        if let Some((scheme, _)) = repo.split_once("://")
            && !["https", "http", "ssh", "git", "file"].contains(&scheme)
        {
            return Err(refused());
        }
        if !git_ref(reference) {
            return Err(refused());
        }
        Ok(Self::Git {
            repo: repo.to_owned(),
            reference: reference.to_owned(),
            path: path.trim_end_matches('/').to_owned(),
        })
    }
}

/// The ref names `git check-ref-format` takes, without its options, plus a commit id; never one that opens with `-`.
fn git_ref(reference: &str) -> bool {
    !reference.is_empty()
        && !reference.starts_with(['-', '/', '.'])
        && !reference.ends_with(['/', '.'])
        && !reference.ends_with(".lock")
        && !reference.contains("..")
        && !reference.contains("//")
        && !reference.contains("@{")
        && !reference.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, ':' | '~' | '^' | '?' | '*' | '[' | '\\')
        })
}

/// 1-based line and column of byte `offset`.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset.min(text.len())).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

#[cfg(test)]
mod tests;
