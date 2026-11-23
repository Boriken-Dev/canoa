//! The `canoa` command line as a library: the `canoa` program is `main` in front of it, and an application hosting this
//! library in its own process is another [`Front`] to the same commands.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::ImportSearch;
use crate::home::Home;
use crate::{HostLine, Origin, ProviderOptions, peers};

mod args;
mod bridge;
mod commands;
mod office;
#[cfg(all(unix, feature = "portal"))]
mod portal;

#[cfg(test)]
mod tests;

pub use args::Cli;
use args::Command;
pub(crate) use args::Format;

/// The status of `serve` when another host holds its home.
pub const STATUS_HELD: u8 = 3;

/// The status of `serve` when a key granted `host.restart` asked it to end so that its supervisor starts it again.
pub const STATUS_RESTART: u8 = 75;

/// What a command needs from whatever runs it.
pub trait Front: Send + Sync {
    /// Writes one line of the command's results, standard output for the `canoa` program.
    ///
    /// # Errors
    ///
    /// The line could not be written.
    fn print(&self, line: &str) -> std::io::Result<()>;

    /// Says one line of diagnostics, standard error for the `canoa` program.
    fn say(&self, line: &str);

    /// Runs `then` when the command's input closes, for `--until-stdin-closes`. A front that ends its process afterwards
    /// ends it with `status`.
    ///
    /// # Errors
    ///
    /// The wait could not be set up.
    fn when_input_closes(&self, status: u8, then: Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

    /// The program that runs `provider`, `tool`, `launch` and `mcp-bridge` for this host.
    ///
    /// # Errors
    ///
    /// It cannot be found.
    fn program(&self) -> std::io::Result<Program>;

    /// The directory of the identity, trust store and lock: `None` for this user's own.
    fn home(&self) -> Option<PathBuf>;

    /// What `serve` calls itself to another host that finds this home held.
    fn holder(&self) -> &str;

    /// Asks the person for one line, `secret` ones typed without echo; `Ok(None)` when this front has no way to take a
    /// typed answer, as an application that asks in forms of its own. A secret is never read from a command line.
    ///
    /// # Errors
    ///
    /// The answer could not be read.
    fn ask(&self, prompt: &str, secret: bool) -> std::io::Result<Option<String>>;

    /// Hands over the way into the entries from inside this process, once `serve` is bound and before it says it is ready.
    /// It works until `serve` has ended. A front that has no use for it ignores it.
    fn local(&self, _local: crate::Local) {}

    /// The programs the control channel's `store/*` methods keep and the ids of the stores it offers, for a front that serves
    /// stores of its own; `None` serves the built-in stores over this user's programs folder.
    fn programs(&self) -> Option<(Arc<crate::store::programs::Programs>, Vec<String>)> {
        None
    }
}

/// The name of the command that runs a library's helper: `helper LIBRARY ARGS`.
pub const HELPER: &str = "helper";

/// How a host starts its own `provider`, `tool` and `launch` commands.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Program {
    /// The executable.
    pub path: PathBuf,
    /// What goes before the subcommand on its command line.
    pub args: Vec<OsString>,
}

impl Program {
    /// `path`, with `args` before each subcommand.
    #[must_use]
    pub const fn new(path: PathBuf, args: Vec<OsString>) -> Self {
        Self { path, args }
    }
}

/// A command that failed: the status it ends with, and why, in one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    /// 1 the operation failed, 2 the invocation or canoa.cfg was wrong, [`STATUS_HELD`] another host holds this home.
    pub status: u8,
    /// Why.
    pub why: String,
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.why)
    }
}

impl core::error::Error for Failed {}

impl Failed {
    pub(crate) fn usage(why: impl ToString) -> Self {
        Self {
            status: 2,
            why: why.to_string(),
        }
    }

    pub(crate) fn run(why: impl ToString) -> Self {
        Self {
            status: 1,
            why: why.to_string(),
        }
    }
}

pub(super) fn home_of(front: &dyn Front) -> Result<Home, Failed> {
    match front.home() {
        Some(dir) => Ok(Home::at(dir)),
        None => Home::of_user().map_err(Failed::run),
    }
}

pub(super) fn store(home: &Home) -> Result<coqui_peer_webcrypto::TrustStore, Failed> {
    home.store().map_err(Failed::run)
}

