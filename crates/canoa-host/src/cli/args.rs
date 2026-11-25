//! Command-line argument structs and parsers for `canoa`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names what it uses from std"
)]

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use coqui_link::auth;
use coqui_link::auth::Method;

use super::bridge;
use super::{Failed, STATUS_HELD};
use crate::home::{Home, LockError};

#[cfg(all(unix, feature = "portal"))]
use super::portal::PortalArgs;

/// The help's closing section; the variables carry the build's brand prefix.
fn after_help() -> String {
    format!(
        "\
Environment:
  {config:<24} canoa.cfg to read, below --config and above the platform's default
  {installed:<24} the folder whose canoa.cfg is the installed copy (default: beside the program)
  {log:<24} what to log on standard error, as a tracing filter (default: info)
  {home:<24} the directory of this host's identity and trust store (default: .canoa in the user's home folder)
  {color:<24} no colour in the log

Exit status: 0 success, 1 the operation failed, 2 the invocation or canoa.cfg was wrong, 3 another host holds this home.",
        config = canoa::brand::CONFIG,
        installed = canoa::brand::INSTALLED_DIR,
        log = canoa::brand::LOG,
        home = "COQUI_AGENTIC_PEER_HOME",
        color = "NO_COLOR",
    )
}

/// Serves stdio ACP agents and Guatiao agent providers from canoa.cfg to paired keys, on loopback.
#[derive(Debug, Parser)]
#[command(name = "canoa", version, after_help = after_help())]
pub struct Cli {
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Debug, Subcommand)]
pub(super) enum Command {
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
    /// Answer one request of the control channel as this computer's owner, over canoa.cfg's files, with no host running: the way an
    /// application reads and changes its configuration while none serves. Prints the answer, one JSON-RPC line.
    Control(ControlArgs),
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
    /// Call a control method of a session service of this host's libraries, as the application does: `canoa service mail accounts`
    /// lists the mail accounts, `canoa service mail add --params -` adds one from the JSON on standard input (a password goes
    /// there, never on the command line). The answer is the method's JSON.
    Service(Box<ServiceArgs>),
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
    /// from standard input. For a session whose tools are served from an environment, `env-tool` prints the host's answer to
    /// one of the agent's own tool calls (status 2 when it cannot be had, which refuses the call), and `env-exec ID` runs the
    /// command the hook prepared as ID there, printing its output and ending with its status (126 when nothing ran); `env-start` tells the host
    /// the session started with that hook in force, without which its prompts are refused.
    #[command(hide = true, name = "hook-forward")]
    HookForward {
        /// The file that holds the channel's address and secret.
        #[arg(long)]
        channel: PathBuf,
        /// Stop what the harness was about to do when the host cannot be asked: an overlay's hook that guards it is on.
        #[arg(long)]
        refuse: bool,
        /// What the harness reports: `start`, `pre-compact` or `batch`; or `env-tool`, `env-exec` or `env-start`.
        kind: String,
        /// For `env-exec`, the command the hook prepared.
        id: Option<String>,
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
pub(super) enum NativeAction {
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
pub(super) enum NativeBrowser {
    Chrome,
    Edge,
    Chromium,
    /// Chrome for Testing reads Chrome's place.
    ChromeForTesting,
}

/// Which registration a `native` command is about.
#[derive(Debug, Args)]
pub(super) struct NativeScope {
    /// The native host's name, `dev.boriken.canoa` unless given. Another name is one the extension does not look for: for
    /// trying a registration without touching the real one.
    #[arg(long, value_name = "NAME")]
    pub(super) name: Option<String>,
    /// A browser's --user-data-dir, where Chromium on Linux and macOS reads native hosts when it is started with one.
    #[arg(long, value_name = "DIR")]
    pub(super) profile: Vec<PathBuf>,
}

/// `canoa native install`.
#[derive(Debug, Args)]
pub(super) struct NativeInstall {
    #[command(flatten)]
    pub(super) scope: NativeScope,
    /// An extension that may start it, besides the ones this version knows.
    #[arg(long, value_name = "ID")]
    pub(super) extension_id: Vec<String>,
    /// Only these browsers; every one by default.
    #[arg(long, value_enum)]
    pub(super) browser: Vec<NativeBrowser>,
    /// What the extension's Start button runs, in place of this program's `serve`.
    #[arg(long, value_name = "PROGRAM")]
    pub(super) launch: Option<PathBuf>,
    /// Its arguments, after `--`.
    #[arg(last = true)]
    pub(super) launch_args: Vec<String>,
}

/// What `canoa options` takes.
#[derive(Debug, Args)]
pub(super) struct OptionsArgs {
    /// The id of a bundled agent; with --entry, an entry's name.
    pub(super) source: String,
    /// SOURCE names an entry of canoa.cfg, not a built-in harness.
    #[arg(long)]
    pub(super) entry: bool,
    /// For a bundled agent, the program; the variable `canoa agent` reads (COQUI_ACP_ID_EXECUTABLE), or the agent's own name,
    /// without it.
    #[arg(long)]
    pub(super) executable: Option<OsString>,
    /// With --entry, the entry's account to ask as, so that it lists what that account may use.
    #[arg(long, requires = "entry")]
    pub(super) account: Option<String>,
    /// With --entry, the seconds a probe session of an agent that lists its models only in a session is waited for.
    #[arg(long, requires = "entry", default_value_t = 20)]
    pub(super) wait: u64,
    #[command(flatten)]
    pub(super) config: ConfigArgs,
}

/// What `canoa accounts` does.
#[derive(Debug, Subcommand)]
pub(super) enum AccountsCommand {
    /// List the accounts of canoa.cfg with their state; never a credential.
    List {
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// List the account providers this host carries, each with its own ways to sign in.
    Providers {
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Make an account by signing in. A secret it asks for is typed without echo, or read from standard input when that is
    /// not a terminal, and never taken from the command line. Appends `[host.accounts.NAME]` to canoa.cfg.
    Add {
        /// The account's name, as an entry's `accounts` names it.
        name: String,
        /// The account provider that makes the account, by the id `canoa accounts providers` lists (`claude_accounts`, `litellm_accounts`).
        #[arg(long)]
        provider: String,
        /// How to sign in, one of the provider's own login methods (naming none lists them).
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

/// What `canoa service` takes.
#[derive(Debug, Args)]
pub(super) struct ServiceArgs {
    /// The service's id (`mail`).
    pub(super) id: String,
    /// The method, without the service's namespace (`accounts`, `add`, `test`, `remove` for the mail service).
    pub(super) method: String,
    /// The method's parameters as a JSON object, or `-` to read it from standard input.
    #[arg(long, default_value = "{}")]
    pub(super) params: String,
    #[command(flatten)]
    pub(super) config: ConfigArgs,
}

/// What `canoa agent` takes: the agent, its program and settings, and what every agent takes the same way.
#[derive(Debug, Args)]
pub(super) struct AgentCommand {
    /// The agent to serve: the id of a harness provider the bundled agents offer, such as claude or codex.
    pub(super) id: String,
    /// The agent's program; else the variable COQUI_ACP_ID_EXECUTABLE (ID in upper case), else the copy the Store installed, else
    /// the agent's own name on the PATH.
    #[arg(long)]
    pub(super) executable: Option<OsString>,
    /// A setting of the agent's provider, KEY=VALUE, given as JSON when it is (`true`, `3`, `["user"]`) and as text when not; repeat
    /// for several. The provider refuses a key it does not declare.
    #[arg(long = "setting", value_name = "KEY=VALUE", value_parser = parse_setting)]
    pub(super) settings: Vec<(String, serde_json::Value)>,
    #[command(flatten)]
    pub(super) common: AgentArgs,
}

/// What `canoa agent` takes for every agent.
#[derive(Debug, Args)]
pub(super) struct AgentArgs {
    /// The model every session starts with, until the client chooses another.
    #[arg(long)]
    pub(super) model: Option<String>,
    /// The effort every session starts with, such as low or high.
    #[arg(long)]
    pub(super) effort: Option<String>,
    /// The highest ACP version answered.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    pub(super) max_acp_version: Option<u8>,
    /// Sessions at once.
    #[arg(long)]
    pub(super) max_sessions: Option<usize>,
    /// A directory every session's cwd and more directories must lie in; repeat for several. None takes any.
    #[arg(long = "working-root", value_name = "DIRECTORY")]
    pub(super) working_roots: Vec<PathBuf>,
    /// Refuse a session that a client gives MCP servers.
    #[arg(long)]
    pub(super) no_client_mcp_servers: bool,
    /// How much a command may reach: none (the default), workspace or read-only. The same for every agent.
    #[arg(long, value_name = "SANDBOX", value_parser = parse_sandbox)]
    pub(super) sandbox: Option<coqui_acp_harness::Sandbox>,
    /// Take the network from the commands a session runs, where the agent can.
    #[arg(long)]
    pub(super) no_network: bool,
    /// What is asked first: ask (the default), edits or never. The same for every agent.
    #[arg(long, value_name = "APPROVALS", value_parser = parse_approvals)]
    pub(super) approvals: Option<coqui_acp_harness::Approvals>,
    /// How long the prompt cache should live: default (the agent's own), short (about five minutes, for one-shot and
    /// background runs) or long (about an hour, for an interactive session). An agent with no such setting ignores it.
    #[arg(long, value_name = "LIFETIME", default_value = "default", value_parser = parse_lifetime)]
    pub(super) cache_lifetime: coqui_acp_api::CacheLifetime,
    /// The longest ACP message, as one line, either way, in bytes: raise it for a client that sends large attachments.
    #[arg(long, value_name = "BYTES", default_value_t = crate::config::DEFAULT_MAX_MESSAGE_BYTES)]
    pub(super) max_message_bytes: usize,
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
    pub(super) fn run(self) -> crate::harness::AgentRun {
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
pub(super) struct MethodArg {
    /// `noise`, or `webcrypto` for a browser whose WebCrypto has no X25519. This user's identity of that method, made when
    /// absent. Without it, the identity this user has, or a new one of the default method; with one of each, name one.
    #[arg(long, value_name = "METHOD")]
    pub(super) method: Option<Method>,
}

impl MethodArg {
    pub(super) fn identity(self, home: &Home) -> Result<auth::Identity, Failed> {
        home.identity(self.method).map_err(Failed::run)
    }

    /// The identity, for a command that does not hold the home: a method other than the one the home records is refused
    /// while a host runs on it, since that host's next restart would refuse every client already paired.
    pub(super) fn identity_unless_running(
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
pub(super) struct ForceArg {
    /// Change the method of a home a host is running on anyway; the host's next restart refuses every client already paired.
    #[arg(long)]
    pub(super) force: bool,
}

#[derive(Debug, Args)]
pub(super) struct ServeArgs {
    #[command(flatten)]
    pub(super) config: ConfigArgs,
    /// Serve the one program after `--` to paired keys over the network, with no canoa.cfg: this host's own proof (--method,
    /// --key-limit, --no-rekey) and the flags below it. The way to give one agent TLS, WebTransport, a linger or a
    /// standing MCP offer.
    #[arg(long)]
    pub(super) program: bool,
    /// End, with every agent started, when standard input closes: for a program that runs the host as its child.
    #[arg(long)]
    pub(super) until_stdin_closes: bool,
    /// Say that a supervisor starts this host again when it ends with status 75, which is what lets a paired key ask for
    /// `host/restart`; without it that request is refused.
    #[arg(long)]
    pub(super) supervised: bool,
    /// With no entry in canoa.cfg, or no canoa.cfg at all, keep running with nothing to serve, rather than ending with
    /// status 2.
    #[arg(long)]
    pub(super) allow_empty: bool,
    /// Run for a portal: every entry and the control channel listen on a socket under DIR, which only `--portal-user`'s
    /// processes may use, and the web app is not served. For `canoa portal`'s per-user service.
    #[arg(long, value_name = "DIR")]
    pub(super) portal: Option<PathBuf>,
    /// The portal's user, a name in /etc/passwd or a number: the only user whose processes may use those sockets.
    #[arg(long, value_name = "USER", requires = "portal")]
    pub(super) portal_user: Option<String>,
    /// A group whose members may have a host run for a portal; this account must be in one (the portal's own `groups`).
    /// Fixed in the unit, so that an account outside them is never given a host.
    #[arg(long, value_name = "GROUP", requires = "portal")]
    pub(super) portal_group: Vec<String>,
    /// The lowest user id a host run for a portal may have; root, `nobody` and the system ranges never may.
    #[arg(long, value_name = "UID", default_value_t = 1000, requires = "portal")]
    pub(super) portal_min_uid: u32,
    /// Serve the host's own sources too (every bundled agent's), to this process only: they have no address,
    /// and each line says what a source is missing.
    #[arg(long)]
    pub(super) local_sources: bool,
    /// Serve the web app too, on [host.web]'s address (default 127.0.0.1:7800): from DIR, else [host.web]'s dir, else the
    /// site built in (feature `web`).
    #[arg(long, value_name = "DIR", num_args = 0..=1)]
    pub(super) web: Option<Option<PathBuf>>,
    /// The web app's address, in place of [host.web]'s listen. Beyond loopback it needs [host.web]'s cert and key.
    #[arg(long, value_name = "ADDRESS")]
    pub(super) web_listen: Option<SocketAddr>,
    /// Which of the library's flags the command line gave. Before them, since each field takes its values out of the matches
    /// as it is read.
    #[command(flatten)]
    pub(super) given: bridge::Given,
    /// The proof's method and session settings, which both forms of `serve` take, and with --program the program's own flags.
    #[command(flatten)]
    pub(super) bridge: coqui_acp_bridge::serve::Serve,
}

#[derive(Debug, Args)]
pub(super) struct LauncherArgs {
    #[command(flatten)]
    pub(super) config: ConfigArgs,
    /// Register `office-launcher` (with the --config named here) to start when this user signs in: a Run value on Windows, a
    /// LaunchAgent on macOS, an autostart entry elsewhere. Needs no elevation; running it again replaces the entry.
    #[arg(long, conflicts_with_all = ["uninstall", "status"])]
    pub(super) install: bool,
    /// Delete the entry --install wrote, and nothing else.
    #[arg(long, conflicts_with_all = ["install", "status"])]
    pub(super) uninstall: bool,
    /// Say whether the entry is registered and whether a launcher answers at the add-in's address.
    #[arg(long, conflicts_with_all = ["install", "uninstall"])]
    pub(super) status: bool,
    /// Start the launcher again with no console window and end at once, printing its process id: its output goes to
    /// launcher.log in the home's `office` folder. What the Windows entry runs, since `canoa.exe` is a console program. On
    /// Windows the launcher holds every handle this process held, so a caller that reads a pipe of this process to its end
    /// waits for the launcher to end: have the output go to a file or a terminal.
    #[arg(long, conflicts_with_all = ["install", "uninstall", "status"])]
    pub(super) detach: bool,
}

#[derive(Debug, Args)]
pub(super) struct ControlArgs {
    #[command(flatten)]
    pub(super) config: ConfigArgs,
    /// The request, one JSON-RPC line; `@FILE` reads it from the file, which a large one needs on a command line that is short.
    pub(super) request: String,
}

#[derive(Debug, Args)]
pub(super) struct ConfigArgs {
    /// canoa.cfg files, a list split at `;` on Windows and `:` elsewhere, the last one highest. A leading `+` puts them on
    /// top of the installed, system and user files; without it they are the only ones read. By default those three layers,
    /// the user's at %APPDATA%\Canoa\canoa.cfg or $XDG_CONFIG_HOME/Canoa/canoa.cfg (~/.config without it).
    #[arg(long, env = canoa::brand::CONFIG)]
    pub(super) config: Option<String>,
    /// How `check` prints entries and `options` its choices.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub(super) format: Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
}
