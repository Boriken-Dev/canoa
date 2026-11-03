//! `canoa-host`: serves the agents and providers Canoa.cfg names to paired keys, on loopback.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a binary names what it uses from std"
)]

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use canoa_host::{Config, PeerState, ProviderOptions, ServeOptions, Serving, peers};
use clap::{Args, Parser, Subcommand, ValueEnum};
use coqui_agentic_peer::home;

const AFTER_HELP: &str = "\
Environment:
  CANOA_CONFIG             Canoa.cfg to read, below --config and above the platform's default
  CANOA_LOG                what to log on standard error, as a tracing filter (default: info)
  COQUI_AGENTIC_PEER_HOME  the directory of this host's identity and trust store, shared with coqui-acp-bridge
  NO_COLOR                 no colour in the log

Exit status: 0 success, 1 the operation failed, 2 the invocation or Canoa.cfg was wrong.";

/// Serves stdio ACP agents and Guatiao agent providers from Canoa.cfg to paired keys, on loopback.
#[derive(Debug, Parser)]
#[command(name = "canoa-host", version, after_help = AFTER_HELP)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve every entry of Canoa.cfg's [host] table until stopped; prints one JSON line per entry.
    Serve(ConfigArgs),
    /// Check Canoa.cfg's [host] table and print its entries, serving nothing.
    Check(ConfigArgs),
    /// Print this host's peer id, which a client pins.
    Id,
    /// List paired keys, then keys waiting to be paired.
    Peers {
        /// How to print them.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Pair a waiting key, after comparing its id with the one the client shows.
    Accept {
        /// The key's peer id.
        peer: String,
    },
    /// Remove a key, paired or waiting; a link it holds ends within about a second.
    Revoke {
        /// The key's peer id.
        peer: String,
    },
    /// Serve one Guatiao agent provider as an ACP agent on standard input and output.
    Provider {
        /// The Guatiao library file. Loading it runs its code: name only a file you would run.
        #[arg(long)]
        library: PathBuf,
        /// The provider's id in the library.
        #[arg(long)]
        provider: String,
        /// A directory a session's cwd must be under; repeat for several. None takes any absolute path.
        #[arg(long = "working-root")]
        working_roots: Vec<PathBuf>,
        /// Sessions the connection may hold.
        #[arg(long, default_value_t = canoa_host::config::DEFAULT_MAX_SESSIONS)]
        max_sessions: usize,
    },
}

#[derive(Debug, Args)]
struct ConfigArgs {
    /// Canoa.cfg to read; default %APPDATA%\Canoa\Canoa.cfg, ~/Library/Application Support/Canoa/Canoa.cfg, or
    /// $XDG_CONFIG_HOME/Canoa/Canoa.cfg (~/.config without it).
    #[arg(long, env = "CANOA_CONFIG")]
    config: Option<PathBuf>,
    /// How `check` prints entries.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// The platform's place for Canoa.cfg, as the app reads it.
fn default_config() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("APPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))?
    };
    Some(base.join("Canoa").join("Canoa.cfg"))
}

/// A failure, and the status it ends with.
struct Failed(u8, String);

impl Failed {
    fn usage(why: impl ToString) -> Self {
        Self(2, why.to_string())
    }

    fn run(why: impl ToString) -> Self {
        Self(1, why.to_string())
    }
}

fn load(args: &ConfigArgs) -> Result<Config, Failed> {
    let path = args
        .config
        .clone()
        .or_else(default_config)
        .ok_or_else(|| Failed::usage("no Canoa.cfg: give --config or CANOA_CONFIG"))?;
    Config::load(&path).map_err(Failed::usage)
}

fn store() -> Result<coqui_peer_webcrypto::TrustStore, Failed> {
    home::store(None).map_err(Failed::run)
}

fn print(line: &str) -> Result<(), Failed> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(Failed::run)
}

fn serve(args: &ConfigArgs) -> Result<(), Failed> {
    let config = load(args)?;
    let identity = home::identity(None).map_err(Failed::run)?;
    let peer = identity.peer_id().to_string();
    let program = std::env::current_exe()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let serving = Serving::bind(&config, identity, store()?, &ServeOptions::new(program)).map_err(
        |error| match error {
            canoa_host::ServeError::NothingToServe => Failed::usage(error),
            _ => Failed::run(error),
        },
    )?;
    for listening in serving.listening() {
        let line = serde_json::json!({
            "name": listening.name,
            "kind": listening.kind,
            "url": listening.url,
            "peer": peer,
        });
        print(&line.to_string())?;
    }
    serving.run();
    Ok(())
}

