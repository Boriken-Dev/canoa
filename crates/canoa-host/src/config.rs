//! The `[host]` table of Canoa.cfg: what the host serves, and to whom. The other tables belong to the app and are not read.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use coqui_peer_webcrypto::PeerId;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::tool::ToolSpec;

/// Connections an entry serves at once when the file does not say.
pub const DEFAULT_MAX_PEERS: usize = 4;
/// Sessions one connection to a provider may hold when the file does not say.
pub const DEFAULT_MAX_SESSIONS: usize = 8;
/// The longest entry name, which appears in logs and in what `serve` prints.
pub const MAX_NAME: usize = 64;

/// What the host serves: every entry of `[host.agents]`, `[host.providers]` and `[host.tools]`, in the file's order, agents
/// first, then providers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Browser origins that may connect; empty lets any connect, and the proof still decides.
    pub origins: Vec<String>,
    /// The entries.
    pub entries: Vec<Entry>,
}

/// One agent the host offers on an address of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// Its name in the file, unique across both tables.
    pub name: String,
    /// What runs for each connection.
    pub kind: EntryKind,
    /// A loopback address; port 0 takes any free port.
    pub listen: SocketAddr,
    /// Directories a session's `cwd` must be under. Empty takes any absolute path.
    pub working_roots: Vec<PathBuf>,
    /// The paired peers that may use it; `None` lets every paired peer.
    pub peers: Option<Vec<PeerId>>,
    /// Connections served at once; another is turned away while they run.
    pub max_peers: usize,
}

/// What an entry runs for each connection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryKind {
    /// A program speaking ACP on its standard input and output.
    Agent {
        /// The program, found as the operating system finds a command.
        program: OsString,
        /// Its arguments.
        args: Vec<OsString>,
    },
    /// A Guatiao `agent_provider`, from a library or a source, served through the ACP facade in a process of its own.
    Provider {
        /// Where its code comes from.
        origin: Origin,
        /// The provider's id.
        provider: String,
        /// The provider's configuration, checked against the schema it declares when a connection loads it.
        settings: Option<Settings>,
        /// The directory the provider's process runs in, absolute: Canoa.cfg's.
        directory: PathBuf,
        /// Sessions one connection may hold.
        max_sessions: usize,
    },
    /// A command-line tool with no ACP of its own, served as an ACP agent by the rules its entry gives.
    Tool {
        /// The program, how a prompt reaches it and what its events become.
        spec: crate::tool::ToolSpec,
    },
}

/// Where a provider's code comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// A Guatiao library file, absolute.
    Library(PathBuf),
    /// A Guatiao source file, absolute, such as `guatiao_source_py`, which serves libraries written in another language.
    Source {
        /// The source file.
        file: PathBuf,
        /// What the source is added with: for Python, the directories of its manifests and its import paths.
        settings: Settings,
        /// Where Windows looks for what the source imports itself (Python's `python3.dll`) that its own directory, the
        /// executable's and the system's leave missing.
        import_search: ImportSearch,
    },
}

/// Where Windows looks for an import a source names itself (Python's `python3.dll`) that the source's own directory, the
/// executable's and the system's leave missing: `import_search` on a source entry. Elsewhere the platform's loader
/// decides, and both are the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImportSearch {
    /// `library`: the source's `import_dirs` only.
    Library,
    /// `path`: `import_dirs`, then the Windows directory, the working directory and `PATH`, so whoever can write any of
    /// them chooses the code the source loads. A weakening, which `serve` logs.
    Path,
}

impl ImportSearch {
    /// `library` or `path`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Path => "path",
        }
    }

    /// The search a source entry takes when it names none: [`ImportSearch::Library`] when its `import_dirs` name at
    /// least one directory, since they say where to look; [`ImportSearch::Path`] otherwise, so a source that relied on
    /// `PATH` still loads, and is logged as weakened.
    #[must_use]
    pub fn default_for(settings: &Settings) -> Self {
        let listed = settings
            .as_map()
            .get("import_dirs")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|dirs| !dirs.is_empty());
        if listed { Self::Library } else { Self::Path }
    }

    /// The search `text` names: `library` or `path`.
    #[must_use]
    pub fn named(text: &str) -> Option<Self> {
        match text {
            "library" => Some(Self::Library),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}

/// A table of settings from Canoa.cfg, as JSON, the form a provider process receives it in. `Debug` shows its keys,
/// never its values, which may be secrets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Settings(serde_json::Map<String, serde_json::Value>);

impl Settings {
    /// Settings holding `map`.
    #[must_use]
    pub fn new(map: serde_json::Map<String, serde_json::Value>) -> Self {
        Self(map)
    }

    /// The table.
    #[must_use]
    pub fn as_map(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// Why Canoa.cfg could not be used. No variant quotes a value from the file, which may hold a secret.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file could not be read.
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The file is not TOML of the expected shape.
    Parse {
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
        /// What was wrong, in words of the schema's.
        what: String,
    },
    /// An entry, or `[host]` itself when `entry` is `None`, has a setting the host refuses.
    Invalid {
        /// The entry's name.
        entry: Option<String>,
        /// Which setting and why.
        what: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "could not read {}: {source}", path.display()),
            Self::Parse { line, column, what } => {
                write!(
                    f,
                    "the configuration is malformed at line {line}, column {column}: {what}"
                )
            }
            Self::Invalid {
                entry: Some(entry),
                what,
            } => write!(f, "host entry {entry}: {what}"),
            Self::Invalid { entry: None, what } => write!(f, "[host]: {what}"),
        }
    }
}

