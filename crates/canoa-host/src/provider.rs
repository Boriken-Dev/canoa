//! One Guatiao provider, served as an ACP agent on standard input and output: what the host runs for each connection to a
//! provider entry, so the library or source is mapped in a process of its own and ends with the connection.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;
use std::io::{BufRead, Read as _, Write};
use std::net::TcpStream;
use std::path::PathBuf;

use coqui_acp::lines;
use coqui_acp_provider::guatiao::library::{ImportSearch, Registry};
use coqui_acp_provider::guatiao::{Alloc, Map, Value};
use coqui_acp_provider::{AcpFacade, AgentProvider, Settings as FacadeSettings};
use serde::{Deserialize, Serialize};

use crate::config::{Origin, Settings};

/// What [`serve_stdio`] loads and how it serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProviderOptions {
    /// The Guatiao library or source file.
    pub origin: Origin,
    /// The provider's id.
    pub provider: String,
    /// The provider's configuration; `None` takes the provider's own instance.
    pub settings: Option<Settings>,
    /// Directories a session's `cwd` must be under; empty takes any absolute path.
    pub working_roots: Vec<PathBuf>,
    /// Sessions the connection may hold.
    pub max_sessions: usize,
}

impl ProviderOptions {
    /// Options for `provider` from `origin`, with no configuration, any working directory and the facade's own session
    /// limit.
    #[must_use]
    pub fn new(origin: Origin, provider: impl Into<String>) -> Self {
        Self {
            origin,
            provider: provider.into(),
            settings: None,
            working_roots: Vec::new(),
            max_sessions: FacadeSettings::default().max_sessions,
        }
    }
}

/// The settings `canoa serve` gives a provider process as the first line of its standard input, so that no value is
/// on a command line, which other users of the machine may read. `Debug` shows no setting's value and no token.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostLine {
    /// What a source is added with.
    pub source_settings: Option<Settings>,
    /// The provider's configuration.
    pub settings: Option<Settings>,
    /// What the process sends first on the socket it serves ACP on, proving it is the process the host started.
    pub token: Option<String>,
}

impl fmt::Debug for HostLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostLine")
            .field("source_settings", &self.source_settings)
            .field("settings", &self.settings)
            .field("token", &self.token.as_ref().map(|_| "…"))
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    canoa_host: u32,
    #[serde(default)]
    source_settings: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    settings: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    token: Option<String>,
}

/// What [`Line::canoa_host`] holds: a client's ACP message never has the field, so one is never read as settings.
const LINE_VERSION: u32 = 1;

impl HostLine {
    /// The line as JSON, with no line break in it or after it.
    #[must_use]
    pub fn to_line(&self) -> Vec<u8> {
        let line = Line {
            canoa_host: LINE_VERSION,
            source_settings: self.source_settings.as_ref().map(|s| s.as_map().clone()),
            settings: self.settings.as_ref().map(|s| s.as_map().clone()),
            token: self.token.clone(),
        };
        serde_json::to_vec(&line).unwrap_or_default()
    }

    /// Reads one line of at most `max` bytes from `input`, leaving the rest of it for the transport.
    ///
    /// # Errors
    ///
    /// [`ProviderError::HostLine`] for input that ends first, a line too long, or a line that is not this one.
    pub fn read(input: &mut impl BufRead, max: usize) -> Result<Self, ProviderError> {
        let mut text = String::new();
        let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
        input
            .take(limit)
            .read_line(&mut text)
            .map_err(|_| ProviderError::HostLine("standard input could not be read"))?;
        if !text.ends_with('\n') {
            return Err(ProviderError::HostLine(if text.len() > max {
                "the first line is too long"
            } else {
                "standard input ended before the first line did"
            }));
        }
        let line: Line = serde_json::from_str(text.trim_end())
            .map_err(|_| ProviderError::HostLine("the first line is not the host's settings"))?;
        if line.canoa_host != LINE_VERSION {
            return Err(ProviderError::HostLine(
                "the first line is another version's",
            ));
        }
        Ok(Self {
            source_settings: line.source_settings.map(Settings::new),
            settings: line.settings.map(Settings::new),
            token: line.token,
        })
    }
}

