//! Host serving options, startup errors, advertised entry records, and concurrency gates.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use canoa::brand::CONFIG_FILE;
use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::PathBuf;

use coqui_transport::Failure;

use crate::config::Layers;
use crate::control;
use crate::pipe;
use crate::portal_link::PortalSockets;

/// What [`super::Serving`] needs beyond the configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServeOptions {
    /// The program that serves a provider, a tool, a profile's launch or the delegation's stdio bridge: this host's own executable, run as
    /// `PROGRAM [program_args] provider ...`.
    pub provider_program: PathBuf,
    /// What goes before the subcommand on `provider_program`'s command line: nothing for `canoa`, the flag that makes an
    /// application hosting this library behave as one when it starts itself.
    pub program_args: Vec<OsString>,
    /// What one key may carry in each direction, in bytes; the method's own default, 2^38, when `None`.
    pub key_limit: Option<u64>,
    /// How long a question put to the person (a sign-in's form, an install's) waits for its answer: four minutes by default, which
    /// is how long the request stays bound, and the longest the application accepts.
    pub ask_wait: Duration,
    /// Whether a Noise sender changes its key at the limit; WebCrypto has no key change and ends the session there.
    pub rekey: bool,
    /// Whether a configuration with no entry binds, serving nothing, rather than being refused.
    pub nothing_to_serve: bool,
    /// The host's directory for what it keeps, where an agent started only to read its usage gets an empty working
    /// directory; without one, no such agent is started.
    pub state_dir: Option<PathBuf>,
    /// Whether the host's own sources (every bundled agent's) are served too, over [`super::Local`] only.
    pub local_sources: bool,
    /// The programs the control channel's `store/*` methods keep, and the ids of the stores it offers; without it, the built-in
    /// stores over this user's own programs folder.
    pub programs: Option<(Arc<crate::store::programs::Programs>, Vec<String>)>,
    /// The files canoa.cfg was read from. With them, each connection to one of the host's own sources reads them again for
    /// its profile, so a change to `[overlays]`, `[profile]` or `[agents.NAME]` reaches the next session; a file that no longer
    /// parses leaves the profile as it was.
    pub layers: Option<Layers>,
    /// The person's own canoa.cfg, which a key granted `config.write` edits by section; without it those methods are refused.
    pub config_file: Option<PathBuf>,
    /// The user's home, which a key granted `folders` may browse; the environment's when `None`.
    pub home_dir: Option<PathBuf>,
    /// Whether a supervisor starts the host again when it ends with `cli::STATUS_RESTART`; only then is `host/restart` offered.
    pub supervised: bool,
    /// The canoa.cfg files the control channel's `accounts/*` methods read and write; with the state directory, which is the
    /// home they keep accounts in. Without it the host manages no accounts over the control channel.
    pub account_layers: Option<crate::config::Layers>,
    /// Where a host run for a portal listens: every entry and the control channel on a socket in a directory a group may
    /// reach, and a socket that introduces a browser's key. Unix only.
    pub portal: Option<PortalSockets>,
}

impl ServeOptions {
    /// Options running providers through `provider_program`, with each method's default session settings.
    #[must_use]
    pub fn new(provider_program: impl Into<PathBuf>) -> Self {
        Self {
            provider_program: provider_program.into(),
            program_args: Vec::new(),
            key_limit: None,
            ask_wait: crate::secrets::ask::ASK_WAIT,
            rekey: true,
            nothing_to_serve: false,
            state_dir: None,
            local_sources: false,
            programs: None,
            layers: None,
            config_file: None,
            home_dir: None,
            supervised: false,
            account_layers: None,
            portal: None,
        }
    }

    /// Runs the host for a portal: see [`PortalSockets`]. Every entry's address is replaced by a socket, the
    /// web app is not served, and every key the portal introduces is the user's own browser, so each gets the control
    /// channel's page grants.
    #[must_use]
    pub fn with_portal(mut self, sockets: PortalSockets) -> Self {
        self.portal = Some(sockets);
        self
    }