impl core::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    host: Option<RawHost>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHost {
    #[serde(default)]
    origins: Vec<String>,
    #[serde(default)]
    agents: IndexMap<String, RawAgent>,
    #[serde(default)]
    providers: IndexMap<String, RawProvider>,
    #[serde(default)]
    tools: IndexMap<String, toml::Table>,
}

/// The settings both tables share. serde does not hold `deny_unknown_fields` across `flatten`, so each table lists them.
struct RawCommon {
    listen: Option<String>,
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgent {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    listen: Option<String>,
    #[serde(default)]
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvider {
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    settings: Option<toml::Table>,
    provider: String,
    max_sessions: Option<usize>,
    listen: Option<String>,
    #[serde(default)]
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

impl Config {
    /// Reads `path`. Relative `library` and `source` paths are taken from the file's directory, where a provider runs.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] when the file cannot be read, and as [`Config::from_str_in`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let bytes = std::fs::read(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let text = core::str::from_utf8(&bytes).map_err(|error| {
            let (line, column) = position(&bytes, error.valid_up_to());
            ConfigError::Parse {
                line,
                column,
                what: "the file is not UTF-8".to_owned(),
            }
        })?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        Self::from_str_in(text, base)
    }

    /// Parses Canoa.cfg's text, with relative `library` and `source` paths taken from `base`, where a provider runs.
    /// Touches no file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] for text that is not TOML of this shape, with its position; [`ConfigError::Invalid`] for a
    /// setting the host refuses.
    pub fn from_str_in(text: &str, base: &Path) -> Result<Self, ConfigError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawFile = toml::from_str(text).map_err(|error| parse_error(text, &error))?;
        let Some(host) = raw.host else {
            return Ok(Self::default());
        };
        for origin in &host.origins {
            if origin.is_empty() || origin.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "an origin is empty or holds spaces or control characters".to_owned(),
                });
            }
        }
        let mut entries = Vec::new();
        for (name, agent) in host.agents {
            check_name(&name)?;
            if agent.program.is_empty() {
                return Err(invalid(&name, "program is empty"));
            }
            let common = RawCommon {
                listen: agent.listen,
                working_roots: agent.working_roots,
                peers: agent.peers,
                max_peers: agent.max_peers,
            };
            let kind = EntryKind::Agent {
                program: OsString::from(agent.program),
                args: agent.args.into_iter().map(OsString::from).collect(),
            };
            entries.push(entry(name, kind, common)?);
        }
        let directory = directory_of(base);
        for (name, provider) in host.providers {
            check_name(&name)?;
            if provider.provider.is_empty() {
                return Err(invalid(&name, "provider must not be empty"));
            }
            let file = |path: &str| {
                if path.is_empty() {
                    return Err(invalid(&name, "library or source must not be empty"));
                }
                let path = base.join(path);
                Ok(std::path::absolute(&path).unwrap_or(path))
            };
            let origin = match (
                &provider.library,
                &provider.source,
                provider.source_settings,
            ) {
                (Some(_), Some(_), _) => {
                    return Err(invalid(&name, "library and source exclude each other"));
                }
                (None, None, _) => {
                    return Err(invalid(&name, "a provider needs library or source"));
                }
                (Some(_), None, Some(_)) => {
                    return Err(invalid(
                        &name,
                        "source_settings go with source, not library",
                    ));
                }
                (Some(_), None, None) if provider.import_search.is_some() => {
                    return Err(invalid(
                        &name,
                        "import_search goes with source, not library",
                    ));
                }
                (Some(library), None, None) => Origin::Library(file(library)?),
                (None, Some(source), settings) => {
                    let mut settings = settings_of(&name, settings.unwrap_or_default())?;
                    absolute_paths(&name, &directory, &mut settings)?;
                    let import_search = match provider.import_search.as_deref() {
                        None => ImportSearch::default_for(&settings),
                        Some(text) => ImportSearch::named(text)
                            .ok_or_else(|| invalid(&name, "import_search is library or path"))?,
                    };
                    Origin::Source {
                        file: file(source)?,
                        settings,
                        import_search,
                    }
                }
            };
            let settings = provider
                .settings
                .map(|table| settings_of(&name, table))
                .transpose()?;
            let max_sessions = provider.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS);
            if max_sessions == 0 {
                return Err(invalid(&name, "max_sessions must be at least 1"));
            }
            let common = RawCommon {
                listen: provider.listen,
                working_roots: provider.working_roots,
                peers: provider.peers,
                max_peers: provider.max_peers,
            };
            let kind = EntryKind::Provider {
                origin,
                provider: provider.provider,
                settings,
                directory: directory.clone(),
                max_sessions,
            };
            entries.push(entry(name, kind, common)?);
        }
        for (name, table) in host.tools {
            check_name(&name)?;
            let (common, spec) = tool_of(&name, table)?;
            entries.push(entry(name, EntryKind::Tool { spec }, common)?);
        }
        check_unique(&entries)?;
        Ok(Self {
            origins: host.origins,
            entries,
        })
    }

    /// The entry called `name`.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