/// Why a provider could not be served.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProviderError {
    /// The library could not be loaded; the loader's words.
    Load(String),
    /// The source could not be added, or refused its settings; its words or the loader's.
    Source(String),
    /// Nothing loaded offers a provider by that id.
    Missing,
    /// What it offers by that id is another kind, or a table of another version of the kind.
    WrongKind,
    /// The provider needs a configuration, and the entry gives no `settings`.
    NeedsConfiguration,
    /// The entry gives `settings` to a provider that takes none.
    TakesNoSettings,
    /// The settings do not fit the provider's schema, or the provider refused them. Names a setting, never a value.
    Settings(String),
    /// The first line of standard input was not the host's settings.
    HostLine(&'static str),
    /// Standard input and output could not carry ACP.
    Transport(std::io::Error),
    /// The facade ended with an error; its words.
    Facade(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(why) => write!(f, "the library did not load: {why}"),
            Self::Source(why) => write!(f, "the source was not added: {why}"),
            Self::Missing => f.write_str("nothing loaded offers a provider by that id"),
            Self::WrongKind => {
                f.write_str("that provider is not an agent_provider of this version")
            }
            Self::NeedsConfiguration => {
                f.write_str("that provider needs a configuration: give the entry a settings table")
            }
            Self::TakesNoSettings => f.write_str(
                "that provider takes no configuration: remove the entry's settings table",
            ),
            Self::Settings(why) => write!(f, "the provider's settings were refused: {why}"),
            Self::HostLine(why) => write!(f, "the host's settings did not arrive: {why}"),
            Self::Transport(error) => write!(f, "standard input and output did not start: {error}"),
            Self::Facade(why) => write!(f, "the ACP facade failed: {why}"),
        }
    }
}

impl core::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

/// Loads the provider and serves it on this process's standard input and output until the client closes them.
///
/// **Loading runs code** with this process's rights, before any check: a library's, or a source's and then whatever it
/// loads, which for Python is the interpreter, its environment and every module its settings and manifests reach. Name
/// only files you would run as a program. A library's imports are found beside it and in the system's directories, not in
/// the working directory or `PATH`; on Windows an import a source names itself is looked for in its settings' `import_dirs`,
/// and with [`ImportSearch::Path`](crate::config::ImportSearch::Path) in the working directory and on `PATH` too, when those
/// leave it missing. Nothing may print to standard output while this runs, since that is the transport.
///
/// # Errors
///
/// [`ProviderError`] for a library or source that does not load, a provider it does not offer or settings it refuses, and
/// for a transport or facade failure.
pub fn serve_stdio(options: &ProviderOptions) -> Result<(), ProviderError> {
    serve_on(options, std::io::stdin(), std::io::stdout())
}

/// As [`serve_stdio`], on the socket `canoa serve` listens on at `address`, after sending it `token`. The process's
/// standard input, output and error are then the provider's alone: none of them is the transport.
///
/// # Errors
///
/// [`ProviderError::Transport`] for an address that is not loopback or a socket that does not connect, and as
/// [`serve_stdio`].
pub fn serve_connected(
    options: &ProviderOptions,
    address: SocketAddr,
    token: &str,
) -> Result<(), ProviderError> {
    if !address.ip().is_loopback() {
        return Err(ProviderError::Transport(std::io::Error::other(
            "the host's socket is on loopback",
        )));
    }
    let mut stream = TcpStream::connect(address).map_err(ProviderError::Transport)?;
    let _ = stream.set_nodelay(true);
    stream
        .write_all(format!("{token}\n").as_bytes())
        .map_err(ProviderError::Transport)?;
    let reader = stream.try_clone().map_err(ProviderError::Transport)?;
    serve_on(options, reader, stream)
}

fn serve_on(
    options: &ProviderOptions,
    input: impl std::io::Read + Send + 'static,
    output: impl Write + Send + 'static,
) -> Result<(), ProviderError> {
    // What the provider hands out points into the library, so the registry and its mapping outlive the facade.
    let (_registry, provider) = load(options)?;
    let mut settings = FacadeSettings::default();
    settings.working_roots.clone_from(&options.working_roots);
    settings.max_sessions = options.max_sessions;
    let (transport, _diagnostics) = lines::over_io(input, output, 0, lines::DEFAULT_MAX_LINE)
        .map_err(ProviderError::Transport)?;
    AcpFacade::new(provider)
        .with_settings(settings)
        .with_agent_info("canoa", env!("CARGO_PKG_VERSION"))
        .run(transport)
        .map_err(|error| ProviderError::Facade(error.to_string()))
}