    /// Reads canoa.cfg's profile again from `layers` at each connection to one of the host's own sources.
    #[must_use]
    pub fn with_layers(mut self, layers: Layers) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Lets the control channel's owner manage accounts in these canoa.cfg files, in the state directory's home.
    #[must_use]
    pub fn with_account_layers(mut self, layers: crate::config::Layers) -> Self {
        self.account_layers = Some(layers);
        self
    }

    /// Serves the host's own sources over [`super::Local`] alongside canoa.cfg's entries. A name canoa.cfg already uses is left to it.
    #[must_use]
    pub const fn with_local_sources(mut self, on: bool) -> Self {
        self.local_sources = on;
        self
    }

    /// Waits `wait` for the person to answer a question the host puts to them, and binds the question for as long.
    #[must_use]
    pub const fn with_ask_wait(mut self, wait: Duration) -> Self {
        self.ask_wait = wait;
        self
    }

    /// Offers the control channel's `store/*` methods over `programs`, through the stores named `providers`.
    #[must_use]
    pub fn with_programs(
        mut self,
        programs: Arc<crate::store::programs::Programs>,
        providers: Vec<String>,
    ) -> Self {
        self.programs = Some((programs, providers));
        self
    }

    /// The canoa.cfg a key granted `config.write` edits, by section.
    #[must_use]
    pub fn with_config_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.config_file = Some(file.into());
        self
    }

    /// Says that a supervisor restarts the host when it ends with `cli::STATUS_RESTART`, which offers `host/restart`.
    #[must_use]
    pub const fn with_supervised(mut self, supervised: bool) -> Self {
        self.supervised = supervised;
        self
    }

    /// The home a key granted `folders` may browse, in place of the environment's.
    #[must_use]
    pub fn with_home_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.home_dir = Some(dir.into());
        self
    }

    /// The directory the host keeps its own working files in, which a private read of an agent's usage needs.
    #[must_use]
    pub fn with_state_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.state_dir = Some(dir.into());
        self
    }

    /// What one key may carry in each direction, in bytes. No floor; above the default the host logs it as a non-default setting.
    #[must_use]
    pub const fn with_key_limit(mut self, bytes: u64) -> Self {
        self.key_limit = Some(bytes);
        self
    }

    /// Whether a Noise sender changes its key at the limit, rather than ending the session.
    #[must_use]
    pub const fn with_rekey(mut self, rekey: bool) -> Self {
        self.rekey = rekey;
        self
    }

    /// What goes before the subcommand on the provider program's command line.
    #[must_use]
    pub fn with_program_args(mut self, args: Vec<OsString>) -> Self {
        self.program_args = args;
        self
    }

    /// Lets a configuration with no entry bind, serving nothing until it is stopped.
    #[must_use]
    pub const fn with_nothing_to_serve(mut self, allowed: bool) -> Self {
        self.nothing_to_serve = allowed;
        self
    }
}

/// Why the host could not start serving.
#[derive(Debug)]
#[non_exhaustive]
pub enum ServeError {
    /// The configuration has no entry.
    NothingToServe,
    /// An entry's address could not be bound.
    Listen {
        /// The entry.
        entry: String,
        /// Why.
        failure: Failure,
    },
    /// An entry's pipe or Unix socket could not be bound.
    Pipe {
        /// The entry.
        entry: String,
        /// Why.
        error: pipe::Error,
    },
    /// An entry's transport provider could not be loaded, found, configured, or started on its endpoint.
    Transport {
        /// The entry.
        entry: String,
        /// Why; names a setting, never a value.
        what: String,
    },
    /// The proof could not be built from the options.
    Proof(String),
    /// A harness entry's provider could not be loaded, found, or configured.
    Harness {
        /// The entry.
        entry: String,
        /// Why; names a setting, never a value.
        what: String,
    },
    /// The accounts canoa.cfg names cannot be served.
    Accounts {
        /// Why; names a key, never a value.
        what: String,
    },
    /// The host's ledger could not be opened.
    Control(control::ControlError),
    /// `[host.network]` cannot be used.
    Network {
        /// Why; names a key, never a value.
        what: String,
    },
    /// An agent entry's `usage` names a reader that no bundled agent and no library or source of canoa.cfg offers.
    Usage {
        /// The entry.
        entry: String,
        /// Why; names the reader's id.
        what: String,
    },
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToServe => f.write_str(
                &format!("{CONFIG_FILE} has no [host.agents], [host.providers] or [host.tools] entry to serve"),
            ),
            Self::Listen { entry, failure } => {
                write!(f, "host entry {entry} could not listen: {failure}")
            }
            Self::Pipe { entry, error } => {
                write!(f, "host entry {entry} could not listen: {error}")
            }
            Self::Transport { entry, what } => {
                write!(
                    f,
                    "host entry {entry}: its transport provider is unusable: {what}"
                )
            }
            Self::Proof(reason) => write!(f, "the proof could not be built: {reason}"),
            Self::Harness { entry, what } => {
                write!(
                    f,
                    "host entry {entry}: its harness provider is unusable: {what}"
                )
            }
            Self::Accounts { what } => write!(f, "[host.accounts]: {what}"),
            Self::Network { what } => write!(f, "[host.network]: {what}"),
            Self::Usage { entry, what } => {
                write!(
                    f,
                    "host entry {entry}: its usage reader is unusable: {what}"
                )
            }
            Self::Control(error) => write!(f, "the control channel's ledger {error}"),
        }
    }
}