fn invalid(name: &str, what: &str) -> ConfigError {
    ConfigError::Invalid {
        entry: Some(name.to_owned()),
        what: what.to_owned(),
    }
}

/// `base` made absolute; an empty one is the current directory, as a path joined to it is taken from there.
fn directory_of(base: &Path) -> PathBuf {
    if base.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_default();
    }
    std::path::absolute(base).unwrap_or_else(|_| base.to_owned())
}

/// A TOML table as JSON. A date, which JSON has no type for, and a float that is not a number are refused.
fn settings_of(name: &str, table: toml::Table) -> Result<Settings, ConfigError> {
    object_of(table).map(Settings).ok_or_else(|| {
        invalid(
            name,
            "a setting holds a date, or a float that is not a number, which a provider cannot be given",
        )
    })
}

/// The source settings read as lists of paths: `dirs` and `manifests` by every Guatiao bridge source, `paths` by Python's,
/// `import_dirs` by the registry. A source may resolve a relative one against whatever working directory it has later, or
/// against its own file's, so each is made absolute here.
const PATH_LISTS: [&str; 4] = ["dirs", "manifests", "paths", "import_dirs"];

/// The source settings read as one path: `home`, the Python installation Python's source runs from.
const PATHS: [&str; 1] = ["home"];

fn absolute_paths(
    name: &str,
    directory: &Path,
    settings: &mut Settings,
) -> Result<(), ConfigError> {
    let absolute = |text: &mut String| -> Result<(), ConfigError> {
        let path = directory.join(&*text);
        let path = std::path::absolute(&path).unwrap_or(path);
        *text = path
            .to_str()
            .ok_or_else(|| invalid(name, "Canoa.cfg's directory is not Unicode, so a relative path cannot be made absolute"))?
            .to_owned();
        Ok(())
    };
    for key in PATH_LISTS {
        let Some(serde_json::Value::Array(items)) = settings.0.get_mut(key) else {
            continue;
        };
        for item in items {
            if let serde_json::Value::String(text) = item {
                absolute(text)?;
            }
        }
    }
    for key in PATHS {
        if let Some(serde_json::Value::String(text)) = settings.0.get_mut(key) {
            absolute(text)?;
        }
    }
    Ok(())
}

fn object_of(table: toml::Table) -> Option<serde_json::Map<String, serde_json::Value>> {
    table
        .into_iter()
        .map(|(key, value)| Some((key, json_of(value)?)))
        .collect()
}

fn json_of(value: toml::Value) -> Option<serde_json::Value> {
    use serde_json::Value as Json;
    Some(match value {
        toml::Value::String(text) => Json::String(text),
        toml::Value::Integer(number) => Json::from(number),
        toml::Value::Float(number) => Json::Number(serde_json::Number::from_f64(number)?),
        toml::Value::Boolean(flag) => Json::Bool(flag),
        toml::Value::Datetime(_) => return None,
        toml::Value::Array(items) => {
            Json::Array(items.into_iter().map(json_of).collect::<Option<_>>()?)
        }
        toml::Value::Table(table) => Json::Object(object_of(table)?),
    })
}

