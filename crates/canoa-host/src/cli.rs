//! The `canoa` command line as a library: the `canoa` program is `main` in front of it, and an application hosting this
//! library in its own process is another [`Front`] to the same commands.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use std::ffi::OsString;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand, ValueEnum};
use coqui_agentic_peer::auth::Method;
use coqui_agentic_peer::{auth, pairing};
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel as _, Failure, Limits};

use crate::config::ImportSearch;
use crate::home::{Home, LockError};
use crate::{
    Config, HostLine, Origin, PeerState, ProviderOptions, ServeOptions, Serving, WebConfig,
    WebError, WebServer, peers,
};

/// The status of `serve` when another host holds its home.
pub const STATUS_HELD: u8 = 3;

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

    /// The program that runs `provider`, `tool` and `launch` for this host.
    ///
    /// # Errors
    ///
    /// It cannot be found.
    fn program(&self) -> std::io::Result<Program>;

    /// The directory of the identity, trust store and lock: `None` for this user's own.
    fn home(&self) -> Option<PathBuf>;

    /// What `serve` calls itself to another host that finds this home held.
    fn holder(&self) -> &str;
}

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

const AFTER_HELP: &str = "\
Environment:
  CANOA_CONFIG             canoa.cfg to read, below --config and above the platform's default
  CANOA_LOG                what to log on standard error, as a tracing filter (default: info)
  COQUI_AGENTIC_PEER_HOME  the directory of this host's identity and trust store, shared with coqui-acp-bridge
  NO_COLOR                 no colour in the log

Exit status: 0 success, 1 the operation failed, 2 the invocation or canoa.cfg was wrong, 3 another host holds this home.";

/// Serves stdio ACP agents and Guatiao agent providers from canoa.cfg to paired keys, on loopback.
#[derive(Debug, Parser)]
#[command(name = "canoa", version, after_help = AFTER_HELP)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve every entry of canoa.cfg's [host] table until stopped; prints one JSON line per entry, then {"ready"}.
    /// Only one host runs on a home: a second says which is running and ends with status 3.
    Serve(ServeArgs),
    /// Check canoa.cfg's [host] table and print its entries, serving nothing.
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
        /// With --source on Windows: library looks for what the source imports itself only in its settings'
        /// import_dirs; path also in the Windows directory, the working directory and PATH. Without it: library when
        /// import_dirs names a directory, path otherwise.
        #[arg(long, requires = "source", value_parser = ["library", "path"])]
        import_search: Option<String>,
        /// The provider's id.
        #[arg(long)]
        provider: String,
        /// A directory a session's cwd must be under; repeat for several. None takes any absolute path.
        #[arg(long = "working-root")]
        working_roots: Vec<PathBuf>,
        /// Sessions the connection may hold.
        #[arg(long, default_value_t = crate::config::DEFAULT_MAX_SESSIONS)]
        max_sessions: usize,
        /// The directory to run in, from which relative paths in the settings are taken.
        #[arg(long)]
        directory: Option<PathBuf>,
        /// Read the settings from the first line of standard input, as `canoa serve` writes them.
        #[arg(long)]
        settings_on_stdin: bool,
        /// Serve ACP on the loopback socket `canoa serve` listens on, proving the token its first line gives, instead
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
    /// Run an agent with the environment the first line of standard input gives: what `serve` runs for an agent entry
    /// with profiles. The rest of standard input goes to the agent.
    Launch {
        /// The agent's program.
        program: std::ffi::OsString,
        /// Its arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
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
    /// The identity a host serves with, its method recorded for the next start.
    fn identity(self, home: &Home) -> Result<auth::Identity, Failed> {
        home.identity(self.method).map_err(Failed::run)
    }

    /// The identity for a command that holds no lock, the home's recorded method left alone.
    fn identity_as_is(self, home: &Home) -> Result<auth::Identity, Failed> {
        home.identity_as_is(self.method).map_err(Failed::run)
    }
}

/// Takes back the PIN when `pair` returns, however it returns: a connection still pairing runs on a thread of its own, which
/// in an application's process goes on after the command has ended.
struct ClearPin<'a>(&'a coqui_peer_webcrypto::pair::PairingDesk);