/// Runs one command to its end.
///
/// # Errors
///
/// Why it failed, and the status it ends with.
pub fn run(cli: Cli, front: &dyn Front) -> Result<(), Failed> {
    match cli.command {
        #[cfg(all(unix, feature = "portal"))]
        Command::Portal(args) => portal::portal(&args, front),
        Command::Serve(args) => commands::serve(&args, front),
        Command::Check(args) => commands::check(&args, front),
        Command::OfficeLauncher(args) => office::office_launcher(&args, front),
        Command::Layers(args) => commands::layers(&args, front),
        Command::Options(args) => commands::options(&args, front),
        Command::Id {
            method,
            identity: Some(path),
            ..
        } => commands::print(front, &bridge::identity_file(&path, method.method)?),
        Command::Id { method, force, .. } => commands::print(
            front,
            &method
                .identity_unless_running(&home_of(front)?, force)?
                .peer_id()
                .to_string(),
        ),
        Command::Peers { format } => commands::list_peers(format, front),
        Command::Pair {
            join: Some(url),
            pin_file,
            ca_file,
            timeout_seconds,
            method,
            ..
        } => bridge::pair_join(
            &url,
            pin_file,
            ca_file,
            timeout_seconds,
            method.method,
            front,
        ),
        Command::Pair {
            listen: _,
            timeout_seconds,
            until_stdin_closes,
            native,
            method,
            force,
            ..
        } if native => commands::pair_native(
            Duration::from_secs(timeout_seconds),
            until_stdin_closes,
            method,
            force,
            front,
        ),
        Command::Pair {
            listen,
            timeout_seconds,
            until_stdin_closes,
            origins,
            method,
            force,
            ..
        } => commands::pair(
            listen,
            Duration::from_secs(timeout_seconds),
            until_stdin_closes,
            origins,
            method,
            force,
            front,
        ),
        Command::Discover { seconds } => bridge::discover(seconds),
        Command::Dial(args) => bridge::dial(*args, front),
        Command::Forward(args) => bridge::forward(*args, front),
        Command::Grant {
            peer,
            protocol,
            revoke,
        } => bridge::grant(&peer, &protocol, revoke, front),
        Command::McpConfig { peer, name } => {
            commands::print(front, &bridge::mcp_config(&peer, &name, front)?)
        }
        Command::McpShim { peer } => bridge::mcp_shim(peer.as_deref(), front),
        Command::AgentGuard { pid } => {
            bridge::agent_guard(pid);
            Ok(())
        }
        Command::Accept { peer } => {
            let id =
                peers::accept(&store(&home_of(front)?)?, &peer).map_err(commands::peers_failure)?;
            commands::print(front, &format!("accepted {id}"))
        }
        Command::Revoke { peer } => {
            if peers::revoke(&store(&home_of(front)?)?, &peer).map_err(commands::peers_failure)? {
                commands::print(front, "removed")
            } else {
                Err(Failed::run("the trust store does not know that key"))
            }
        }
        Command::Provider {
            library,
            source,
            import_search,
            provider,
            working_roots,
            max_sessions,
            directory,
            settings_on_stdin,
            connect,
        } => {
            // A file named on the command line is taken from where the command was given.
            let absolute = |path: PathBuf| std::path::absolute(&path).unwrap_or(path);
            let (library, source) = (library.map(absolute), source.map(absolute));
            if let Some(directory) = directory {
                std::env::set_current_dir(&directory).map_err(|error| {
                    Failed::run(format!("could not run in {}: {error}", directory.display()))
                })?;
            }
            let line = if settings_on_stdin {
                HostLine::read(
                    &mut std::io::stdin().lock(),
                    crate::config::DEFAULT_MAX_MESSAGE_BYTES,
                )
                .map_err(Failed::run)?
            } else {
                HostLine::default()
            };
            let origin = match (library, source) {
                (Some(library), None) if line.source_settings.is_none() => Origin::Library(library),
                (None, Some(file)) => {
                    let settings = line.source_settings.unwrap_or_default();
                    let named = import_search.as_deref().and_then(ImportSearch::named);
                    let import_search =
                        named.unwrap_or_else(|| ImportSearch::default_for(&settings));
                    Origin::Source {
                        file,
                        settings,
                        import_search,
                        import_search_named: named.is_some(),
                    }
                }
                _ => return Err(Failed::usage("source settings go with --source alone")),
            };
            let mut options = ProviderOptions::new(origin, provider);
            options.settings = line.settings;
            options.working_roots = working_roots;
            options.max_sessions = max_sessions;
            match (connect, line.token) {
                (Some(address), Some(token)) => {
                    crate::provider::serve_connected(&options, address, &token)
                }
                (Some(_), None) => {
                    return Err(Failed::usage("--connect needs a token in the settings"));
                }
                (None, _) => crate::serve_stdio(&options),
            }
            .map_err(Failed::run)
        }
        Command::Tool { name, spec } => {
            let spec: crate::ToolSpec = serde_json::from_str(&spec)
                .map_err(|_| Failed::usage("--spec is not a tool entry of this version"))?;
            crate::tool::serve_stdio(spec, &name).map_err(|error| match error {
                crate::tool::ToolError::Spec(_) => Failed::usage(error),
                _ => Failed::run(error),
            })
        }
        Command::Agent(agent) => {
            let run = agent.run();
            crate::harness::serve_stdio(&run).map_err(|refused| {
                if refused.usage {
                    Failed::usage(refused.why)
                } else {
                    Failed::run(refused.why)
                }
            })
        }
        Command::Accounts { command } => commands::accounts(command, front),
        Command::Service(args) => commands::service(*args, front),
        Command::Native { action } => commands::native(action, front),
        Command::NativeHost { origin, .. } => commands::native_host(&origin, front),
        Command::McpBridge => commands::mcp_bridge(),
        Command::Helper { library, args } => crate::bundled::run_helper(&library, &args)
            .ok_or_else(|| {
                Failed::usage(format!(
                    "no library of this host has a helper called {library}"
                ))
            })?
            .map_err(Failed::run),
        Command::HookForward {
            channel, kind, id, ..
        } if kind == crate::environments::intercept::KIND_TOOL
            || kind == crate::environments::intercept::KIND_EXEC
            || kind == crate::environments::intercept::KIND_START =>
        {
            let status = crate::environments::intercept::forward(
                &kind,
                &channel,
                id.as_deref(),
                std::io::stdin(),
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            );
            if status == 0 {
                Ok(())
            } else {
                // What went wrong is on standard error already; the status is the answer.
                Err(Failed {
                    status,
                    why: String::new(),
                })
            }
        }
        Command::HookForward {
            channel,
            refuse,
            kind,
            ..
        } => match crate::profile::forward_hook(&kind, &channel, refuse, std::io::stdin()) {
            Ok(false) => Ok(()),
            Ok(true) => Err(Failed::usage("an overlay's hook stopped this")),
            Err(why) => Err(Failed::usage(why)),
        },
        #[cfg(unix)]
        Command::Spawner => crate::spawn::serve().map_err(Failed::run),
        Command::Launch { program, args } => {
            let error =
                crate::profile::launch(&program, &args, crate::config::DEFAULT_MAX_MESSAGE_BYTES);
            Err(match error.kind() {
                std::io::ErrorKind::InvalidData => Failed::usage(error),
                _ => Failed::run(error),
            })
        }
    }
}