/// A tool entry: the settings every entry has, taken out by hand since serde does not hold `deny_unknown_fields` across
/// `flatten`, then the rest as the tool's. Errors name a setting, never a value.
fn tool_of(name: &str, mut table: toml::Table) -> Result<(RawCommon, ToolSpec), ConfigError> {
    let wrong = |key: &str| invalid(name, &format!("{key} has the wrong type"));
    let text = |value: toml::Value, key: &str| match value {
        toml::Value::String(text) => Ok(text),
        _ => Err(wrong(key)),
    };
    let texts = |value: toml::Value, key: &str| match value {
        toml::Value::Array(items) => items.into_iter().map(|item| text(item, key)).collect(),
        _ => Err(wrong(key)),
    };
    let listen = table
        .remove("listen")
        .map(|value| text(value, "listen"))
        .transpose()?;
    let working_roots = table
        .remove("working_roots")
        .map(|value| texts(value, "working_roots"))
        .transpose()?
        .unwrap_or_default();
    let peers = table
        .remove("peers")
        .map(|value| texts(value, "peers"))
        .transpose()?;
    let max_peers = match table.remove("max_peers") {
        None => None,
        Some(toml::Value::Integer(count)) => Some(
            usize::try_from(count).map_err(|_| invalid(name, "max_peers must be at least 1"))?,
        ),
        Some(_) => return Err(wrong("max_peers")),
    };
    let spec: ToolSpec =
        toml::Value::Table(table)
            .try_into()
            .map_err(|error: toml::de::Error| {
                let message = error.message();
                let what = if message.starts_with("missing field") {
                    message.to_string()
                } else if message.starts_with("unknown field") {
                    "a setting this host does not know".to_owned()
                } else if message.starts_with("unknown variant") {
                    "a setting has a value this host does not know".to_owned()
                } else {
                    "a setting has the wrong type".to_owned()
                };
                invalid(name, &what)
            })?;
    spec.check().map_err(|what| invalid(name, &what))?;
    let common = RawCommon {
        listen,
        working_roots,
        peers,
        max_peers,
    };
    Ok((common, spec))
}

/// Checked before anything quotes the name, since a key in the file may hold any character.
fn check_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "an entry name has 1 to {MAX_NAME} letters, digits, hyphens or underscores"
            ),
        });
    }
    Ok(())
}

fn entry(name: String, kind: EntryKind, common: RawCommon) -> Result<Entry, ConfigError> {
    let listen = match common.listen {
        None => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Some(text) => text
            .parse::<SocketAddr>()
            .map_err(|_| invalid(&name, "listen is not an address and port"))?,
    };
    if !listen.ip().is_loopback() {
        return Err(invalid(
            &name,
            "listen must be a loopback address: this host serves this machine only, and a proxy that ends TLS can carry it further",
        ));
    }
    let mut working_roots = Vec::new();
    for root in common.working_roots {
        let root = PathBuf::from(root);
        if !plain_absolute(&root) {
            return Err(invalid(
                &name,
                "a working root must be an absolute path with no . or .. in it",
            ));
        }
        working_roots.push(root);
    }
    let peers = common
        .peers
        .map(|peers| {
            peers
                .iter()
                .map(|peer| peer.parse::<PeerId>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid(&name, "a peer is not a peer id"))
        })
        .transpose()?;
    let max_peers = common.max_peers.unwrap_or(DEFAULT_MAX_PEERS);
    if max_peers == 0 {
        return Err(invalid(&name, "max_peers must be at least 1"));
    }
    Ok(Entry {
        name,
        kind,
        listen,
        working_roots,
        peers,
        max_peers,
    })
}

fn check_unique(entries: &[Entry]) -> Result<(), ConfigError> {
    for (index, entry) in entries.iter().enumerate() {
        for earlier in &entries[..index] {
            if earlier.name == entry.name {
                return Err(invalid(&entry.name, "two entries share this name"));
            }
            if entry.listen.port() != 0 && earlier.listen == entry.listen {
                return Err(invalid(
                    &entry.name,
                    "another entry listens on the same address",
                ));
            }
        }
    }
    Ok(())
}

/// Absolute, with no `.` or `..`: such a path is compared as written, so one of those could step outside it. The text is
/// checked as well as the components, since a Unix path's components drop a `.` in the middle.
pub(crate) fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
        && !path
            .to_string_lossy()
            .split(['/', '\\'])
            .any(|segment| segment == "." || segment == "..")
}

/// The 1-based line and column of byte `offset`.
fn position(text: &[u8], offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let column = String::from_utf8_lossy(&before[start..]).chars().count() + 1;
    (line, column)
}

/// The parser's position, and its words only where they come from the schema: a value, or a key nobody declared, is never told.
fn parse_error(text: &str, error: &toml::de::Error) -> ConfigError {
    let (line, column) = error
        .span()
        .map_or((1, 1), |span| position(text.as_bytes(), span.start));
    let message = error.message();
    let what = if message.starts_with("missing field") {
        message.to_string()
    } else if message.starts_with("unknown field") {
        "a setting this host does not know".to_owned()
    } else if message.starts_with("invalid type") {
        "a setting has the wrong type".to_owned()
    } else if message.starts_with("duplicate key") {
        "a key is given twice".to_owned()
    } else {
        "not TOML of the expected shape".to_owned()
    };
    ConfigError::Parse { line, column, what }
}

#[cfg(test)]
mod tests;