impl Drop for ClearPin<'_> {
    fn drop(&mut self) {
        self.0.clear_pin();
    }
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    config: ConfigArgs,
    #[command(flatten)]
    method: MethodArg,
    /// What one key may carry in each direction, in bytes. Default 2^38; above it the log says it is weakened.
    #[arg(long, value_name = "BYTES")]
    key_limit: Option<u64>,
    /// Under Noise, end a session at the key limit rather than changing keys, as WebCrypto always does.
    #[arg(long)]
    no_rekey: bool,
    /// End, with every agent started, when standard input closes: for a program that runs the host as its child.
    #[arg(long)]
    until_stdin_closes: bool,
    /// With no entry in canoa.cfg, or no canoa.cfg at all, keep running with nothing to serve, rather than ending with
    /// status 2.
    #[arg(long)]
    allow_empty: bool,
    /// Serve the web app too, on [host.web]'s address (default 127.0.0.1:7800): from DIR, else [host.web]'s dir, else the
    /// site built in (feature `web`).
    #[arg(long, value_name = "DIR", num_args = 0..=1)]
    web: Option<Option<PathBuf>>,
    /// The web app's address, in place of [host.web]'s listen. Beyond loopback it needs [host.web]'s cert and key.
    #[arg(long, value_name = "ADDRESS")]
    web_listen: Option<SocketAddr>,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    /// canoa.cfg to read; default %APPDATA%\Canoa\canoa.cfg, ~/Library/Application Support/Canoa/canoa.cfg, or
    /// $XDG_CONFIG_HOME/Canoa/canoa.cfg (~/.config without it).
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

/// The platform's place for canoa.cfg, as the app reads it. A `Canoa.cfg` there with no `canoa.cfg` beside it is renamed
/// first, on the platforms whose file names are case sensitive.
fn default_config(front: &dyn Front) -> Option<PathBuf> {
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
    let path = base.join("Canoa").join("canoa.cfg");
    if !cfg!(windows) {
        adopt_legacy_name(&path, &|line| front.say(line));
    }
    Some(path)
}

