//! One Guatiao provider, served as an ACP agent on standard input and output: what the host runs for each connection to a
//! provider entry, so the library is mapped in a process of its own and ends with the connection.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::PathBuf;

use coqui_acp::lines;
use coqui_acp_provider::guatiao::library::Registry;
use coqui_acp_provider::{AcpFacade, AgentProvider, Settings};

/// What [`serve_stdio`] loads and how it serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProviderOptions {
    /// The Guatiao library file.
    pub library: PathBuf,
    /// The provider's id in it.
    pub provider: String,
    /// Directories a session's `cwd` must be under; empty takes any absolute path.
    pub working_roots: Vec<PathBuf>,
    /// Sessions the connection may hold.
    pub max_sessions: usize,
}

impl ProviderOptions {
    /// Options for `provider` in `library`, with any working directory and the facade's own session limit.
    #[must_use]
    pub fn new(library: impl Into<PathBuf>, provider: impl Into<String>) -> Self {
        Self {
            library: library.into(),
            provider: provider.into(),
            working_roots: Vec::new(),
            max_sessions: Settings::default().max_sessions,
        }
    }
}

/// Why a provider could not be served.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProviderError {
    /// The library could not be loaded; the loader's words.
    Load(String),
    /// The library offers nothing by that id.
    Missing,
    /// What it offers by that id is another kind, or a table of another version of the kind.
    WrongKind,
    /// The provider needs a configuration, which this host does not give.
    NeedsConfiguration,
    /// Standard input and output could not carry ACP.
    Transport(std::io::Error),
    /// The facade ended with an error; its words.
    Facade(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(why) => write!(f, "the library did not load: {why}"),
            Self::Missing => f.write_str("the library offers no provider by that id"),
            Self::WrongKind => {
                f.write_str("that provider is not an agent_provider of this version")
            }
            Self::NeedsConfiguration => {
                f.write_str("that provider needs a configuration, which this host does not give")
            }
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
/// **Mapping the library runs its code** with this process's rights: name only a file you would run as a program. Its
/// imports are found beside it and in the system's directories, not in the working directory or `PATH`. Nothing may print
/// to standard output while this runs, since that is the transport.
///
/// # Errors
///
/// [`ProviderError`] for a library that does not load or offer the provider, and for a transport or facade failure.
pub fn serve_stdio(options: &ProviderOptions) -> Result<(), ProviderError> {
    // What the provider hands out points into the library, so the registry and its mapping outlive the facade.
    let (_registry, provider) = load(options)?;
    let mut settings = Settings::default();
    settings.working_roots.clone_from(&options.working_roots);
    settings.max_sessions = options.max_sessions;
    let (transport, _diagnostics) = lines::over_io(
        std::io::stdin(),
        std::io::stdout(),
        0,
        lines::DEFAULT_MAX_LINE,
    )
    .map_err(ProviderError::Transport)?;
    AcpFacade::new(provider)
        .with_settings(settings)
        .with_agent_info("canoa-host", env!("CARGO_PKG_VERSION"))
        .run(transport)
        .map_err(|error| ProviderError::Facade(error.to_string()))
}
fn load(options: &ProviderOptions) -> Result<(Registry, Arc<dyn AgentProvider>), ProviderError> {
    let mut registry = Registry::new("canoa-host", env!("CARGO_PKG_VERSION"));
    registry
        .load_file(&options.library)
        .map_err(|error| ProviderError::Load(error.to_string()))?;
    let offer = registry
        .offer::<dyn AgentProvider>(&options.provider)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    let provider = offer.shared().ok_or(ProviderError::NeedsConfiguration)?;
    Ok((registry, provider))
}
