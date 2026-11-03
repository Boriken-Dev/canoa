//! `canoa-host`: serves the agents and providers Canoa.cfg names to paired keys, on loopback.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a binary names what it uses from std"
)]

use std::io::{IsTerminal as _, Write as _};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use canoa_host::{
    Config, HostLine, Origin, PeerState, ProviderOptions, ServeOptions, Serving, peers,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use coqui_agentic_peer::auth::Method;
use coqui_agentic_peer::{auth, home, pairing};
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel as _, Failure, Limits};

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
    Serve(ServeArgs),
    /// Check Canoa.cfg's [host] table and print its entries, serving nothing.
    Check(ConfigArgs),
    /// Print this host's peer id, which a client pins.
    Id {
        #[command(flatten)]
        method: MethodArg,
    },
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
    /// Show a PIN and pair the native key that uses it, then end. Prints {"pin", "peer", "url"}, then {"paired"}.
    /// One wrong try ends it. A browser cannot pair this way: it connects once and is accepted by its id.
    Pair {
        /// The loopback address to wait on.
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: SocketAddr,
        /// How long to wait for the PIN to be used.
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        /// End, the PIN unused, when standard input closes: for a program that runs this as its child.
        #[arg(long)]
        until_stdin_closes: bool,
        #[command(flatten)]
        method: MethodArg,
    },
    /// Serve one Guatiao agent provider as an ACP agent on standard input and output.
    Provider {
        /// The Guatiao library file. Loading it runs its code: name only a file you would run.
        #[arg(long, required_unless_present = "source", conflicts_with = "source")]
        library: Option<PathBuf>,
        /// A Guatiao source file, such as guatiao_source_py, and the code it loads: name only files you would run.
        #[arg(long)]
        source: Option<PathBuf>,
        /// The provider's id.
        #[arg(long)]
        provider: String,
        /// A directory a session's cwd must be under; repeat for several. None takes any absolute path.
        #[arg(long = "working-root")]
        working_roots: Vec<PathBuf>,
        /// Sessions the connection may hold.
        #[arg(long, default_value_t = canoa_host::config::DEFAULT_MAX_SESSIONS)]
        max_sessions: usize,
        /// The directory to run in, from which relative paths in the settings are taken.
        #[arg(long)]
        directory: Option<PathBuf>,
        /// Read the settings from the first line of standard input, as `canoa-host serve` writes them.
        #[arg(long)]
        settings_on_stdin: bool,
        /// Serve ACP on the loopback socket `canoa-host serve` listens on, proving the token its first line gives, instead
        /// of on standard input and output.
        #[arg(long, requires = "settings_on_stdin")]
        connect: Option<std::net::SocketAddr>,
    },
    /// Serve one [host.tools] entry as an ACP agent on standard input and output: what `serve` runs for each connection.
    Tool {
        /// The entry's name, which the agent's notices use.
        #[arg(long)]
        name: String,
        /// The entry's table as JSON, without listen, working_roots, peers and max_peers.
        #[arg(long)]
        spec: String,
    },
}

/// Which proof this host runs, chosen here and never negotiated on a connection.
#[derive(Clone, Copy, Debug, Args)]
struct MethodArg {
    /// `noise`, or `webcrypto` for a browser whose WebCrypto has no X25519. This user's identity of that method, made when
    /// absent. Without it, the identity this user has, or a new one of the default method; with one of each, name one.
    #[arg(long, value_name = "METHOD")]
    method: Option<Method>,
}

impl MethodArg {
    fn identity(self) -> Result<auth::Identity, Failed> {
        home::identity(None, self.method).map_err(Failed::run)
    }
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    config: ConfigArgs,
    #[command(flatten)]
    method: MethodArg,
    /// End, with every agent started, when standard input closes: for a program that runs the host as its child.
    #[arg(long)]
    until_stdin_closes: bool,
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

fn serve(args: &ServeArgs) -> Result<(), Failed> {
    let config = load(&args.config)?;
    let identity = args.method.identity()?;
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
            "method": serving.method().name(),
        });
        print(&line.to_string())?;
    }
    if args.until_stdin_closes {
        let stopper = serving.stopper();
        end_with_input(0, move || stopper.stop_agents())?;
    }
    serving.run();
    Ok(())
}

/// Connections pairing at once; past it a new one is dropped.
const MAX_PAIRING_RUNS: usize = 16;
/// How long one connection may take to pair.
const PAIRING_RUN: Duration = Duration::from_secs(60);

/// Ends this process when standard input closes, logging nothing first: a parent that died can leave standard error a
/// pipe nobody reads, and on Windows a write to it then never returns.
fn end_with_input(code: i32, before: impl FnOnce() + Send + 'static) -> Result<(), Failed> {
    std::thread::Builder::new()
        .name("canoa-host-stdin".to_owned())
        .spawn(move || {
            // Whatever arrives is not read as anything: only its end matters.
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            // Should what runs first wait like that, the process ends anyway.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                std::process::exit(code);
            });
            before();
            std::process::exit(code);
        })
        .map(drop)
        .map_err(Failed::run)
}

/// No browser sends this as its `Origin`, so the pairing listener takes only clients that send none: native programs.
const NO_BROWSER: &str = "-";