/// Gives a `Canoa.cfg` beside `path` the name `path`, once, and says so on standard error. A file already at `path`, a
/// dangling link included, is never replaced. Returns whether it renamed.
fn adopt_legacy_name(path: &Path, say: &dyn Fn(&str)) -> bool {
    let Some(old) = path.parent().map(|dir| dir.join("Canoa.cfg")) else {
        return false;
    };
    if path.symlink_metadata().is_ok() || !old.is_file() {
        return false;
    }
    // A hard link fails where the name is taken, which a rename would replace.
    let renamed = std::fs::hard_link(&old, path).and_then(|()| std::fs::remove_file(&old));
    match renamed {
        Ok(()) => {
            say(&format!(
                "canoa: renamed {} to {}",
                old.display(),
                path.display()
            ));
            true
        }
        Err(error) => {
            say(&format!(
                "canoa: {} was not renamed to canoa.cfg: {error}",
                old.display()
            ));
            false
        }
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
    fn usage(why: impl ToString) -> Self {
        Self {
            status: 2,
            why: why.to_string(),
        }
    }

    fn run(why: impl ToString) -> Self {
        Self {
            status: 1,
            why: why.to_string(),
        }
    }
}

fn config_path(args: &ConfigArgs, front: &dyn Front) -> Result<PathBuf, Failed> {
    args.config
        .clone()
        .or_else(|| default_config(front))
        .ok_or_else(|| Failed::usage("no canoa.cfg: give --config or CANOA_CONFIG"))
}

fn load(args: &ConfigArgs, front: &dyn Front) -> Result<Config, Failed> {
    Config::load(&config_path(args, front)?).map_err(Failed::usage)
}

/// canoa.cfg for `serve`. With `--allow-empty`, a file that does not exist names nothing, as an empty one does: a person
/// who has written none yet has a host to run all the same.
fn load_for_serving(args: &ServeArgs, front: &dyn Front) -> Result<Config, Failed> {
    let path = config_path(&args.config, front)?;
    match Config::load(&path) {
        Err(crate::ConfigError::Read { source, .. })
            if args.allow_empty && source.kind() == std::io::ErrorKind::NotFound =>
        {
            Config::from_str_in("", path.parent().unwrap_or_else(|| Path::new("")))
                .map_err(Failed::usage)
        }
        loaded => loaded.map_err(Failed::usage),
    }
}

fn home_of(front: &dyn Front) -> Result<Home, Failed> {
    match front.home() {
        Some(dir) => Ok(Home::at(dir)),
        None => Home::of_user().map_err(Failed::run),
    }
}

fn store(home: &Home) -> Result<coqui_peer_webcrypto::TrustStore, Failed> {
    home.store().map_err(Failed::run)
}

fn print(front: &dyn Front, line: &str) -> Result<(), Failed> {
    front.print(line).map_err(Failed::run)
}

/// `[host.web]` with `--web` and `--web-listen` over it; `None` when neither the file nor the command line asks for it.
fn web_config(
    config: &Config,
    args: &ServeArgs,
    home: &Home,
    front: &dyn Front,
) -> Result<Option<WebConfig>, Failed> {
    let mut web = config.web.clone();
    if let Some(dir) = &args.web {
        let web = web.get_or_insert_with(WebConfig::new);
        if let Some(dir) = dir {
            web.dir = Some(std::path::absolute(dir).unwrap_or_else(|_| dir.clone()));
        }
    }
    if let Some(listen) = args.web_listen {
        web.as_mut()
            .ok_or_else(|| Failed::usage("--web-listen needs --web or [host.web]"))?
            .listen = listen;
    }
    if let Some(web) = web.as_mut() {
        // The site may hold neither canoa.cfg nor this user's identity and trust store.
        let file = args.config.config.clone().or_else(|| default_config(front));
        web.keep_out.extend(
            file.and_then(|file| std::path::absolute(file).ok())
                .and_then(|file| file.parent().map(PathBuf::from)),
        );
        web.keep_out.push(home.dir().to_owned());
    }
    Ok(web)
}

fn serve(args: &ServeArgs, front: &dyn Front) -> Result<(), Failed> {
    let config = load_for_serving(args, front)?;
    let home = home_of(front)?;
    let web = web_config(&config, args, &home, front)?;
    // Before anything of the home is touched or any address bound.
    let _lock = home.lock(front.holder()).map_err(|error| match error {
        LockError::Held { .. } => Failed {
            status: STATUS_HELD,
            why: error.to_string(),
        },
        _ => Failed::run(error),
    })?;
    let identity = args.method.identity(&home)?;
    let peer = identity.peer_id().to_string();
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let mut options = ServeOptions::new(program.path)
        .with_program_args(program.args)
        .with_rekey(!args.no_rekey)
        .with_nothing_to_serve(args.allow_empty);
    if let Some(bytes) = args.key_limit {
        options = options.with_key_limit(bytes);
    }
    let serving =
        Serving::bind(&config, identity, store(&home)?, &options).map_err(|error| match error {
            crate::ServeError::NothingToServe => Failed::usage(error),
            _ => Failed::run(error),
        })?;
    let web = web
        .map(|web| WebServer::bind(&web, &serving))
        .transpose()
        .map_err(|error| match error {
            WebError::Listen { .. } => Failed::run(error),
            _ => Failed::usage(error),
        })?;
    for listening in serving.listening() {
        let line = serde_json::json!({
            "name": listening.name,
            "kind": listening.kind,
            "url": listening.url,
            "peer": peer,
            "method": serving.method().name(),
        });
        print(front, &line.to_string())?;
    }
    if let Some(web) = &web {
        print(front, &serde_json::json!({ "web": web.url() }).to_string())?;
    }
    print(
        front,
        &serde_json::json!({
            "ready": true,
            "peer": peer,
            "method": serving.method().name(),
        })
        .to_string(),
    )?;
    if args.until_stdin_closes {
        let stopper = serving.stopper();
        let web_stopper = web.as_ref().map(WebServer::stopper);
        front
            .when_input_closes(
                0,
                Box::new(move || {
                    // The web app is stopped whatever happens to the rest, or `serve` would wait on it for ever.
                    let _ =
                        std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| stopper.stop()));
                    if let Some(web) = web_stopper {
                        web.stop();
                    }
                }),
            )
            .map_err(Failed::run)?;
    }
    let web = web
        .map(|web| {
            std::thread::Builder::new()
                .name("canoa-web".to_owned())
                .spawn(move || web.run())
        })
        .transpose()
        .map_err(Failed::run)?;
    serving.run();
    if let Some(web) = web {
        let _ = web.join();
    }
    Ok(())
}