fn check(args: &ConfigArgs) -> Result<(), Failed> {
    let config = load(args)?;
    for entry in &config.entries {
        let kind = match entry.kind {
            canoa_host::EntryKind::Agent { .. } => "agent",
            canoa_host::EntryKind::Provider { .. } => "provider",
            _ => "other",
        };
        let line = match args.format {
            Format::Json => serde_json::json!({
                "name": entry.name,
                "kind": kind,
                "listen": entry.listen.to_string(),
            })
            .to_string(),
            Format::Text => format!("{} {kind} {}", entry.name, entry.listen),
        };
        print(&line)?;
    }
    Ok(())
}

fn list_peers(format: Format) -> Result<(), Failed> {
    for peer in peers::list(&store()?).map_err(Failed::run)? {
        let (state, source) = match peer.state {
            PeerState::Accepted => ("accepted", None),
            PeerState::Waiting { source } => ("waiting", source),
            _ => ("other", None),
        };
        let line = match format {
            Format::Json => serde_json::json!({
                "peer": peer.id.to_string(),
                "state": state,
                "source": source.map(|source| source.to_string()),
            })
            .to_string(),
            Format::Text => match source {
                Some(source) => format!("{state} {} from {source}", peer.id),
                None => format!("{state} {}", peer.id),
            },
        };
        print(&line)?;
    }
    Ok(())
}

fn peers_failure(error: peers::PeersError) -> Failed {
    match error {
        peers::PeersError::Store(_) => Failed::run(error),
        _ => Failed::usage(error),
    }
}

fn run(cli: Cli) -> Result<(), Failed> {
    match cli.command {
        Command::Serve(args) => serve(&args),
        Command::Check(args) => check(&args),
        Command::Id => print(
            &home::identity(None)
                .map_err(Failed::run)?
                .peer_id()
                .to_string(),
        ),
        Command::Peers { format } => list_peers(format),
        Command::Accept { peer } => {
            let id = peers::accept(&store()?, &peer).map_err(peers_failure)?;
            print(&format!("accepted {id}"))
        }
        Command::Revoke { peer } => {
            if peers::revoke(&store()?, &peer).map_err(peers_failure)? {
                print("removed")
            } else {
                Err(Failed::run("the trust store does not know that key"))
            }
        }
        Command::Provider {
            library,
            provider,
            working_roots,
            max_sessions,
        } => {
            let mut options = ProviderOptions::new(library, provider);
            options.working_roots = working_roots;
            options.max_sessions = max_sessions;
            canoa_host::serve_stdio(&options).map_err(Failed::run)
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let filter = tracing_subscriber::EnvFilter::try_from_env("CANOA_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .with_target(false)
        .init();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failed(status, why)) => {
            eprintln!("canoa-host: {why}");
            ExitCode::from(status)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn serve_takes_a_config_flag() {
        let cli =
            Cli::try_parse_from(["canoa-host", "serve", "--config", "x.cfg"]).expect("parses");
        let Command::Serve(args) = cli.command else {
            panic!("serve");
        };
        assert_eq!(args.config, Some(PathBuf::from("x.cfg")));
    }

    #[test]
    fn a_provider_needs_its_library_and_id() {
        assert!(Cli::try_parse_from(["canoa-host", "provider", "--library", "x"]).is_err());
        let cli = Cli::try_parse_from([
            "canoa-host",
            "provider",
            "--library",
            "x",
            "--provider",
            "echo",
            "--working-root",
            "/a",
            "--working-root",
            "/b",
        ])
        .expect("parses");
        let Command::Provider {
            working_roots,
            max_sessions,
            ..
        } = cli.command
        else {
            panic!("provider");
        };
        assert_eq!(working_roots, [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(max_sessions, canoa_host::config::DEFAULT_MAX_SESSIONS);
    }

    #[test]
    fn accept_needs_a_peer() {
        assert!(Cli::try_parse_from(["canoa-host", "accept"]).is_err());
    }
}
