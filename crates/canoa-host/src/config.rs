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

/// Connections an entry serves at once when the file does not say.
pub const DEFAULT_MAX_PEERS: usize = 4;
/// Sessions one connection to a provider may hold when the file does not say.
pub const DEFAULT_MAX_SESSIONS: usize = 8;
/// The longest entry name, which appears in logs and in what `serve` prints.
pub const MAX_NAME: usize = 64;

/// What the host serves: every entry of `[host.agents]` and `[host.providers]`, in the file's order, agents first.
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
    /// A Guatiao library offering an `agent_provider`, served through the ACP facade in a process of its own.
    Provider {
        /// The library file, absolute.
        library: PathBuf,
        /// The provider's id in the library.
        provider: String,
        /// Sessions one connection may hold.
        max_sessions: usize,
    },
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
    library: String,
    provider: String,
    max_sessions: Option<usize>,
    listen: Option<String>,
    #[serde(default)]
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

impl Config {
    /// Reads `path`. Relative library paths are taken from the file's directory.
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

    /// Parses Canoa.cfg's text, with relative library paths taken from `base`. Touches no file.
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
        for (name, provider) in host.providers {
            if provider.library.is_empty() || provider.provider.is_empty() {
                return Err(invalid(&name, "library and provider must not be empty"));
            }
            let library = base.join(&provider.library);
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
                library: std::path::absolute(&library).unwrap_or(library),
                provider: provider.provider,
                max_sessions,
            };
            entries.push(entry(name, kind, common)?);
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

fn entry(name: String, kind: EntryKind, common: RawCommon) -> Result<Entry, ConfigError> {
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
                return Err(invalid(
                    &entry.name,
                    "an agent and a provider share this name",
                ));
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

/// Absolute, with no `.` or `..`: such a path is compared as written, so one of those could step outside it.
pub(crate) fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
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