/// Connections pairing at once; past it a new one is dropped.
const MAX_PAIRING_RUNS: usize = 16;
/// How long one connection may take to pair.
const PAIRING_RUN: Duration = Duration::from_secs(60);

/// No browser sends this as its `Origin`, so the pairing listener takes only clients that send none: native programs.
const NO_BROWSER: &str = "-";

fn pair(
    listen: SocketAddr,
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    if !listen.ip().is_loopback() {
        return Err(Failed::usage("pair listens only on loopback"));
    }
    let home = home_of(front)?;
    let identity = Arc::new(method.identity_as_is(&home)?);
    let store = Arc::new(store(&home)?);
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
    let cancelled = Arc::new(AtomicBool::new(false));
    if until_stdin_closes {
        let cancel = Arc::clone(&cancelled);
        front
            .when_input_closes(1, Box::new(move || cancel.store(true, Ordering::SeqCst)))
            .map_err(Failed::run)?;
    }
    let desk = pairing::desk().map_err(Failed::run)?;
    let shown = desk.show_pin().map_err(Failed::run)?;
    let _clear = ClearPin(&desk);
    print(
        front,
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
        if cancelled.load(Ordering::SeqCst) {
            return Err(Failed::run("pairing was cancelled"));
        }
        while let Ok(result) = reports.try_recv() {
            match result {
                Ok(peer) => {
                    return print(front, &serde_json::json!({ "paired": peer }).to_string());
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

fn check(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    let config = load(args, front)?;
    for entry in &config.entries {
        let kind = match entry.kind {
            crate::EntryKind::Agent { .. } => "agent",
            crate::EntryKind::Provider { .. } => "provider",
            crate::EntryKind::Tool { .. } => "tool",
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
        print(front, &line)?;
    }
    Ok(())
}

fn list_peers(format: Format, front: &dyn Front) -> Result<(), Failed> {
    for peer in peers::list(&store(&home_of(front)?)?).map_err(Failed::run)? {
        let (state, source) = match peer.state {
            PeerState::Accepted => ("accepted", None),
            PeerState::Waiting { source } => ("waiting", source),
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
        print(front, &line)?;
    }
    Ok(())
}

fn peers_failure(error: peers::PeersError) -> Failed {
    match error {
        peers::PeersError::Store(_) => Failed::run(error),
        _ => Failed::usage(error),
    }
}

/// Runs one command to its end.
///
/// # Errors
///
/// Why it failed, and the status it ends with.
pub fn run(cli: Cli, front: &dyn Front) -> Result<(), Failed> {
    match cli.command {
        Command::Serve(args) => serve(&args, front),
        Command::Check(args) => check(&args, front),
        Command::Id { method } => print(
            front,
            &method
                .identity_as_is(&home_of(front)?)?
                .peer_id()
                .to_string(),
        ),
        Command::Peers { format } => list_peers(format, front),
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
            front,
        ),
        Command::Accept { peer } => {
            let id = peers::accept(&store(&home_of(front)?)?, &peer).map_err(peers_failure)?;
            print(front, &format!("accepted {id}"))
        }
        Command::Revoke { peer } => {
            if peers::revoke(&store(&home_of(front)?)?, &peer).map_err(peers_failure)? {
                print(front, "removed")
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
                HostLine::read(&mut std::io::stdin().lock(), crate::serve::MAX_LINE)
                    .map_err(Failed::run)?
            } else {
                HostLine::default()
            };
            let origin = match (library, source) {
                (Some(library), None) if line.source_settings.is_none() => Origin::Library(library),
                (None, Some(file)) => {
                    let settings = line.source_settings.unwrap_or_default();
                    let import_search = import_search
                        .as_deref()
                        .and_then(ImportSearch::named)
                        .unwrap_or_else(|| ImportSearch::default_for(&settings));
                    Origin::Source {
                        file,
                        settings,
                        import_search,
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
        Command::Launch { program, args } => {
            let error = crate::profile::launch(&program, &args, crate::serve::MAX_LINE);
            Err(match error.kind() {
                std::io::ErrorKind::InvalidData => Failed::usage(error),
                _ => Failed::run(error),
            })
        }
    }
}

impl Cli {
    /// Whether this is one of the commands a host starts for itself: `provider`, `tool` or `launch`. An application that
    /// runs as one when it starts itself takes nothing else on that command line.
    #[must_use]
    pub const fn is_a_hosts_own(&self) -> bool {
        matches!(
            self.command,
            Command::Provider { .. } | Command::Tool { .. } | Command::Launch { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("canoa-config-name-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_legacy_name_is_renamed_once_when_nothing_has_the_new_one() {
        let dir = scratch("rename");
        std::fs::write(dir.join("Canoa.cfg"), "[host]\n").expect("written");
        assert!(adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()));
        assert_eq!(
            std::fs::read_to_string(dir.join("canoa.cfg")).expect("read"),
            "[host]\n"
        );
        assert!(!dir.join("Canoa.cfg").exists());
        assert!(
            !adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()),
            "nothing is left to rename"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_legacy_name_never_replaces_the_new_one() {
        let dir = scratch("keep");
        std::fs::write(dir.join("Canoa.cfg"), "old").expect("written");
        std::fs::write(dir.join("canoa.cfg"), "new").expect("written");
        assert!(!adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()));
        assert_eq!(
            std::fs::read_to_string(dir.join("canoa.cfg")).expect("read"),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("Canoa.cfg")).expect("read"),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_canoa_cfg_that_exists_is_read_by_the_new_name_on_windows() {
        let dir = scratch("windows");
        std::fs::write(dir.join("Canoa.cfg"), "[host]\n").expect("written");
        assert!(
            !adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()),
            "nothing to rename: it is the same file"
        );
        Config::load(&dir.join("canoa.cfg")).expect("the file with the old spelling loads");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .expect("listed")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["Canoa.cfg"], "it keeps the name it was given");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn serve_takes_a_config_flag() {
        let cli = Cli::try_parse_from(["canoa", "serve", "--config", "x.cfg"]).expect("parses");
        let Command::Serve(args) = cli.command else {
            panic!("serve");
        };
        assert_eq!(args.config.config, Some(PathBuf::from("x.cfg")));
        assert!(!args.until_stdin_closes);
    }

    #[test]
    fn serve_takes_the_web_app_with_or_without_a_directory() {
        let parsed = |args: &[&str]| {
            let cli = Cli::try_parse_from(["canoa", "serve"].iter().chain(args)).expect("parses");
            let Command::Serve(args) = cli.command else {
                panic!("serve");
            };
            (args.web, args.web_listen)
        };
        assert_eq!(parsed(&[]), (None, None));
        assert_eq!(parsed(&["--web"]), (Some(None), None));
        assert_eq!(parsed(&["--web", "--config", "x.cfg"]), (Some(None), None));
        assert_eq!(
            parsed(&["--web", "site", "--web-listen", "127.0.0.1:0"]),
            (
                Some(Some(PathBuf::from("site"))),
                Some("127.0.0.1:0".parse().expect("an address"))
            )
        );
        assert!(Cli::try_parse_from(["canoa", "serve", "--web-listen", "nowhere"]).is_err());
    }

    #[test]
    fn a_provider_needs_its_library_and_id() {
        assert!(Cli::try_parse_from(["canoa", "provider", "--library", "x"]).is_err());
        let cli = Cli::try_parse_from([
            "canoa",
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
        assert_eq!(max_sessions, crate::config::DEFAULT_MAX_SESSIONS);
    }

    #[test]
    fn only_what_a_host_runs_for_itself_counts_as_its_own() {
        let own = |args: &[&str]| {
            Cli::try_parse_from(["canoa"].iter().chain(args))
                .expect("parses")
                .is_a_hosts_own()
        };
        assert!(own(&["launch", "x"]));
        assert!(own(&["tool", "--name", "t", "--spec", "{}"]));
        assert!(own(&["provider", "--library", "x", "--provider", "p"]));
        assert!(!own(&["serve"]));
        assert!(!own(&["peers"]));
    }

    #[test]
    fn serve_takes_allow_empty() {
        let cli = Cli::try_parse_from(["canoa", "serve", "--allow-empty"]).expect("parses");
        let Command::Serve(args) = cli.command else {
            panic!("serve");
        };
        assert!(args.allow_empty);
    }

    #[test]
    fn accept_needs_a_peer() {
        assert!(Cli::try_parse_from(["canoa", "accept"]).is_err());
    }
}