impl core::error::Error for ServeError {}

/// An entry that is listening.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listening {
    /// The entry's name.
    pub name: String,
    /// `agent`, `provider` or `tool`.
    pub kind: &'static str,
    /// What a client dials: `ws://ADDRESS`, or `pipe:NAME` or `unix:PATH` for an entry on a pipe, which the served web page
    /// never lists; empty for a local entry.
    pub url: String,
    /// The provider id of a `harness` entry; `None` for any other entry.
    pub harness: Option<String>,
    /// The `transport` provider's id of an entry that listens through one, whose `url` is then the endpoint it says a peer
    /// dials; `None` for an entry on an address or a pipe.
    pub transport: Option<String>,
    /// The entry's `title`, if it has one.
    pub title: Option<String>,
    /// Whether it is reached over [`super::Local`] only: one of the host's own sources, with no address.
    pub local: bool,
    /// The programs a local entry needs that were not found; empty for any other, and for a source that can start.
    pub missing: Vec<String>,
}

impl Listening {
    /// Whether a browser can dial it: it listens on an address, not on a pipe, a Unix socket or a transport provider, whose
    /// framing a page does not know.
    #[must_use]
    pub fn on_address(&self) -> bool {
        self.transport.is_none() && !self.url.starts_with("pipe:") && !self.url.starts_with("unix:")
    }

    /// The entry as `serve` prints it and `/canoa.json` lists it: `name`, `kind`, `url`, and `harness` and `title` when set.
    pub(crate) fn to_json(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), self.name.clone().into());
        map.insert("kind".into(), self.kind.into());
        if self.local {
            map.insert("local".into(), true.into());
            map.insert("missing".into(), self.missing.clone().into());
        } else {
            map.insert("url".into(), self.url.clone().into());
        }
        if let Some(harness) = &self.harness {
            map.insert("harness".into(), harness.clone().into());
        }
        if let Some(transport) = &self.transport {
            map.insert("transport".into(), transport.clone().into());
        }
        if let Some(title) = &self.title {
            map.insert("title".into(), title.clone().into());
        }
        map
    }
}

/// A count of what is running, with a ceiling.
#[derive(Clone, Debug)]
pub(crate) struct Gate(Arc<AtomicUsize>, usize);

/// One place in a [`Gate`], given back when dropped.
pub(crate) struct Entered(Arc<AtomicUsize>);

impl Drop for Entered {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Gate {
    pub(crate) fn new(ceiling: usize) -> Self {
        Self(Arc::new(AtomicUsize::new(0)), ceiling)
    }

    /// The most it lets in at once.
    #[allow(dead_code, reason = "queried by tests or limits")]
    pub(super) const fn ceiling(&self) -> usize {
        self.1
    }

    pub(crate) fn enter(&self) -> Option<Entered> {
        let mut running = self.0.load(Ordering::SeqCst);
        while running < self.1 {
            match self
                .0
                .compare_exchange(running, running + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Entered(Arc::clone(&self.0))),
                Err(now) => running = now,
            }
        }
        None
    }
}
