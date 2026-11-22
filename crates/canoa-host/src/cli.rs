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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use coqui_link::auth;
use coqui_link::auth::Method;

mod bridge;

use crate::accounts::commands::Env as AccountsEnv;
use crate::config::{ImportSearch, Layers};
use crate::home::{Home, LockError};
use crate::{
    Config, HostLine, Origin, PeerState, ProviderOptions, ServeOptions, Serving, WebConfig,
    WebError, WebServer, peers,
};

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

const AFTER_HELP: &str = "\
Environment:
  CANOA_CONFIG             canoa.cfg to read, below --config and above the platform's default
  CANOA_LOG                what to log on standard error, as a tracing filter (default: info)
  COQUI_AGENTIC_PEER_HOME  the directory of this host's identity and trust store (default: .canoa in the user's home folder)
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
    /// Sign people in with their accounts on this machine and give each their own host, as them: the web app for a machine
    /// several people use. Binds loopback only, for a proxy that speaks TLS; prints {"portal": URL}.
    #[cfg(all(unix, feature = "portal"))]
    Portal(PortalArgs),
    /// Serve every entry of canoa.cfg's [host] table until stopped; prints one JSON line per entry, then {"ready"}.
    /// Only one host runs on a home: a second says which is running and ends with status 3.
    Serve(Box<ServeArgs>),
    /// Check canoa.cfg's [host] table and print its entries, serving nothing.
    Check(ConfigArgs),
    /// Stay resident and hold the Office add-in's address while no Canoa runs, so that a task pane opened then finds a
    /// page that offers to start Canoa. It lets go of the address as soon as a host holds the home, and starts the host
    /// `native install` recorded (else `serve`) when the page asks. Needs [host.web] with office and office_local.
    /// `--install` makes it start when this user signs in, `--uninstall` stops that and `--status` says whether it does.
    OfficeLauncher(LauncherArgs),
    /// Print the canoa.cfg files in the order they are read, lowest layer first, and the one that is written.
    Layers(ConfigArgs),
    /// Print this host's peer id, which a client pins.
    /// A home a host is running on keeps its method: naming another is refused unless --force.
    Id {
        #[command(flatten)]
        method: MethodArg,
        #[command(flatten)]
        force: ForceArg,
        /// Print the peer id of the key in FILE instead, making the key when the file is absent: for a client or a program that
        /// proves itself with a key of its own, as `dial` and `forward` take with --identity.
        #[arg(long, value_name = "FILE", conflicts_with = "force")]
        identity: Option<PathBuf>,
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
    /// Show a PIN and pair the key that uses it, then end. Prints {"pin", "peer", "url"}, then {"paired"}.
    /// One wrong try ends it. A browser is refused unless --origin names it; with --join, pair with a waiting `pair` instead.
    Pair {
        /// The address to wait on; loopback unless you say otherwise.
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: SocketAddr,
        /// How long to wait for the PIN to be used.
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        /// End, the PIN unused, when standard input closes: for a program that runs this as its child.
        #[arg(long)]
        until_stdin_closes: bool,
        /// Pair the browser extension on this computer, through its native messaging host, in place of a native client over
        /// an address: waits on a pipe or socket in the home and prints {"pin", "peer", "method", "native"}. The extension
        /// is bound to the key it pairs.
        #[arg(long)]
        native: bool,
        /// The one browser origin allowed to pair, as scheme://host[:port], such as chrome-extension://ID for an extension's
        /// page; the key it pairs is recorded as that origin's. Without it a browser is refused before its handshake, so a page
        /// cannot spend the PIN; a native client sends no origin.
        #[arg(long = "origin", value_name = "ORIGIN", conflicts_with_all = ["native", "join"])]
        origins: Vec<String>,
        /// Join the `pair` waiting at URL with its PIN, under this host's own identity, and print its peer id. The PIN is read
        /// from --pin-file, else from standard input, never from an argument. The server is kept as one this host dials (`dial
        /// --paired`, `forward --paired`), apart from the keys it serves: it is not let in to this host's entries.
        #[arg(long, value_name = "URL", conflicts_with_all = ["native", "until_stdin_closes"])]
        join: Option<String>,
        /// With --join: a file holding the PIN.
        #[arg(long, requires = "join", value_name = "FILE")]
        pin_file: Option<PathBuf>,
        /// With --join: a PEM bundle the server's certificate must chain to. Without it any certificate is taken, since the PIN
        /// authenticates the peer.
        #[arg(long, requires = "join", value_name = "FILE")]
        ca_file: Option<PathBuf>,
        #[command(flatten)]
        method: MethodArg,
        #[command(flatten)]
        force: ForceArg,
    },
    /// Search this machine and the link for agents, and print each as one line of JSON. What it prints is what they claim.
    Discover {
        /// How long to listen on the link, in seconds.
        #[arg(long, default_value_t = 3)]
        seconds: u64,
    },
    /// Connect out to a peer, for an agent behind NAT: reconnects and keeps the agent running, and ends with its status.
    Dial(Box<coqui_acp_bridge::dial::Dial>),
    /// Forward TCP over a link: listen here for what the far side was asked to reach, and let it reach what is named here. Runs
    /// until stopped, connecting again when the link goes.
    Forward(Box<coqui_acp_bridge::forward::Forward>),
    /// Let a peer this user paired with open a protocol, or with --revoke no longer. It counts at once, on a live link too.
    /// The grants are in the home's grants.json, which `serve --program --paired` reads (a paired key has acp/1 and mcp-offer/1
    /// there, and what is granted here); canoa.cfg's entries and `serve` of canoa.cfg never read them.
    Grant {
        /// The peer id.
        peer: String,
        /// The protocol token, or `tcp/1:HOST:PORT` for a target a forward may reach, which also grants `tcp/1`.
        protocol: String,
        /// Take it away instead.
        #[arg(long)]
        revoke: bool,
    },
    /// Print the `mcpServers` entry that gives an MCP client on this machine a peer's tools, such as a browser's. It holds
    /// nothing secret and stays good across restarts of either side.
    McpConfig {
        /// The peer whose tools to reach, by its id.
        #[arg(long)]
        peer: String,
        /// What the client calls this server.
        #[arg(long, default_value = coqui_acp_bridge::standing::DEFAULT_NAME)]
        name: String,
    },
    /// The stdio MCP server that `mcp-offer/1` declares to an agent: carries its lines to the peer that offered it. With --peer,
    /// the one `mcp-config` prints, which finds that peer's standing stream.
    McpShim {
        /// The peer whose tools to reach, by its id.
        #[arg(long)]
        peer: Option<String>,
    },
    /// Waits on standard input, and ends the agent's process group if it closes with nothing said: what `serve --program
    /// --linger` starts for each agent on Unix, to clean up after a host that went.
    #[command(hide = true)]
    AgentGuard {
        /// The agent's process id.
        pid: u32,
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
    /// Serve one of the bundled agents as an ACP agent on standard input and output, in ACP version 1 or 2, through its
    /// harness provider, with no Node: ID is the agent's id (claude and codex are two). Each session runs the installed
    /// command line; permission prompts are asked through ACP. A setting the provider takes that has no flag of its own is
    /// given with --setting. Notices are logged at info (CANOA_LOG).
    Agent(AgentCommand),
    /// Print what a source offers before a session starts, asked of its provider with no session and no model turn: its
    /// models with their versions and each model's efforts. SOURCE is the id of a bundled agent, run as `canoa agent` runs
    /// it, or with --entry the name of a [host.agents] entry. A source that lists its models only in its session says so.
    Options(OptionsArgs),
    /// Make, list, sign in again to and remove accounts: logins an agent entry may run its sessions on, which keep their
    /// credentials in the account store and never in canoa.cfg.
    Accounts {
        #[command(subcommand)]
        command: AccountsCommand,
    },
    /// Make, list and remove the mail accounts the host's mail tools read: a password rests in the account store and never in
    /// canoa.cfg, which holds the id it rests under.
    Mail {
        #[command(subcommand)]
        command: MailCommand,
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
    /// Run the helper command of a library this host carries, on standard input and output: what the library keeps in a process of
    /// its own so that a crash is that process's (a native library a model runs in, say). `LIBRARY` is the library's name and `ARGS`
    /// what its helper reads. The host starts it; there is nothing for a person to run.
    #[command(hide = true, name = "helper")]
    Helper {
        /// The name of the library whose helper to run.
        library: String,
        /// What the helper reads.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Serve the host's delegation tools as an MCP server on standard input and output, for an agent that takes no HTTP MCP
    /// server: what the host adds to a session's MCP servers. Relays each request to the host's loopback tool server, at
    /// CANOA_DELEGATE_URL, with the bearer in CANOA_DELEGATE_TOKEN; ends when standard input closes.
    #[command(hide = true)]
    McpBridge,
    /// Report what a harness's own hook was told to the host that runs the session, on the channel in a file only that
    /// session's directory holds, and exit with the status that says whether the harness is to stop what it was about to
    /// do (2) or not (0). Prints nothing: what an overlay's hook says never reaches the harness. Reads the harness's payload
    /// from standard input.
    #[command(hide = true, name = "hook-forward")]
    HookForward {
        /// The file that holds the channel's address and secret.
        #[arg(long)]
        channel: PathBuf,
        /// Stop what the harness was about to do when the host cannot be asked: an overlay's hook that guards it is on.
        #[arg(long)]
        refuse: bool,
        /// What the harness reports: `start`, `pre-compact` or `batch`.
        kind: String,
    },
    /// Let a browser's extension reach the Canoa on this computer: register, remove or show the native messaging host the
    /// browser starts, for this user only.
    Native {
        #[command(subcommand)]
        action: NativeAction,
    },
    /// What a browser starts as its native messaging host, with the extension's origin as its first argument: relays the
    /// extension's link to the host that holds the home, over native messaging on standard input and output. The program
    /// started with that argument alone runs this.
    #[command(hide = true, name = "native-host")]
    NativeHost {
        /// The extension's origin, as the browser passes it.
        origin: String,
        /// What else the browser passes (`--parent-window=N` on Windows).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
    /// Start the processes this host asks for, over a socket on standard input, until it closes: the host's own process
    /// starter, for a host that runs inside an application whose process handling collects every child that ends.
    #[cfg(unix)]
    #[command(hide = true)]
    Spawner,
}

/// What `canoa native` does.
#[derive(Debug, Subcommand)]
enum NativeAction {
    /// Register this program as the native messaging host `dev.boriken.canoa` for Chrome, Edge and Chromium, for this user.
    /// Prints what it wrote.
    Install(NativeInstall),
    /// Remove what `install` wrote.
    Uninstall(NativeScope),
    /// Show where it is registered, what Start would run, and whether Canoa runs. Prints one JSON object.
    Status(NativeScope),
}

/// A browser `canoa native install` registers for.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum NativeBrowser {
    Chrome,
    Edge,
    Chromium,
    /// Chrome for Testing reads Chrome's place.
    ChromeForTesting,
}

/// Which registration a `native` command is about.
#[derive(Debug, Args)]
struct NativeScope {
    /// The native host's name, `dev.boriken.canoa` unless given. Another name is one the extension does not look for: for
    /// trying a registration without touching the real one.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// A browser's --user-data-dir, where Chromium on Linux and macOS reads native hosts when it is started with one.
    #[arg(long, value_name = "DIR")]
    profile: Vec<PathBuf>,
}

/// `canoa native install`.
#[derive(Debug, Args)]
struct NativeInstall {
    #[command(flatten)]
    scope: NativeScope,
    /// An extension that may start it, besides the ones this version knows.
    #[arg(long, value_name = "ID")]
    extension_id: Vec<String>,
    /// Only these browsers; every one by default.
    #[arg(long, value_enum)]
    browser: Vec<NativeBrowser>,
    /// What the extension's Start button runs, in place of this program's `serve`.
    #[arg(long, value_name = "PROGRAM")]
    launch: Option<PathBuf>,
    /// Its arguments, after `--`.
    #[arg(last = true)]
    launch_args: Vec<String>,
}

/// What `canoa options` takes.
#[derive(Debug, Args)]
struct OptionsArgs {
    /// The id of a bundled agent; with --entry, an entry's name.
    source: String,
    /// SOURCE names an entry of canoa.cfg, not a built-in harness.
    #[arg(long)]
    entry: bool,
    /// For a bundled agent, the program; the variable `canoa agent` reads (COQUI_ACP_ID_EXECUTABLE), or the agent's own name,
    /// without it.
    #[arg(long)]
    executable: Option<OsString>,
    /// With --entry, the entry's account to ask as, so that it lists what that account may use.
    #[arg(long, requires = "entry")]
    account: Option<String>,
    /// With --entry, the seconds a probe session of an agent that lists its models only in a session is waited for.
    #[arg(long, requires = "entry", default_value_t = 20)]
    wait: u64,
    #[command(flatten)]
    config: ConfigArgs,
}

/// What `canoa accounts` does.
#[derive(Debug, Subcommand)]
enum AccountsCommand {
    /// List the accounts of canoa.cfg with their state; never a credential.
    List {
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Make an account by signing in. A secret it asks for is typed without echo, or read from standard input when that is
    /// not a terminal, and never taken from the command line. Appends `[host.accounts.NAME]` to canoa.cfg.
    Add {
        /// The account's name, as an entry's `accounts` names it.
        name: String,
        /// The provider the account is for, as an entry's `harness`.
        #[arg(long)]
        harness: String,
        /// How to sign in, one of the provider's login methods (naming none lists them).
        #[arg(long)]
        method: Option<String>,
        /// An input the method asks before it runs (an email, an issuer), as KEY=VALUE.
        #[arg(long = "input", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
        /// Give up, with the sign-in, when standard input closes: for a program that runs this as its job.
        #[arg(long)]
        until_stdin_closes: bool,
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Sign in again to an account that needs it.
    Login {
        /// The account's name.
        name: String,
        /// How to sign in, when the account's kind has more than one.
        #[arg(long)]
        method: Option<String>,
        /// An input the method asks before it runs, as KEY=VALUE.
        #[arg(long = "input", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
        /// Give up, with the sign-in, when standard input closes.
        #[arg(long)]
        until_stdin_closes: bool,
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Forget an account: its stored credential and its state, and its block in canoa.cfg. Nothing is signed out at the
    /// provider.
    Remove {
        /// The account's name.
        name: String,
        #[command(flatten)]
        config: ConfigArgs,
    },
}

/// What `canoa mail` does.
#[derive(Debug, Subcommand)]
enum MailCommand {
    /// List the mail accounts of canoa.cfg with their state; never a password.
    List {
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Add a mail account: a password it needs is typed without echo, or read from standard input when that is not a terminal,
    /// and never taken from the command line. It rests in the account store; canoa.cfg gets `[host.mail.accounts.NAME]`.
    Add {
        /// The account's name: 1 to 32 lower-case letters, digits, `-` and `_`.
        name: String,
        /// What reaches the mailbox: `ews` (Exchange Server) or `outlook` (classic Outlook for Windows).
        #[arg(long, value_enum)]
        backend: MailBackendArg,
        /// The mailbox's address, which Autodiscover asks about (ews).
        #[arg(long)]
        email: Option<String>,
        /// The EWS address, when Autodiscover is not to find it (ews).
        #[arg(long)]
        server: Option<String>,
        /// The Autodiscover address to ask (ews).
        #[arg(long)]
        autodiscover: Option<String>,
        /// The user name, which signs in with a password (ews); without it the Windows logon is used.
        #[arg(long)]
        user: Option<String>,
        /// The Outlook profile to log on to (outlook).
        #[arg(long)]
        profile: Option<String>,
        /// How long one call may take, in seconds.
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Forget a mail account: its stored password and its table in canoa.cfg. Nothing changes in the mailbox.
    Remove {
        /// The account's name.
        name: String,
        #[command(flatten)]
        config: ConfigArgs,
    },
}

/// The backends `canoa mail add` names.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum MailBackendArg {
    Ews,
    Outlook,
}

/// What `canoa agent` takes: the agent, its program and settings, and what every agent takes the same way.
#[derive(Debug, Args)]
struct AgentCommand {
    /// The agent to serve: the id of a harness provider the bundled agents offer, such as claude or codex.
    id: String,
    /// The agent's program; else the variable COQUI_ACP_ID_EXECUTABLE (ID in upper case), else the copy the Store installed, else
    /// the agent's own name on the PATH.
    #[arg(long)]
    executable: Option<OsString>,
    /// A setting of the agent's provider, KEY=VALUE, given as JSON when it is (`true`, `3`, `["user"]`) and as text when not; repeat
    /// for several. The provider refuses a key it does not declare.
    #[arg(long = "setting", value_name = "KEY=VALUE", value_parser = parse_setting)]
    settings: Vec<(String, serde_json::Value)>,
    #[command(flatten)]
    common: AgentArgs,
}

/// What `canoa agent` takes for every agent.
#[derive(Debug, Args)]
struct AgentArgs {
    /// The model every session starts with, until the client chooses another.
    #[arg(long)]
    model: Option<String>,
    /// The effort every session starts with, such as low or high.
    #[arg(long)]
    effort: Option<String>,
    /// The highest ACP version answered.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    max_acp_version: Option<u8>,
    /// Sessions at once.
    #[arg(long)]
    max_sessions: Option<usize>,
    /// A directory every session's cwd and more directories must lie in; repeat for several. None takes any.
    #[arg(long = "working-root", value_name = "DIRECTORY")]
    working_roots: Vec<PathBuf>,
    /// Refuse a session that a client gives MCP servers.
    #[arg(long)]
    no_client_mcp_servers: bool,
    /// How much a command may reach: none (the default), workspace or read-only. The same for every agent.
    #[arg(long, value_name = "SANDBOX", value_parser = parse_sandbox)]
    sandbox: Option<coqui_acp_harness::Sandbox>,
    /// Take the network from the commands a session runs, where the agent can.
    #[arg(long)]
    no_network: bool,
    /// What is asked first: ask (the default), edits or never. The same for every agent.
    #[arg(long, value_name = "APPROVALS", value_parser = parse_approvals)]
    approvals: Option<coqui_acp_harness::Approvals>,
    /// How long the prompt cache should live: default (the agent's own), short (about five minutes, for one-shot and
    /// background runs) or long (about an hour, for an interactive session). An agent with no such setting ignores it.
    #[arg(long, value_name = "LIFETIME", default_value = "default", value_parser = parse_lifetime)]
    cache_lifetime: coqui_acp_api::CacheLifetime,
    /// The longest ACP message, as one line, either way, in bytes: raise it for a client that sends large attachments.
    #[arg(long, value_name = "BYTES", default_value_t = crate::config::DEFAULT_MAX_MESSAGE_BYTES)]
    max_message_bytes: usize,
}

fn parse_setting(text: &str) -> Result<(String, serde_json::Value), String> {
    let (key, value) = text
        .split_once('=')
        .filter(|(key, _)| !key.is_empty())
        .ok_or_else(|| "KEY=VALUE".to_owned())?;
    let value =
        serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));
    Ok((key.to_owned(), value))
}

fn parse_sandbox(text: &str) -> Result<coqui_acp_harness::Sandbox, String> {
    coqui_acp_harness::Sandbox::from_id(text)
        .ok_or_else(|| "one of none, workspace, read-only".to_owned())
}

fn parse_approvals(text: &str) -> Result<coqui_acp_harness::Approvals, String> {
    coqui_acp_harness::Approvals::from_id(text).ok_or_else(|| "one of ask, edits, never".to_owned())
}

fn parse_lifetime(text: &str) -> Result<coqui_acp_api::CacheLifetime, String> {
    crate::config::cache_lifetime_named(text)
        .ok_or_else(|| format!("one of {}", crate::config::CACHE_LIFETIMES.join(", ")))
}

impl AgentCommand {
    /// What this command line serves.
    fn run(self) -> crate::harness::AgentRun {
        let common = self.common;
        let mut settings: serde_json::Map<String, serde_json::Value> =
            self.settings.into_iter().collect();
        let mut give = |key: &str, value: Option<serde_json::Value>| {
            if let Some(value) = value {
                settings.insert(key.to_owned(), value);
            }
        };
        give("model", common.model.map(Into::into));
        give("effort", common.effort.map(Into::into));
        give("max_acp_version", common.max_acp_version.map(Into::into));
        give("max_sessions", common.max_sessions.map(Into::into));
        give(
            "client_mcp_servers",
            common.no_client_mcp_servers.then_some(false.into()),
        );
        let access_asked =
            common.sandbox.is_some() || common.no_network || common.approvals.is_some();
        let mut access = coqui_acp_harness::Access::STANDARD;
        access.sandbox = common.sandbox.unwrap_or(access.sandbox);
        access.network = !common.no_network;
        access.approvals = common.approvals.unwrap_or(access.approvals);
        crate::harness::AgentRun {
            id: self.id,
            executable: self.executable,
            settings,
            access: Some(access.to_json()),
            access_asked,
            roots: common.working_roots,
            lifetime: common.cache_lifetime,
            max_message_bytes: common.max_message_bytes,
        }
    }
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
    fn identity(self, home: &Home) -> Result<auth::Identity, Failed> {
        home.identity(self.method).map_err(Failed::run)
    }

    /// The identity, for a command that does not hold the home: a method other than the one the home records is refused
    /// while a host runs on it, since that host's next restart would refuse every client already paired.
    fn identity_unless_running(
        self,
        home: &Home,
        force: ForceArg,
    ) -> Result<auth::Identity, Failed> {
        let recorded = home.recorded().map_err(Failed::run)?;
        if let (Some(requested), Some(recorded)) = (self.method, recorded)
            && requested != recorded
            && let Err(LockError::Held { holder, .. }) = home.probe()
        {
            let who = holder.map_or_else(
                || "another host".to_owned(),
                |holder| format!("{} (process {})", holder.name, holder.pid),
            );
            if !force.force {
                return Err(Failed {
                    status: STATUS_HELD,
                    why: format!(
                        "{who} is running on this home, which proves by {recorded}: already-paired clients would be refused after its next restart if the method became {requested}. Stop it first, or add --force to change it anyway"
                    ),
                });
            }
            tracing::info!(
                "--force: method is {requested} while {who} runs on this home, which proves by {recorded}: already-paired clients are refused after its next restart"
            );
        }
        self.identity(home)
    }
}

/// `--force`, for a command that would change the method of a home a host is running on.
#[derive(Clone, Copy, Debug, Args)]
struct ForceArg {
    /// Change the method of a home a host is running on anyway; the host's next restart refuses every client already paired.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    config: ConfigArgs,
    /// Serve the one program after `--` to paired keys over the network, with no canoa.cfg: this host's own proof (--method,
    /// --key-limit, --no-rekey) and the flags below it. The way to give one agent TLS, WebTransport, a linger or a
    /// standing MCP offer.
    #[arg(long)]
    program: bool,
    /// End, with every agent started, when standard input closes: for a program that runs the host as its child.
    #[arg(long)]
    until_stdin_closes: bool,
    /// Say that a supervisor starts this host again when it ends with status 75, which is what lets a paired key ask for
    /// `host/restart`; without it that request is refused.
    #[arg(long)]
    supervised: bool,
    /// With no entry in canoa.cfg, or no canoa.cfg at all, keep running with nothing to serve, rather than ending with
    /// status 2.
    #[arg(long)]
    allow_empty: bool,
    /// Run for a portal: every entry and the control channel listen on a socket under DIR, which only `--portal-user`'s
    /// processes may use, and the web app is not served. For `canoa portal`'s per-user service.
    #[arg(long, value_name = "DIR")]
    portal: Option<PathBuf>,
    /// The portal's user, a name in /etc/passwd or a number: the only user whose processes may use those sockets.
    #[arg(long, value_name = "USER", requires = "portal")]
    portal_user: Option<String>,
    /// A group whose members may have a host run for a portal; this account must be in one (the portal's own `groups`).
    /// Fixed in the unit, so that an account outside them is never given a host.
    #[arg(long, value_name = "GROUP", requires = "portal")]
    portal_group: Vec<String>,
    /// The lowest user id a host run for a portal may have; root, `nobody` and the system ranges never may.
    #[arg(long, value_name = "UID", default_value_t = 1000, requires = "portal")]
    portal_min_uid: u32,
    /// Serve the host's own sources too (every bundled agent's), to this process only: they have no address,
    /// and each line says what a source is missing.
    #[arg(long)]
    local_sources: bool,
    /// Serve the web app too, on [host.web]'s address (default 127.0.0.1:7800): from DIR, else [host.web]'s dir, else the
    /// site built in (feature `web`).
    #[arg(long, value_name = "DIR", num_args = 0..=1)]
    web: Option<Option<PathBuf>>,
    /// The web app's address, in place of [host.web]'s listen. Beyond loopback it needs [host.web]'s cert and key.
    #[arg(long, value_name = "ADDRESS")]
    web_listen: Option<SocketAddr>,
    /// Which of the library's flags the command line gave. Before them, since each field takes its values out of the matches
    /// as it is read.
    #[command(flatten)]
    given: bridge::Given,
    /// The proof's method and session settings, which both forms of `serve` take, and with --program the program's own flags.
    #[command(flatten)]
    bridge: coqui_acp_bridge::serve::Serve,
}

#[derive(Debug, Args)]
struct LauncherArgs {
    #[command(flatten)]
    config: ConfigArgs,
    /// Register `office-launcher` (with the --config named here) to start when this user signs in: a Run value on Windows, a
    /// LaunchAgent on macOS, an autostart entry elsewhere. Needs no elevation; running it again replaces the entry.
    #[arg(long, conflicts_with_all = ["uninstall", "status"])]
    install: bool,
    /// Delete the entry --install wrote, and nothing else.
    #[arg(long, conflicts_with_all = ["install", "status"])]
    uninstall: bool,
    /// Say whether the entry is registered and whether a launcher answers at the add-in's address.
    #[arg(long, conflicts_with_all = ["install", "uninstall"])]
    status: bool,
    /// Start the launcher again with no console window and end at once, printing its process id: its output goes to
    /// launcher.log in the home's `office` folder. What the Windows entry runs, since `canoa.exe` is a console program. On
    /// Windows the launcher holds every handle this process held, so a caller that reads a pipe of this process to its end
    /// waits for the launcher to end: have the output go to a file or a terminal.
    #[arg(long, conflicts_with_all = ["install", "uninstall", "status"])]
    detach: bool,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    /// canoa.cfg files, a list split at `;` on Windows and `:` elsewhere, the last one highest. A leading `+` puts them on
    /// top of the installed, system and user files; without it they are the only ones read. By default those three layers,
    /// the user's at %APPDATA%\Canoa\canoa.cfg or $XDG_CONFIG_HOME/Canoa/canoa.cfg (~/.config without it).
    #[arg(long, env = "CANOA_CONFIG")]
    config: Option<String>,
    /// How `check` prints entries and `options` its choices.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
}

/// The files canoa.cfg is read from. With none named, the person's own is the user's, and a `Canoa.cfg` there with no
/// `canoa.cfg` beside it is renamed first, on the platforms whose file names are case sensitive.
fn layers_of(args: &ConfigArgs, front: &dyn Front, must_exist: bool) -> Result<Layers, Failed> {
    let named = args.config.as_deref().filter(|value| !value.is_empty());
    if named.is_none()
        && !cfg!(windows)
        && let Ok(user) = anycfg::user_config_path(Some(Path::new(crate::config::layers::NAME)))
    {
        adopt_legacy_name(&user, &|line| front.say(line));
    }
    Layers::resolve(named, must_exist).map_err(Failed::usage)
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

fn load(args: &ConfigArgs, front: &dyn Front) -> Result<Config, Failed> {
    Config::load_layers(&layers_of(args, front, true)?).map_err(Failed::usage)
}

/// canoa.cfg for `serve`. With `--allow-empty`, no file at all names nothing, as an empty one does: a person who has
/// written none yet has a host to run all the same. A host for a portal is always run so: the person's own file is the one
/// their settings are written to, and it does not exist until the first of them.
fn load_for_serving(
    args: &ServeArgs,
    front: &dyn Front,
) -> Result<(Config, crate::Layers), Failed> {
    let lenient = args.allow_empty || args.portal.is_some();
    let layers = layers_of(&args.config, front, !lenient)?;
    let config = match Config::load_layers(&layers) {
        Err(crate::ConfigError::Read { source, .. })
            if lenient && source.kind() == std::io::ErrorKind::NotFound =>
        {
            Config::from_str_in("", layers.own.parent().unwrap_or_else(|| Path::new("")))
                .map_err(Failed::usage)
        }
        loaded => loaded.map_err(Failed::usage),
    }?;
    // An entry an earlier version of the application made for a provider's accounts is folded into the agent's own before the host
    // lists its agents, so the person never sees a second agent for one provider.
    let portal = args.portal.is_some();
    match crate::control::accounts::fold_made_entries(&layers.own, &config, portal) {
        Ok(true) => return Ok((Config::load_layers(&layers).map_err(Failed::usage)?, layers)),
        Ok(false) => {}
        Err((_, why)) => tracing::warn!("canoa.cfg's accounts entry was left as it is: {why}"),
    }
    Ok((config, layers))
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
    if args.portal.is_some() {
        return Ok(None);
    }
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
        if let Ok(layers) = layers_of(&args.config, front, false) {
            web.keep_out.extend(layers.files.iter().filter_map(|file| {
                std::path::absolute(file)
                    .ok()
                    .and_then(|file| file.parent().map(PathBuf::from))
            }));
        }
        web.keep_out.push(home.dir().to_owned());
    }
    Ok(web)
}

/// `canoa mcp-bridge`: the address and the bearer come from the environment, never from the command line.
fn mcp_bridge() -> Result<(), Failed> {
    let var = |name: &str| std::env::var_os(name).and_then(|value| value.into_string().ok());
    let target = crate::delegate::bridge::Target::new(
        var(crate::delegate::bridge::URL_VAR).as_deref(),
        var(crate::delegate::bridge::TOKEN_VAR).as_deref(),
    )
    .map_err(Failed::usage)?;
    crate::delegate::bridge::serve(target, std::io::stdin().lock(), std::io::stdout())
        .map_err(Failed::run)
}

/// What `canoa portal` takes.
#[cfg(all(unix, feature = "portal"))]
#[derive(clap::Args, Debug)]
struct PortalArgs {
    /// The portal's settings, a TOML file; every key has a default.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// The web app's build to serve, in place of the file's `site`.
    #[arg(long, value_name = "DIR")]
    site: Option<PathBuf>,
    /// A loopback TCP address to listen on too, in place of the file's `listen`: for development, since any local user can reach
    /// it and what it says of the browser's address is not believed.
    #[arg(long, value_name = "ADDRESS")]
    listen: Option<SocketAddr>,
    /// The Unix socket a proxy reaches the portal at, in place of the file's `socket`.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
    /// Something to do for the portal's accounts instead of serving.
    #[command(subcommand)]
    action: Option<PortalAction>,
}

/// What `canoa portal` does besides serving. Run as the account the portal runs as, which owns the files it keeps.
#[cfg(all(unix, feature = "portal"))]
#[derive(clap::Subcommand, Debug)]
enum PortalAction {
    /// The one-time codes the portal keeps: who has one, setting one up for a person, taking one away.
    Factor {
        #[command(subcommand)]
        action: FactorAction,
    },
}

/// `canoa portal factor`.
#[cfg(all(unix, feature = "portal"))]
#[derive(clap::Subcommand, Debug)]
enum FactorAction {
    /// Print one line for each account that has a code set up or a setup waiting for its first code. No secret is printed.
    List,
    /// Begin a setup for NAME and print its link and recovery codes, once. The person (or you) confirms it with the first code
    /// from the app: run this again with `--code`. Nothing is a factor until then.
    Enrol {
        /// The account, by name.
        name: String,
        /// The first code from the app: confirms the setup that is waiting.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
    },
    /// Clear what the portal counted of NAME's failed second-step tries, and the provider's delay of the account. For a person who
    /// was locked out by someone else's guesses and has changed their password. A running portal acts on it at the account's next
    /// try.
    Unlock {
        /// The account, by name.
        name: String,
    },
    /// Remove an account's code, its recovery codes and any setup waiting. The sessions made with the code end at their next use,
    /// and the account is marked: it sets no code up for itself until you set one up with `enrol`. Change the account's
    /// password too: a code that had to be reset may have been lost with the password.
    Reset {
        /// The account, by name.
        #[arg(required_unless_present = "id")]
        name: Option<String>,
        /// The code's id as `list` shows it (`1000:alice`), for an account that is gone.
        #[arg(long, value_name = "ID", conflicts_with = "name")]
        id: Option<String>,
    },
}

/// The settings `canoa portal` runs with: the file, then the command line over it.
#[cfg(all(unix, feature = "portal"))]
fn portal_settings(args: &PortalArgs) -> Result<crate::portal::Settings, Failed> {
    use crate::portal::Settings;
    let mut settings = match &args.config {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .map_err(|error| Failed::usage(format!("{}: {error}", file.display())))?;
            Settings::from_toml(&text).map_err(Failed::usage)?
        }
        None => Settings::default(),
    };
    if let Some(site) = &args.site {
        settings.site = Some(site.clone());
    }
    if let Some(listen) = args.listen {
        settings.listen = Some(listen);
    }
    if let Some(socket) = &args.socket {
        settings.socket = Some(socket.clone());
    }
    Ok(settings)
}

/// `canoa portal factor`: an administrator's way to set up, list and reset one-time codes, in the files the portal reads.
#[cfg(all(unix, feature = "portal"))]
fn portal_factor(
    settings: &crate::portal::Settings,
    action: &FactorAction,
    front: &dyn Front,
) -> Result<(), Failed> {
    use crate::portal::{Accounts as _, FactorError, FactorStanding};
    // Before anything is opened: a code file made by another account is one the portal cannot read.
    crate::portal::check_code_files_owned(settings).map_err(Failed::usage)?;
    let access = crate::portal::Access::new(settings).map_err(Failed::usage)?;
    if access.factor_standing("", 0) == FactorStanding::Unsupported {
        return Err(Failed::usage(
            "no authenticator keeps one-time codes: name a provider that does (`coqui_access_totp`) in `authenticators`",
        ));
    }
    let failed = |error: FactorError| match error {
        FactorError::Unsupported => Failed::usage("this portal keeps no one-time codes"),
        FactorError::NothingWaiting => Failed::run(
            "no setup is waiting for NAME, or it timed out: run `factor enrol NAME` without `--code` first",
        ),
        FactorError::Wrong => Failed::run("that code was not accepted"),
        FactorError::Failed(why) => Failed::run(why),
    };
    match action {
        FactorAction::List => {
            for entry in access.list_factors().map_err(Failed::run)? {
                print(
                    front,
                    &serde_json::json!({
                        "id": entry.id,
                        "user": entry.user,
                        "enrolled": entry.enrolled,
                        "disabled": entry.disabled,
                        "waiting": entry.waiting,
                        "reset": entry.reset,
                    })
                    .to_string(),
                )?;
            }
            Ok(())
        }
        FactorAction::Enrol { name, code } => {
            let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
            if let Some(code) = code {
                access
                    .confirm_factor(&user, uid, code.trim())
                    .map_err(failed)?;
                // An administrator set it up: the account is no longer one that is waiting for them.
                access.clear_reset(&user, uid).map_err(Failed::run)?;
                return print(
                    front,
                    &serde_json::json!({"enrolled": {"user": user}}).to_string(),
                );
            }
            let begun = access.begin_factor(&user, uid, false).map_err(failed)?;
            print(
                front,
                &serde_json::json!({"enrol": {
                    "user": user,
                    "uri": begun.uri,
                    "recovery_codes": begun.codes,
                }})
                .to_string(),
            )
        }
        FactorAction::Unlock { name } => {
            let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
            access.unlock(&user, uid).map_err(Failed::run)?;
            print(
                front,
                &serde_json::json!({"unlock": {"user": user}}).to_string(),
            )
        }
        FactorAction::Reset { name, id } => {
            let (shown, had) = match (name, id) {
                (_, Some(id)) => (id.clone(), access.reset_by_id(id).map_err(Failed::usage)?),
                (Some(name), None) => {
                    let (user, uid) = access.admitted(name).map_err(Failed::usage)?;
                    (
                        user.clone(),
                        access.reset_factor(&user, uid).map_err(failed)?,
                    )
                }
                (None, None) => return Err(Failed::usage("name an account or `--id`")),
            };
            print(
                front,
                &serde_json::json!({"reset": {"account": shown, "had_one": had}}).to_string(),
            )
        }
    }
}

#[cfg(all(unix, feature = "portal"))]
fn portal(args: &PortalArgs, front: &dyn Front) -> Result<(), Failed> {
    use crate::portal::{Portal, Systemd};
    let settings = portal_settings(args)?;
    if let Some(PortalAction::Factor { action }) = &args.action {
        return portal_factor(&settings, action, front);
    }
    let (socket, tcp) = (settings.socket.clone(), settings.listen);
    let accounts = crate::portal::Access::new(&settings).map_err(Failed::usage)?;
    let mut controller = Systemd::new();
    controller.systemctl.clone_from(&settings.systemctl);
    controller.elevate = settings.elevate.iter().map(Into::into).collect();
    controller.unit.clone_from(&settings.unit);
    let portal = Portal::bind(
        settings,
        std::sync::Arc::new(accounts),
        std::sync::Arc::new(controller),
    )
    .map_err(|error| match error {
        crate::portal::PortalError::Listen(_) => Failed::run(error),
        _ => Failed::usage(error),
    })?;
    let line = serde_json::json!({
        "portal": {
            "socket": socket,
            "listen": portal.local_addr().or(tcp).map(|address| address.to_string()),
        }
    });
    print(front, &line.to_string())?;
    portal.run();
    Ok(())
}

fn serve(args: &ServeArgs, front: &dyn Front) -> Result<(), Failed> {
    if args.program {
        if args.bridge.agent.is_empty() {
            return Err(Failed::usage(
                "--program needs the program to serve, after --",
            ));
        }
        return bridge::serve_program(args, front);
    }
    bridge::refuse_program_flags(args)?;
    // Before anything of the home is touched: a host for a portal runs only as the account whose folder it is.
    if let Some(dir) = &args.portal {
        crate::portal_link::check_account(dir, args.portal_min_uid, &args.portal_group)
            .map_err(Failed::usage)?;
    }
    let (config, layers) = load_for_serving(args, front)?;
    crate::addresses::log(&config);
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
    let identity = MethodArg {
        method: args.bridge.method.method,
    }
    .identity(&home)?;
    let peer = identity.peer_id().to_string();
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let mut options = ServeOptions::new(program.path)
        .with_program_args(program.args)
        .with_rekey(!args.bridge.session.no_rekey)
        .with_nothing_to_serve(args.allow_empty || args.portal.is_some())
        .with_local_sources(args.local_sources || args.portal.is_some())
        .with_supervised(args.supervised)
        .with_layers(layers)
        .with_state_dir(home.dir());
    if let Ok(layers) = layers_of(&args.config, front, false) {
        options = options.with_config_file(layers.own);
    }
    if let Some(bytes) = args.bridge.session.key_limit {
        options = options.with_key_limit(bytes);
    }
    if let Some((programs, providers)) = front.programs() {
        options = options.with_programs(programs, providers);
    }
    if let Some(dir) = &args.portal {
        let user = args
            .portal_user
            .as_deref()
            .ok_or_else(|| Failed::usage("--portal needs --portal-user"))?;
        let uid = crate::portal_link::uid_of(user)
            .map_err(|error| Failed::usage(format!("--portal-user: {error}")))?;
        options = options.with_portal(crate::portal_link::PortalSockets::new(dir, uid));
    }
    if let Ok(layers) = layers_of(&args.config, front, false) {
        options = options.with_account_layers(layers);
    }
    let serving =
        Serving::bind(&config, identity, store(&home)?, &options).map_err(|error| match error {
            crate::ServeError::NothingToServe => Failed::usage(error),
            _ => Failed::run(error),
        })?;
    front.local(serving.local());
    let web = web
        .map(|web| WebServer::bind(&web, &serving))
        .transpose()
        .map_err(|error| match error {
            WebError::Listen { .. } => Failed::run(error),
            _ => Failed::usage(error),
        })?;
    for listening in serving.listening() {
        let mut line = listening.to_json();
        line.insert("peer".into(), peer.clone().into());
        line.insert("method".into(), serving.method().name().into());
        let line = serde_json::Value::Object(line);
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
    let web_stopper = web.as_ref().map(WebServer::stopper);
    let web = web
        .map(|web| {
            std::thread::Builder::new()
                .name("canoa-web".to_owned())
                .spawn(move || web.run())
        })
        .transpose()
        .map_err(Failed::run)?;
    let restart = serving.restart_flag();
    serving.run();
    let restarting = restart.load(Ordering::SeqCst);
    if restarting && let Some(web) = &web_stopper {
        web.stop();
    }
    if let Some(web) = web {
        let _ = web.join();
    }
    if restarting {
        return Err(Failed {
            status: STATUS_RESTART,
            why: "a paired key asked this host to restart".to_owned(),
        });
    }
    Ok(())
}

/// `canoa native`.
fn native(action: NativeAction, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    match action {
        NativeAction::Install(args) => {
            let program = std::env::current_exe().map_err(Failed::run)?;
            let launch = match args.launch {
                Some(program) => Some(crate::native::Launch {
                    program,
                    args: args.launch_args,
                }),
                None => front.program().ok().map(|program| crate::native::Launch {
                    program: program.path,
                    args: program
                        .args
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .chain(["serve".to_owned()])
                        .collect(),
                }),
            };
            let browsers: Vec<_> = args
                .browser
                .iter()
                .map(|browser| match browser {
                    NativeBrowser::Chrome | NativeBrowser::ChromeForTesting => {
                        coqui_agentic_peer::companion::Browser::Chrome
                    }
                    NativeBrowser::Edge => coqui_agentic_peer::companion::Browser::Edge,
                    NativeBrowser::Chromium => coqui_agentic_peer::companion::Browser::Chromium,
                })
                .collect();
            let options = crate::native::Install {
                name: args.scope.name,
                extensions: args.extension_id,
                browsers,
                launch,
                profiles: args.scope.profile,
            };
            for written in crate::native::install(&home, &program, &options).map_err(Failed::run)? {
                print(front, &written)?;
            }
            Ok(())
        }
        NativeAction::Uninstall(scope) => {
            crate::native::uninstall(&home, scope.name.as_deref(), &scope.profile)
                .map_err(Failed::run)
        }
        NativeAction::Status(scope) => {
            let status = crate::native::status(&home, scope.name.as_deref(), &scope.profile)
                .map_err(Failed::run)?;
            print(
                front,
                &serde_json::json!({
                    "name": scope.name.as_deref().unwrap_or(crate::native::HOST_NAME),
                    "registered": status.registered,
                    "launch": status.launch,
                    "running": status.running,
                    "door_error": status.door_error,
                    "holder": status.holder.map(|holder| serde_json::json!({ "name": holder.name, "pid": holder.pid })),
                })
                .to_string(),
            )
        }
    }
}

/// What a browser starts: relays native messages on this process's standard streams to the host of the home.
fn native_host(origin: &str, front: &dyn Front) -> Result<(), Failed> {
    let home = home_of(front)?;
    crate::native::relay(
        &home,
        origin,
        &mut std::io::stdin().lock(),
        std::io::stdout(),
        || std::process::exit(0),
    )
    .map_err(Failed::run)
}

/// Runs `door` to its end for `canoa pair`: prints what is shown, then `{"paired"}`.
fn pair_at(
    door: crate::pair_run::Door,
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    let home = home_of(front)?;
    let identity = Arc::new(method.identity_unless_running(&home, force)?);
    let store = Arc::new(store(&home)?);
    let cancelled = Arc::new(AtomicBool::new(false));
    if until_stdin_closes {
        let cancel = Arc::clone(&cancelled);
        front
            .when_input_closes(1, Box::new(move || cancel.store(true, Ordering::SeqCst)))
            .map_err(Failed::run)?;
    }
    let run = crate::pair_run::Run {
        identity,
        store,
        home: home.dir().to_owned(),
        timeout,
        cancelled,
    };
    let peer = crate::pair_run::run(door, &run, &mut |shown| {
        front
            .print(&shown.to_string())
            .map_err(|error| error.to_string())
    })
    .map_err(|fail| match fail {
        crate::pair_run::Fail::Usage(why) => Failed::usage(why),
        crate::pair_run::Fail::Run(why) => Failed::run(why),
    })?;
    print(front, &serde_json::json!({ "paired": peer }).to_string())
}

/// `pair --native`: the same PIN pairing, on a pipe in the home that only the extension's native host reaches, for this user.
fn pair_native(
    timeout: Duration,
    until_stdin_closes: bool,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    pair_at(
        crate::pair_run::Door::Native,
        timeout,
        until_stdin_closes,
        method,
        force,
        front,
    )
}

fn pair(
    listen: SocketAddr,
    timeout: Duration,
    until_stdin_closes: bool,
    origins: Vec<String>,
    method: MethodArg,
    force: ForceArg,
    front: &dyn Front,
) -> Result<(), Failed> {
    if origins.len() > 1
        || origins
            .iter()
            .any(|origin| !coqui_acp_bridge::valid_origin(origin))
    {
        return Err(Failed::usage(
            "--origin is one origin, scheme://host or scheme://host:port with no path, and never null",
        ));
    }
    pair_at(
        crate::pair_run::Door::Network {
            listen,
            origin: origins.into_iter().next(),
        },
        timeout,
        until_stdin_closes,
        method,
        force,
        front,
    )
}

/// canoa.cfg for a command that may make it: a file that does not exist names nothing.
fn account_env<'a>(args: &ConfigArgs, front: &'a dyn Front) -> Result<AccountsEnv<'a>, Failed> {
    let layers = layers_of(args, front, false)?;
    let config_path = layers.own.clone();
    let config = match Config::load_layers(&layers) {
        Err(crate::ConfigError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Config::from_str_in("", config_path.parent().unwrap_or_else(|| Path::new("")))
                .map_err(Failed::usage)?
        }
        loaded => loaded.map_err(Failed::usage)?,
    };
    Ok(AccountsEnv {
        front,
        config_path,
        config,
        home: home_of(front)?,
    })
}

fn accounts(command: AccountsCommand, front: &dyn Front) -> Result<(), Failed> {
    use crate::accounts::commands as run;
    match command {
        AccountsCommand::List { config } => run::list(&account_env(&config, front)?, config.format),
        AccountsCommand::Add {
            name,
            harness,
            method,
            inputs,
            until_stdin_closes,
            config,
        } => run::add(
            &account_env(&config, front)?,
            &name,
            &harness,
            method.as_deref(),
            &run::inputs_of(&inputs)?,
            (config.format, until_stdin_closes),
        ),
        AccountsCommand::Login {
            name,
            method,
            inputs,
            until_stdin_closes,
            config,
        } => run::login(
            &account_env(&config, front)?,
            &name,
            method.as_deref(),
            &run::inputs_of(&inputs)?,
            (config.format, until_stdin_closes),
        ),
        AccountsCommand::Remove { name, config } => {
            run::remove(&account_env(&config, front)?, &name)
        }
    }
}

fn mail(command: MailCommand, front: &dyn Front) -> Result<(), Failed> {
    use crate::mail::commands as run;
    match command {
        MailCommand::List { config } => run::list(&account_env(&config, front)?, config.format),
        MailCommand::Add {
            name,
            backend,
            email,
            server,
            autodiscover,
            user,
            profile,
            timeout_secs,
            config,
        } => run::add(
            &account_env(&config, front)?,
            &name,
            &run::NewAccount {
                backend: Some(match backend {
                    MailBackendArg::Ews => crate::MailBackend::Ews,
                    MailBackendArg::Outlook => crate::MailBackend::Outlook,
                }),
                email,
                server,
                autodiscover,
                user,
                profile,
                timeout_secs,
            },
            config.format,
        ),
        MailCommand::Remove { name, config } => run::remove(&account_env(&config, front)?, &name),
    }
}

fn layers(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    let layers = layers_of(args, front, true)?;
    match args.format {
        Format::Json => print(
            front,
            &serde_json::json!({
                "files": layers.files.iter().map(|file| file.display().to_string()).collect::<Vec<_>>(),
                "own": layers.own.display().to_string(),
            })
            .to_string(),
        ),
        Format::Text => {
            for file in &layers.files {
                print(front, &file.display().to_string())?;
            }
            print(front, &format!("own {}", layers.own.display()))
        }
    }
}

/// What `office-launcher` is asked to do about signing in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoginAction {
    Install,
    Uninstall,
    Status,
}

/// The add-in's address from canoa.cfg, which the launcher holds.
fn office_web(args: &ConfigArgs, front: &dyn Front) -> Result<crate::config::WebConfig, Failed> {
    let config = load(args, front)?;
    config
        .web
        .clone()
        .filter(|web| web.office && web.office_local)
        .ok_or_else(|| {
            Failed::usage(
                "canoa.cfg has no [host.web] with office = true and office_local = true: there is no add-in address to hold",
            )
        })
}

/// `canoa office-launcher`.
fn office_launcher(args: &LauncherArgs, front: &dyn Front) -> Result<(), Failed> {
    let action = if args.install {
        Some(LoginAction::Install)
    } else if args.uninstall {
        Some(LoginAction::Uninstall)
    } else if args.status {
        Some(LoginAction::Status)
    } else {
        None
    };
    if let Some(action) = action {
        let place = crate::launcher::login::Place::current().map_err(Failed::run)?;
        return office_login(action, &args.config, front, &place);
    }
    if args.detach {
        return office_detach(&args.config, front);
    }
    let args = &args.config;
    let web = office_web(args, front)?;
    let home = home_of(front)?;
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let launch = crate::launcher::launch_of(&home, &program, args.config.as_deref());
    crate::launcher::run(
        &web,
        &home.dir().join("office"),
        &|| crate::launcher::host_runs(&home),
        &|| launch.start(&home),
        &|line| front.say(line),
        &|| false,
        &crate::launcher::Pace::default(),
    )
    .map_err(Failed::run)
}

/// `canoa office-launcher --detach`: the launcher, started again with no console, and this process ends.
fn office_detach(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    // The launcher checks this itself, but its answer would be in a log nobody is reading when it is started at sign-in.
    office_web(args, front)?;
    let home = home_of(front)?;
    let command = login_command(args, front, false)?;
    let log = home.dir().join("office").join("launcher.log");
    let id = crate::launcher::start_hidden(&command, &log).map_err(Failed::run)?;
    print(
        front,
        &if args.format == Format::Json {
            serde_json::json!({"pid": id, "log": log.display().to_string()}).to_string()
        } else {
            format!(
                "the launcher runs with no window (process {id}); it writes to {}",
                log.display()
            )
        },
    )
}

/// What the sign-in entry runs: this program, its leading arguments and `office-launcher`, with the `--config` it was given
/// and, with `detach`, the flag that starts it with no console.
fn login_command(
    args: &ConfigArgs,
    front: &dyn Front,
    detach: bool,
) -> Result<Vec<String>, Failed> {
    let program = front
        .program()
        .map_err(|error| Failed::run(format!("this program cannot find itself: {error}")))?;
    let leading: Vec<String> = program
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    Ok(crate::launcher::login::command_of(
        &program.path,
        &leading,
        args.config.as_deref(),
        detach,
    ))
}

/// `canoa office-launcher --install|--uninstall|--status`, against `place`.
fn office_login(
    action: LoginAction,
    args: &ConfigArgs,
    front: &dyn Front,
    place: &crate::launcher::login::Place,
) -> Result<(), Failed> {
    match action {
        LoginAction::Uninstall => {
            let line = if place.remove().map_err(Failed::run)? {
                format!("removed {}", place.location())
            } else {
                format!("nothing registered at {}", place.location())
            };
            print(front, &line)
        }
        LoginAction::Install => {
            // The check the launcher makes when it starts: an entry that fails at every sign-in helps nobody.
            office_web(args, front)?;
            let command = login_command(args, front, place.needs_detach())?;
            place.install(&command).map_err(Failed::run)?;
            print(
                front,
                &format!(
                    "registered {}: {} (it starts at the next sign-in)",
                    place.location(),
                    place.text_of(&command)
                ),
            )
        }
        LoginAction::Status => {
            let address = office_web(args, front).ok().map(|web| web.listen);
            let home = home_of(front)?;
            let status = crate::launcher::login::status(
                place,
                &login_command(args, front, place.needs_detach())?,
                address,
                crate::launcher::host_runs(&home),
            )
            .map_err(Failed::run)?;
            print(front, &status.describe(args.format == Format::Json))
        }
    }
}

fn check(args: &ConfigArgs, front: &dyn Front) -> Result<(), Failed> {
    let config = load(args, front)?;
    for entry in &config.entries {
        let kind = match entry.kind {
            crate::EntryKind::Agent { .. } | crate::EntryKind::Harness { .. } => "agent",
            crate::EntryKind::Provider { .. } => "provider",
            crate::EntryKind::Tool { .. } => "tool",
        };
        let profile = crate::serve::applied_profile(&config, entry);
        let line = match args.format {
            Format::Json => serde_json::json!({
                "name": entry.name,
                "kind": kind,
                "listen": entry.listen.to_string(),
                "profile": profile,
            })
            .to_string(),
            Format::Text => format!("{} {kind} {} profile: {profile}", entry.name, entry.listen),
        };
        print(front, &line)?;
    }
    // An address is never refused for being plain http; the person is told what it costs.
    for note in crate::addresses::warnings(&config) {
        let line = match args.format {
            Format::Json => serde_json::json!({"warning": note}).to_string(),
            Format::Text => format!("non-default: {note}"),
        };
        print(front, &line)?;
    }
    Ok(())
}

fn options(args: &OptionsArgs, front: &dyn Front) -> Result<(), Failed> {
    let choices = if args.entry {
        let config = load(&args.config, front)?;
        let mut asking = crate::options::Asking {
            wait: core::time::Duration::from_secs(args.wait),
            any_account: true,
            ..crate::options::Asking::default()
        };
        if let Some(account) = &args.account {
            let home = home_of(front)?;
            let store = crate::accounts::Accounts::open(&config, home.dir())
                .map_err(|error| Failed::run(error.to_string()))?;
            asking.account = Some((account.clone(), store));
        }
        crate::options::entry_with(&config, &args.source, &asking)
            .ok_or_else(|| Failed::usage("canoa.cfg has no entry of that name"))?
    } else {
        if !crate::config::is_built_in_harness(&args.source) {
            return Err(Failed::usage(
                "SOURCE is the id of a bundled agent, or an entry's name with --entry",
            ));
        }
        crate::options::preset(&args.source, args.executable.as_deref())
    };
    match args.config.format {
        Format::Json => print(front, &choices.to_json().to_string()),
        Format::Text => {
            if let Some(why) = &choices.unknown_because {
                print(front, &format!("unknown: {why}"))?;
            }
            if choices.signed_out {
                print(front, "signed out")?;
            }
            for model in &choices.models {
                print(
                    front,
                    &format!(
                        "{}	{}	{}",
                        model.id,
                        model.name.as_deref().unwrap_or(&model.id),
                        model.efforts.join(",")
                    ),
                )?;
            }
            Ok(())
        }
    }
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
        #[cfg(all(unix, feature = "portal"))]
        Command::Portal(args) => portal(&args, front),
        Command::Serve(args) => serve(&args, front),
        Command::Check(args) => check(&args, front),
        Command::OfficeLauncher(args) => office_launcher(&args, front),
        Command::Layers(args) => layers(&args, front),
        Command::Options(args) => options(&args, front),
        Command::Id {
            method,
            identity: Some(path),
            ..
        } => print(front, &bridge::identity_file(&path, method.method)?),
        Command::Id { method, force, .. } => print(
            front,
            &method
                .identity_unless_running(&home_of(front)?, force)?
                .peer_id()
                .to_string(),
        ),
        Command::Peers { format } => list_peers(format, front),
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
        } if native => pair_native(
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
        } => pair(
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
            print(front, &bridge::mcp_config(&peer, &name, front)?)
        }
        Command::McpShim { peer } => bridge::mcp_shim(peer.as_deref(), front),
        Command::AgentGuard { pid } => {
            bridge::agent_guard(pid);
            Ok(())
        }
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
        Command::Accounts { command } => accounts(command, front),
        Command::Mail { command } => mail(command, front),
        Command::Native { action } => native(action, front),
        Command::NativeHost { origin, .. } => native_host(&origin, front),
        Command::McpBridge => mcp_bridge(),
        Command::Helper { library, args } => crate::bundled::run_helper(&library, &args)
            .ok_or_else(|| {
                Failed::usage(format!(
                    "no library of this host has a helper called {library}"
                ))
            })?
            .map_err(Failed::run),
        Command::HookForward {
            channel,
            refuse,
            kind,
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
        assert_eq!(args.config.config.as_deref(), Some("x.cfg"));
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
        assert!(own(&["mcp-bridge"]));
        assert!(own(&["helper", "a-library", "--model", "m"]));
        assert!(own(&["hook-forward", "--channel", "x", "start"]));
        #[cfg(unix)]
        assert!(own(&[crate::spawn::SPAWNER_COMMAND]));
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
    fn serve_knows_which_library_flags_the_command_line_gave() {
        let given = |line: &[&str]| {
            let cli = Cli::try_parse_from(line).expect("parses");
            let Command::Serve(args) = cli.command else {
                panic!("serve");
            };
            args.given.0.clone()
        };
        assert!(given(&["canoa", "serve"]).is_empty());
        assert!(given(&["canoa", "serve", "--method", "noise", "--key-limit", "5"]).is_empty());
        assert_eq!(
            given(&["canoa", "serve", "--max-peers", "4"]),
            ["--max-peers"]
        );
        assert_eq!(
            given(&[
                "canoa",
                "serve",
                "--no-peer-proof",
                "--listen",
                "127.0.0.1:1"
            ]),
            ["--listen", "--no-peer-proof"]
        );
    }

    #[test]
    fn accept_needs_a_peer() {
        assert!(Cli::try_parse_from(["canoa", "accept"]).is_err());
    }

    fn agent_run(args: &[&str]) -> crate::harness::AgentRun {
        let cli = Cli::try_parse_from(["canoa", "agent"].iter().chain(args)).expect("parses");
        assert!(cli.is_a_hosts_own(), "it takes the standard streams");
        let Command::Agent(agent) = cli.command else {
            panic!("agent");
        };
        agent.run()
    }

    #[test]
    fn agent_serves_the_id_it_is_given_with_the_settings_it_is_given_and_what_a_host_narrows() {
        let run = agent_run(&[
            "claude",
            "--executable",
            "/opt/claude",
            "--model",
            "claude-haiku-5-5",
            "--effort",
            "low",
            "--setting",
            "setting_sources=[]",
            "--setting",
            "allow_client_plugins=true",
            "--setting",
            "permission_mode=plan",
            "--working-root",
            "/w",
            "--no-client-mcp-servers",
            "--max-acp-version",
            "1",
        ]);
        assert_eq!(run.id, "claude");
        assert_eq!(
            run.executable.as_deref(),
            Some(std::ffi::OsStr::new("/opt/claude"))
        );
        let settings = &run.settings;
        assert_eq!(settings["model"], "claude-haiku-5-5");
        assert_eq!(settings["effort"], "low");
        assert_eq!(settings["setting_sources"], serde_json::json!([]));
        assert_eq!(settings["allow_client_plugins"], true);
        assert_eq!(settings["permission_mode"], "plan", "text stays text");
        assert_eq!(settings["client_mcp_servers"], false);
        assert_eq!(settings["max_acp_version"], 1);
        assert_eq!(run.roots, [PathBuf::from("/w")]);

        let bare = agent_run(&["claude"]);
        assert!(
            bare.settings.is_empty(),
            "a setting nobody gave is the provider's own: {:?}",
            bare.settings
        );
        assert!(!bare.access_asked && bare.roots.is_empty());
    }

    #[test]
    fn agent_takes_a_cache_lifetime_for_any_agent() {
        use coqui_acp_api::CacheLifetime;
        for lifetime in crate::config::CACHE_LIFETIMES {
            assert_eq!(
                agent_run(&["claude", "--cache-lifetime", lifetime]).lifetime,
                crate::config::cache_lifetime_named(lifetime).expect("a lifetime")
            );
        }
        assert_eq!(agent_run(&["codex"]).lifetime, CacheLifetime::Default);
        assert!(
            Cli::try_parse_from(["canoa", "agent", "claude", "--cache-lifetime", "forever"])
                .is_err()
        );
    }

    /// Whether the provider of `run` takes the settings `canoa agent` would give it.
    fn provider_takes(run: &crate::harness::AgentRun) -> Result<(), String> {
        let registry = std::sync::Arc::new(crate::kind::registry(None).map_err(|e| e.to_string())?);
        let settings = run.provider_settings(&registry)?;
        crate::kind::Harness::new(
            &registry,
            &run.id,
            (&settings, &crate::config::Settings::default()),
            false,
            Vec::new(),
            std::sync::Arc::default(),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    #[test]
    fn the_command_lines_that_start_an_agent_are_accepted_by_every_bundled_agents_provider() {
        // What the host's tests and documents start `canoa agent` with, and what every agent takes alike (a flag that sets a provider setting is
        // left out of the loop over every agent: a provider need not declare it, and says so).
        let documented: [&[&str]; 3] = [
            &[
                "claude",
                "--executable",
                "none",
                "--setting",
                "setting_sources=[\"user\",\"project\"]",
                "--setting",
                "allow_client_plugins=true",
            ],
            &["claude", "--executable", "none"],
            &["codex", "--executable", "none"],
        ];
        for args in documented {
            let run = agent_run(args);
            provider_takes(&run).unwrap_or_else(|why| panic!("{args:?}: {why}"));
        }
        for id in crate::bundled::harnesses() {
            let run = agent_run(&[
                id,
                "--executable",
                "none",
                "--working-root",
                "/w",
                "--cache-lifetime",
                "short",
            ]);
            provider_takes(&run).unwrap_or_else(|why| panic!("{id}: {why}"));
        }
    }

    #[test]
    fn agent_takes_the_access_every_agent_takes_the_same_way_and_any_id() {
        use coqui_acp_harness::{Access, Approvals, Sandbox};
        let run = agent_run(&[
            "codex",
            "--approvals",
            "edits",
            "--sandbox",
            "workspace",
            "--no-network",
        ]);
        let mut expected = Access::STANDARD;
        expected.approvals = Approvals::Edits;
        expected.sandbox = Sandbox::Workspace;
        expected.network = false;
        assert_eq!(run.access, Some(expected.to_json()));
        assert!(run.access_asked);
        assert_eq!(
            agent_run(&["claude"]).access,
            Some(Access::STANDARD.to_json()),
            "no flag asks for the standard access, the same for every agent"
        );
        // The id is whatever the host carries: the command line does not know which.
        assert_eq!(agent_run(&["gemini"]).id, "gemini");
        for wrong in [
            &["codex", "--permission-mode", "plan"][..],
            &["codex", "--approval-policy", "never"],
            &["claude", "--sandbox", "workspace-write"],
            &["claude", "--max-acp-version", "3"],
            &["claude", "--setting", "no-equals-sign"],
            &["claude", "--setting", "=empty"],
            &[],
        ] {
            assert!(
                Cli::try_parse_from(["canoa", "agent"].iter().chain(wrong)).is_err(),
                "{wrong:?}"
            );
        }
    }
}

#[cfg(test)]
mod office_login_tests {
    use std::sync::Mutex;

    use super::*;
    use crate::launcher::login::Place;

    /// A front with a home and a program of its own, keeping what is printed.
    struct Said {
        home: PathBuf,
        lines: Mutex<Vec<String>>,
    }

    impl Front for Said {
        fn print(&self, line: &str) -> std::io::Result<()> {
            self.lines
                .lock()
                .expect("not poisoned")
                .push(line.to_owned());
            Ok(())
        }

        fn say(&self, _line: &str) {}

        fn when_input_closes(
            &self,
            _status: u8,
            _then: Box<dyn FnOnce() + Send>,
        ) -> std::io::Result<()> {
            Ok(())
        }

        fn program(&self) -> std::io::Result<Program> {
            Ok(Program::new(
                PathBuf::from("/opt/canoa app/canoa"),
                Vec::new(),
            ))
        }

        fn home(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }

        fn holder(&self) -> &str {
            "a test"
        }

        fn ask(&self, _prompt: &str, _secret: bool) -> std::io::Result<Option<String>> {
            Ok(None)
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("canoa-login-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    fn config_with(dir: &Path, address: &str, web: bool) -> ConfigArgs {
        let file = dir.join("canoa.cfg");
        let text = if web {
            format!("[host.web]\nlisten = \"{address}\"\noffice = true\noffice_local = true\n")
        } else {
            "[host]\n".to_owned()
        };
        std::fs::write(&file, text).expect("a config");
        ConfigArgs {
            config: Some(file.display().to_string()),
            format: Format::Text,
        }
    }

    #[test]
    fn install_registers_one_entry_naming_the_config_status_says_both_facts_and_uninstall_removes_it()
     {
        let dir = scratch("flow");
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = held.local_addr().expect("its address").to_string();
        let args = config_with(&dir, &address, true);
        let front = Said {
            home: dir.join("home"),
            lines: Mutex::new(Vec::new()),
        };
        let place = Place::Autostart(dir.join("autostart"));
        let said = |front: &Said| {
            front
                .lines
                .lock()
                .expect("not poisoned")
                .pop()
                .unwrap_or_default()
        };

        office_login(LoginAction::Install, &args, &front, &place).expect("installed");
        office_login(LoginAction::Install, &args, &front, &place).expect("installed again");
        let entries: Vec<_> = std::fs::read_dir(dir.join("autostart"))
            .expect("a folder")
            .collect();
        assert_eq!(entries.len(), 1, "one entry however often it is installed");
        let written = std::fs::read_to_string(place.location()).expect("the entry");
        assert!(written.contains("office-launcher"), "{written}");
        assert!(
            written.contains("--config") && written.contains("canoa.cfg"),
            "{written}"
        );

        office_login(LoginAction::Status, &args, &front, &place).expect("status");
        let status = said(&front);
        assert!(status.contains("registered: yes"), "{status}");
        assert!(
            status.contains(&format!("address: {address} answers")),
            "{status}"
        );
        assert!(status.contains("a launcher answers: yes"), "{status}");

        office_login(LoginAction::Uninstall, &args, &front, &place).expect("removed");
        assert!(!Path::new(&place.location()).exists());
        office_login(LoginAction::Status, &args, &front, &place).expect("status");
        assert!(said(&front).contains("registered: no"));
        office_login(LoginAction::Uninstall, &args, &front, &place).expect("again");
        assert!(said(&front).contains("nothing registered"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_refuses_a_config_with_no_address_to_hold_and_writes_nothing() {
        let dir = scratch("refuse");
        let args = config_with(&dir, "", false);
        let front = Said {
            home: dir.join("home"),
            lines: Mutex::new(Vec::new()),
        };
        let place = Place::Autostart(dir.join("autostart"));
        let failed =
            office_login(LoginAction::Install, &args, &front, &place).expect_err("refused");
        assert_eq!(failed.status, 2);
        assert!(!dir.join("autostart").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_install_flags_cannot_be_given_together() {
        for both in [
            ["--install", "--uninstall"],
            ["--install", "--status"],
            ["--uninstall", "--status"],
            ["--detach", "--install"],
            ["--detach", "--uninstall"],
            ["--detach", "--status"],
        ] {
            assert!(
                Cli::try_parse_from(["canoa", "office-launcher", both[0], both[1]]).is_err(),
                "{both:?}"
            );
        }
        assert!(Cli::try_parse_from(["canoa", "office-launcher", "--install"]).is_ok());
        assert!(Cli::try_parse_from(["canoa", "office-launcher", "--detach"]).is_ok());
    }
}