fn pair(
    listen: SocketAddr,
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
) -> Result<(), Failed> {
    if !listen.ip().is_loopback() {
        return Err(Failed::usage("pair listens only on loopback"));
    }
    let identity = Arc::new(method.identity()?);
    let store = Arc::new(store()?);
    let listener = websocket::Listener::bind(listen, None).map_err(Failed::run)?;
    let bound = listener.local_addr().map_err(Failed::run)?;
    // A page could run its own PIN exchange and spend the one try, so a browser is refused before the handshake.
    let listener = listener
        .with_frames(Frames::Binary)
        .with_origins([NO_BROWSER.to_owned()])
        .with_limits(
            Limits::default()
                .with_max_message(auth::MAX_PROOF_MESSAGE)
                .with_pending(8),
        );
    if until_stdin_closes {
        end_with_input(1, || {})?;
    }
    let desk = pairing::desk().map_err(Failed::run)?;
    let shown = desk.show_pin().map_err(Failed::run)?;
    print(
        &serde_json::json!({
            "pin": shown.pin.as_str(),
            "peer": identity.peer_id().to_string(),
            "method": identity.method().name(),
            "url": format!("ws://{bound}"),
        })
        .to_string(),
    )?;
    let until = Instant::now() + timeout;
    let (report, reports) = mpsc::channel();
    let running = Arc::new(AtomicUsize::new(0));
    loop {
        while let Ok(result) = reports.try_recv() {
            match result {
                Ok(peer) => {
                    return print(&serde_json::json!({ "paired": peer }).to_string());
                }
                Err(why) => {
                    tracing::info!("a pairing attempt failed: {why}");
                    if desk.shown_pin().is_none() {
                        return Err(Failed::run("the PIN is used up or expired: start again"));
                    }
                }
            }
        }
        if Instant::now() >= until {
            return Err(Failed::run("nobody paired in time"));
        }
        let pending = match listener.next(Some(Duration::from_millis(100))) {
            Ok(pending) => pending,
            Err(Failure::Timeout) => continue,
            Err(failure) => return Err(Failed::run(failure)),
        };
        // Each connection runs apart, so one that connects and says nothing costs neither the PIN nor the wait.
        if running.fetch_add(1, Ordering::SeqCst) >= MAX_PAIRING_RUNS {
            running.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (identity, store, desk, report, running) = (
            Arc::clone(&identity),
            Arc::clone(&store),
            desk.clone(),
            report.clone(),
            Arc::clone(&running),
        );
        std::thread::spawn(move || {
            let result = pending
                .handshake()
                .map_err(|failure| failure.to_string())
                .and_then(|channel| {
                    let paired =
                        pairing::respond(&channel, &identity, &store, &desk, None, PAIRING_RUN);
                    channel.close();
                    paired.map_err(|error| error.to_string())
                })
                .map(|paired| paired.peer.id().to_string());
            running.fetch_sub(1, Ordering::SeqCst);
            let _ = report.send(result);
        });
    }
}

fn check(args: &ConfigArgs) -> Result<(), Failed> {
    let config = load(args)?;
    for entry in &config.entries {
        let kind = match entry.kind {
            canoa_host::EntryKind::Agent { .. } => "agent",
            canoa_host::EntryKind::Provider { .. } => "provider",
            canoa_host::EntryKind::Tool { .. } => "tool",
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
        Command::Id { method } => print(&method.identity()?.peer_id().to_string()),
        Command::Peers { format } => list_peers(format),
        Command::Pair {
            listen,
            timeout_seconds,
            until_stdin_closes,
            method,
        } => pair(
            listen,
            Duration::from_secs(timeout_seconds),
            until_stdin_closes,
            method,
        ),
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
            source,
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
                HostLine::read(&mut std::io::stdin().lock(), canoa_host::serve::MAX_LINE)
                    .map_err(Failed::run)?
            } else {
                HostLine::default()
            };
            let origin = match (library, source) {
                (Some(library), None) if line.source_settings.is_none() => Origin::Library(library),
                (None, Some(file)) => Origin::Source {
                    file,
                    settings: line.source_settings.unwrap_or_default(),
                },
                _ => return Err(Failed::usage("source settings go with --source alone")),
            };
            let mut options = ProviderOptions::new(origin, provider);
            options.settings = line.settings;
            options.working_roots = working_roots;
            options.max_sessions = max_sessions;
            match (connect, line.token) {
                (Some(address), Some(token)) => {
                    canoa_host::provider::serve_connected(&options, address, &token)
                }
                (Some(_), None) => {
                    return Err(Failed::usage("--connect needs a token in the settings"));
                }
                (None, _) => canoa_host::serve_stdio(&options),
            }
            .map_err(Failed::run)
        }
        Command::Tool { name, spec } => {
            let spec: canoa_host::ToolSpec = serde_json::from_str(&spec)
                .map_err(|_| Failed::usage("--spec is not a tool entry of this version"))?;
            canoa_host::tool::serve_stdio(spec, &name).map_err(|error| match error {
                canoa_host::tool::ToolError::Spec(_) => Failed::usage(error),
                _ => Failed::run(error),
            })
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
        assert_eq!(args.config.config, Some(PathBuf::from("x.cfg")));
        assert!(!args.until_stdin_closes);
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