fn load(options: &ProviderOptions) -> Result<(Registry, Arc<dyn AgentProvider>), ProviderError> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    match &options.origin {
        Origin::Library(library) => {
            registry
                .load_file(library)
                .map_err(|error| ProviderError::Load(error.to_string()))?;
        }
        Origin::Source {
            file,
            settings,
            import_search,
        } => {
            // A language's runtime is often installed elsewhere than the source: Python's python3.dll, on Windows.
            registry.set_import_search(match import_search {
                crate::config::ImportSearch::Path => ImportSearch::Path,
                _ => ImportSearch::Library,
            });
            let config = map_of(settings).map_err(ProviderError::Source)?;
            registry
                .add_source_file(file, &config)
                .map_err(|error| ProviderError::Source(error.to_string()))?;
            for refused in registry.refresh().refused {
                tracing::warn!(
                    library = refused.id.as_deref().unwrap_or("?"),
                    "the source refused a library: {}",
                    refused.reason
                );
            }
        }
    }
    let offer = registry
        .offer::<dyn AgentProvider>(&options.provider)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    let provider: Arc<dyn AgentProvider> = match &options.settings {
        None => offer.shared().ok_or(ProviderError::NeedsConfiguration)?,
        Some(_) if !offer.builds_instances() => return Err(ProviderError::TakesNoSettings),
        Some(settings) => {
            let config = Value::from(map_of(settings).map_err(ProviderError::Settings)?);
            Arc::new(
                offer
                    .instantiate(&config)
                    .map_err(|error| ProviderError::Settings(error.to_string()))?,
            )
        }
    };
    Ok((registry, provider))
}

/// `settings` as a Guatiao map.
fn map_of(settings: &Settings) -> Result<Map, String> {
    let mut map = Map::new();
    for (key, value) in settings.as_map() {
        let value = guatiao_serde::adapt::to_value_in(value, Alloc::rust())
            .map_err(|_| format!("the setting {key:?} cannot be a Guatiao value"))?;
        map.set(key, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::std_instead_of_core,
        reason = "tests name what they use from std"
    )]

    use std::io::BufReader;

    use super::*;

    fn settings(json: serde_json::Value) -> Settings {
        let serde_json::Value::Object(map) = json else {
            panic!("an object");
        };
        Settings::new(map)
    }

    #[test]
    fn the_host_line_is_one_line_and_reads_back() {
        let line = HostLine {
            source_settings: Some(settings(serde_json::json!({"dirs": ["a\nb"], "paths": []}))),
            settings: Some(settings(serde_json::json!({"key": "x\ry"}))),
            token: Some("00ff".to_owned()),
        };
        let bytes = line.to_line();
        assert!(!bytes.iter().any(|byte| matches!(byte, b'\n' | b'\r')));
        let mut input = bytes.clone();
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\"}\n");
        let mut reader = BufReader::new(input.as_slice());
        assert_eq!(HostLine::read(&mut reader, 1024).expect("read"), line);
        let mut rest = String::new();
        reader.read_to_string(&mut rest).expect("the rest");
        assert_eq!(
            rest, "{\"jsonrpc\":\"2.0\"}\n",
            "the transport keeps what follows"
        );
    }

    #[test]
    fn a_client_message_is_never_read_as_the_host_line() {
        for first in [
            "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{}}\n",
            "{}\n",
            "{\"settings\":{\"prefix\":\"x\"}}\n",
            "{\"canoa_host\":2}\n",
            "{\"canoa_host\":1,\"settings\":{},\"jsonrpc\":\"2.0\"}\n",
            "not json\n",
        ] {
            let error =
                HostLine::read(&mut BufReader::new(first.as_bytes()), 1024).expect_err(first);
            assert!(
                matches!(error, ProviderError::HostLine(_)),
                "{first}: {error}"
            );
        }
    }

    #[test]
    fn a_host_line_that_ends_early_or_runs_long_is_refused() {
        let line = HostLine::default().to_line();
        let max = line.len();
        let mut long = line.clone();
        long.extend_from_slice(b" \n");
        for (input, max) in [(line.clone(), max), (long, max), (Vec::new(), max)] {
            let error =
                HostLine::read(&mut BufReader::new(input.as_slice()), max).expect_err("refused");
            assert!(matches!(error, ProviderError::HostLine(_)), "{error}");
        }
        let mut exact = line;
        exact.push(b'\n');
        HostLine::read(&mut BufReader::new(exact.as_slice()), max)
            .expect("a line of max bytes fits");
    }

    #[test]
    fn the_host_line_shows_no_value() {
        let shown = format!(
            "{:?}",
            HostLine {
                source_settings: None,
                settings: Some(settings(serde_json::json!({"token": "sk-SECRET"}))),
                token: Some("TOKEN-SECRET".to_owned()),
            }
        );
        assert!(!shown.contains("SECRET"), "{shown}");
    }
}