impl Cli {
    /// Whether this is one of the commands a host starts for itself, which take the process's own standard streams:
    /// `provider`, `tool`, `agent`, `launch`, `mcp-bridge`, the library's `serve --program`, `dial`, `forward`, `discover`,
    /// `mcp-shim`, `agent-guard` and `pair --join`, or, on Unix, `spawner`. An application that runs as one when it starts
    /// itself takes nothing else on that command line.
    #[must_use]
    pub fn is_a_hosts_own(&self) -> bool {
        #[cfg(unix)]
        if matches!(self.command, Command::Spawner) {
            return true;
        }
        // These run until stopped, end the process (a short-lived certificate does), or print and read the process's own streams.
        match &self.command {
            Command::Serve(args) if args.program => return true,
            Command::Pair { join: Some(_), .. } => return true,
            _ => {}
        }
        matches!(
            self.command,
            Command::Provider { .. }
                | Command::Tool { .. }
                | Command::Agent(..)
                | Command::Launch { .. }
                | Command::McpBridge
                | Command::NativeHost { .. }
                | Command::McpShim { .. }
                | Command::Dial(_)
                | Command::Forward(_)
                | Command::Discover { .. }
                | Command::AgentGuard { .. }
                | Command::HookForward { .. }
                | Command::Helper { .. }
        )
    }
}
