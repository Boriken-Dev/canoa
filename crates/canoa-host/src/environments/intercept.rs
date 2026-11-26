//! An agent that does its own file and shell work, taken over so that what it does acts in the environment a session's tools are
//! served from. Its vendor describes a hook of the agent's own ([`ToolHook`]) that runs `canoa hook-forward` before every tool
//! and when a session starts; the forwarder asks the host on a loopback channel, and the host answers in the agent's terms.
//!
//! The hook changes where a command runs, never whether it may. A command is rewritten to run the host's `env-exec`, which runs
//! it in the environment, and the command as the agent wrote it stays only where the agent's rules read it and nothing runs it
//! ([`rewritten`]). The answer carries no decision, so the agent's own mode and the person's rules judge the call. A tool the
//! vendor lets pass runs as it is; a file tool is refused with the way to do the same in the environment; any other is refused.
//!
//! Every failure refuses the call and nothing runs on this computer: a forwarder that cannot read its channel or reach the host,
//! a payload past `[host] max_message_bytes`, an answer not signed for the request, a host that does not answer within the
//! environment's `hook_seconds` (one budget in all), an `env-exec` naming no command this session's hook prepared within
//! `prepared_seconds`, or a session in plan mode. A session's prompts wait until its hook reports that the session started with
//! it, and are refused when it does not. The channel's address and secret are in a file of a directory made for one session
//! request; a request names the secret by a public key and signs itself with it, and the host signs its answers, so the secret
//! never crosses the channel and nothing that only took the port can answer.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads, files and the clock are std's"
)]

use canoa::brand::PROGRAM;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use canoa::environment::{EnvironmentInfo, capability};
use canoa::vendor::{
    HookCommand, HookKind, HooksRequest, RulesRequest, ShellFamily, ToolHook, ToolRole,
};
use coqui_acp_api::{ErrorOutput, SpawnSpec};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::person::{self, Judged};
use super::places::Rules;
use super::remote::{Remote, code_of};
use super::{Env, lock};
use crate::profile::hosted::{
    Private, Written, plugin_meta, quotable, random_hex, read_line, same,
};

/// The forwarder's kind that asks about one tool call.
pub(crate) const KIND_TOOL: &str = "env-tool";
/// The forwarder's kind that runs a command the hook prepared.
pub(crate) const KIND_EXEC: &str = "env-exec";
/// The forwarder's kind that says the agent's session started with the hook in force.
pub(crate) const KIND_START: &str = "env-start";
/// The file in a session request's directory that holds the channel.
const CHANNEL_FILE: &str = "channel.json";
/// The key under which a forwarder gives the host the value of the variable the agent's folder is named by, in the payload it sends
/// (the agent's own payload never holds it: the forwarder takes it out first).
const HOME_KEY: &str = "canoa_home";
/// The longest channel file the forwarder reads: the host writes an address, a secret and two numbers.
const MAX_CHANNEL_FILE: u64 = 4096;
/// What a request adds to the payload it carries, and the most an answer's header may hold.
const ENVELOPE: usize = 64 * 1024;
/// Node keeps a timer in a signed 32-bit count of milliseconds and ends a longer one at once, which would let the call go on: the
/// longest wait given to a harness's hook, with room for the margin its vendor adds.
const LONGEST_HOOK_SECONDS: u64 = 2_147_000;
/// How often the accepting thread looks for a connection or for its stop.
const POLL: Duration = Duration::from_millis(20);
/// How often a running command is looked at for its end, its time limit and a forwarder that left.
const WAIT_SLICE_MS: u64 = 200;
/// The most commands of one session whose originals are kept to tell the calls the hook saw, and the most calls kept until they end.
const MAX_SEEN: usize = 512;
/// How long a stopped command's end is waited for before its status is given up.
const END_GRACE: Duration = Duration::from_secs(2);
/// A frame of a command's output, and the last frame, which says how it ended.
const TAG_OUTPUT: u8 = 1;
const TAG_END: u8 = 2;
/// The status of an `env-exec` that ran nothing: the command was refused or could not be started.
const NOT_RUN: u8 = 126;
/// The status of an `env-tool` that could not be answered, which the hook's command turns into a refusal.
const REFUSED: u8 = 2;

/// A session's permission mode as the host holds it, strictest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mode {
    /// Nothing is changed or run.
    Plan,
    /// Only what a rule allows runs; nothing is asked.
    DontAsk,
    /// What a rule does not allow is asked.
    Manual,
    /// Edits run; the rest is asked.
    AcceptEdits,
    /// A classifier decides what is asked.
    Auto,
    /// Everything runs.
    Bypass,
}

impl Mode {
    /// The mode an id names (the harness-neutral ids, and `default` for manual); `None` for another, which counts as asking.
    pub(crate) fn of_id(id: &str) -> Option<Self> {
        Some(match id {
            "plan" => Self::Plan,
            "dont_ask" => Self::DontAsk,
            "manual" | "default" => Self::Manual,
            "accept_edits" => Self::AcceptEdits,
            "auto" => Self::Auto,
            "bypass" => Self::Bypass,
            _ => return None,
        })
    }
}

/// The mode of each session of a connection. Only the client's own choice, answered OK, raises one, and so does the person's own
/// answer to the agent's request to leave plan mode ([`Modes::left_plan`]); what the agent says of itself can only lower it. A
/// session nothing was said of asks.
#[derive(Debug, Default)]
pub(crate) struct Modes(Mutex<BTreeMap<String, Mode>>);

impl Modes {
    /// The mode of `session`.
    pub(crate) fn of(&self, session: &str) -> Mode {
        lock(&self.0).get(session).copied().unwrap_or(Mode::Manual)
    }

    /// The client chose `mode` for `session` and the agent took it.
    pub(crate) fn chosen(&self, session: &str, mode: Mode) {
        lock(&self.0).insert(session.to_owned(), mode);
    }

    /// The person allowed the agent's request to leave plan mode, and the agent says `session` is now in `mode`: a session held at
    /// plan goes to `mode`, but never to Bypass, which only the client's own choice reaches.
    pub(crate) fn left_plan(&self, session: &str, mode: Mode) {
        let mut all = lock(&self.0);
        if all.get(session).copied() == Some(Mode::Plan) {
            all.insert(session.to_owned(), mode.min(Mode::Auto));
        }
    }

    /// The agent says `session` is in `mode`: kept only when it is stricter.
    pub(crate) fn reported(&self, session: &str, mode: Mode) {
        let mut all = lock(&self.0);
        let now = all.get(session).copied().unwrap_or(Mode::Manual);
        all.insert(session.to_owned(), now.min(mode));
    }

    /// The session ended.
    pub(crate) fn forget(&self, session: &str) {
        lock(&self.0).remove(session);
    }
}

/// What the host answers a tool call.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Verdict {
    /// The call is refused, for this reason.
    Deny(String),
    /// The call goes on with this input, for the agent's own rules and mode to judge.
    Rewrite(Map<String, Value>),
    /// The tool acts on nothing of this computer's: it goes on as it is.
    Pass,
    /// The person is asked about the call, whatever the agent's mode and rules say, for this reason.
    Ask(String),
}

/// Whether `text` holds `word` as a word of its own, whatever its case.
fn has_word(text: &str, word: &str) -> bool {
    let lower = text.to_lowercase();
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    lower.match_indices(word).any(|(at, _)| {
        !lower[..at].chars().next_back().is_some_and(is_word)
            && !lower[at + word.len()..].chars().next().is_some_and(is_word)
    })
}

/// The command that runs the command prepared as `id` through the forwarder `prefix`, written for `shell`, then ends the shell with
/// its status before `original`, which stays only for the agent's rules to read.
///
/// A POSIX shell reads a line whole before it runs it, so `original` after `exit` never runs. PowerShell reads the whole script
/// first and acts on some of it before the first statement: a `trap` anywhere covers the statements before it, and `#requires`
/// loads modules. So for PowerShell `original` is the body of a script block that is never invoked, the forwarder runs inside
/// `try` with an `exit` in `finally` (a forwarder that cannot start, or a status turned into an error, still ends the script),
/// and a command naming `trap` or `#requires` is not rewritten.
///
/// # Errors
///
/// Why the command cannot be run in the environment: a shell this cannot write for, or a PowerShell command of that kind.
pub(crate) fn rewritten(
    prefix: &str,
    id: &str,
    shell: ShellFamily,
    original: &str,
) -> Result<String, String> {
    match shell {
        ShellFamily::Bash | ShellFamily::Zsh | ShellFamily::Other => {
            Ok(format!("{prefix} {KIND_EXEC} {id}; exit; {original}"))
        }
        ShellFamily::Powershell => {
            if has_word(original, "trap") || original.to_lowercase().contains("#requires") {
                return Err(
                    "a PowerShell command that holds trap or #requires is not run in the environment: PowerShell acts on those before the command's first statement, on this computer. Write it without them".to_owned(),
                );
            }
            Ok(format!(
                "try {{ & {prefix} {KIND_EXEC} {id} }} finally {{ exit $(if ($null -eq $LASTEXITCODE) {{ {NOT_RUN} }} else {{ $LASTEXITCODE }}) }}; $null = {{ {original} }}"
            ))
        }
        other => Err(format!(
            "a command for {} cannot be written to run elsewhere",
            other.as_str()
        )),
    }
}

/// `program [before] hook-forward --channel FILE`, quoted for a shell, or `None` when a path holds a character that cannot be
/// quoted safely.
fn forwarder(program: &str, before: &[String], channel: &Path) -> Option<String> {
    let slashed = |text: &str| text.replace('\\', "/");
    let mut parts = Vec::new();
    for each in core::iter::once(program).chain(before.iter().map(String::as_str)) {
        let each = slashed(each);
        if !quotable(&each) {
            return None;
        }
        parts.push(format!("\"{each}\""));
    }
    let channel = slashed(&channel.to_string_lossy());
    if !quotable(&channel) {
        return None;
    }
    parts.push(crate::profile::hosted::FORWARD_COMMAND.to_owned());
    parts.push("--channel".to_owned());
    parts.push(format!("\"{channel}\""));
    Some(parts.join(" "))
}

/// Why the file tool `tool` is refused, and how the same is done in the environment `env` that `info` describes.
fn files_refused(hook: &ToolHook, (env, info): (&str, &EnvironmentInfo), tool: &str) -> String {
    let server = super::mcp::server_name(env);
    let mut ways = Vec::new();
    let names: Vec<&str> = [
        (capability::FS_READ, "fs_read"),
        (capability::FS_WRITE, "fs_write"),
        (capability::FS_WRITE, "edit"),
        (capability::FS_WRITE, "apply_patch"),
        (capability::FS_LIST, "fs_list"),
    ]
    .into_iter()
    .filter(|(needs, _)| info.offers(needs))
    .map(|(_, name)| name)
    .collect();
    if !names.is_empty() {
        ways.push(format!(
            "the tools of the MCP server {server} ({})",
            names.join(", ")
        ));
    }
    if info.offers(capability::EXEC) {
        let shells: Vec<&str> = hook
            .tools
            .iter()
            .filter(|each| each.role == ToolRole::Shell)
            .map(|each| each.name.as_str())
            .collect();
        if !shells.is_empty() {
            ways.push(format!(
                "the {} tool, whose commands run in {env} (grep, find, ls, cat and the like)",
                shells.join(" or "),
            ));
        }
    }
    let way = if ways.is_empty() {
        format!("environment {env} offers nothing to do it with")
    } else {
        format!("use {}", ways.join(", or "))
    };
    format!(
        "In this session the files are in environment {env}, not on this computer, so {tool} is not used: {way}."
    )
}

/// What the person's own rules make of a call of the tool `name` with the input: a refusal, a question, or nothing.
type Person<'a> = &'a mut dyn FnMut(&str, &Map<String, Value>) -> Option<Judged>;

/// What `payload`, a hook's payload in the terms `hook` describes, is answered, for a session in `session` mode, in the environment
/// `env` as `info` described it when the take-over began: nothing here waits on the environment. `prepare` keeps a command for
/// `env-exec` and gives the command that runs it, or why it cannot. `person` is asked about every call first: the person's own
/// rules reach the tools of an environment through it, whatever mode the agent is in.
pub(crate) fn evaluate(
    hook: &ToolHook,
    (env, info): (&str, &EnvironmentInfo),
    session: Mode,
    payload: &Value,
    prepare: &mut dyn FnMut(ShellFamily, &str) -> Result<String, String>,
    person: Person<'_>,
) -> Verdict {
    let Some(name) = payload.pointer(&hook.name_at).and_then(Value::as_str) else {
        return Verdict::Deny("the hook's payload names no tool".to_owned());
    };
    if let Some(input) = payload.pointer(&hook.input_at).and_then(Value::as_object)
        && let Some(judged) = person(name, input)
    {
        return match judged {
            Judged::Deny(why) => Verdict::Deny(why),
            Judged::Ask(why) => Verdict::Ask(why),
        };
    }
    if hook
        .pass_prefixes
        .iter()
        .any(|prefix| !prefix.is_empty() && name.starts_with(prefix.as_str()))
    {
        return Verdict::Pass;
    }
    let Some(tool) = hook.tools.iter().find(|each| each.name == name) else {
        return Verdict::Deny(format!(
            "{name} would act on this computer, not in environment {env}, and is not a tool that can be run there"
        ));
    };
    if tool.role == ToolRole::Pass {
        return Verdict::Pass;
    }
    let Some(input) = payload.pointer(&hook.input_at).and_then(Value::as_object) else {
        return Verdict::Deny(format!("the hook's payload holds no input for {name}"));
    };
    // The agent's word for its mode can only make the session stricter.
    let reported = hook
        .mode_at
        .as_deref()
        .and_then(|at| payload.pointer(at))
        .and_then(Value::as_str)
        .map(|word| {
            hook.modes
                .iter()
                .find(|each| each.word == word)
                .and_then(|each| Mode::of_id(&each.mode))
                .unwrap_or(Mode::Manual)
        })
        .unwrap_or(Mode::Manual);
    let mode = session.min(reported);
    match tool.role {
        ToolRole::Files => Verdict::Deny(files_refused(hook, (env, info), name)),
        ToolRole::Shell | ToolRole::Background => {
            if let Err(why) = super::offered(env, info.clone(), capability::EXEC) {
                return Verdict::Deny(why);
            }
            if mode == Mode::Plan {
                return Verdict::Deny(
                    "The session is in plan mode, so no command runs; say what you would run instead.".to_owned(),
                );
            }
            let (Some(key), Some(shell)) = (tool.command.as_deref(), tool.shell) else {
                return Verdict::Deny(format!("{name}'s description names no command"));
            };
            let Some(command) = input.get(key).and_then(Value::as_str) else {
                return Verdict::Deny(format!("{name} was given no command"));
            };
            match prepare(shell, command) {
                Ok(run) => {
                    let mut changed = input.clone();
                    changed.insert(key.to_owned(), Value::String(run));
                    Verdict::Rewrite(changed)
                }
                Err(why) => Verdict::Deny(why),
            }
        }
        _ => Verdict::Deny(format!(
            "{name} is a tool of a kind the host does not take over"
        )),
    }
}

/// `verdict` in the agent's terms: the JSON text of `hook`'s answer with the reason or the input in place, or nothing for a tool
/// that passes. `None` when the agent's answer form is not a JSON document holding the place.
pub(crate) fn answer(hook: &ToolHook, verdict: Verdict) -> Option<String> {
    let (form, at, value) = match verdict {
        Verdict::Pass => return Some(String::new()),
        Verdict::Deny(why) => (&hook.deny, &hook.deny_reason_at, Value::String(why)),
        Verdict::Ask(why) => match (&hook.ask, &hook.ask_reason_at) {
            (Some(form), Some(at)) => (form, at, Value::String(why)),
            _ => (
                &hook.deny,
                &hook.deny_reason_at,
                Value::String(format!(
                    "{why} The agent has no way to ask the person about a call here, so it is refused."
                )),
            ),
        },
        Verdict::Rewrite(input) => (&hook.rewrite, &hook.rewrite_input_at, Value::Object(input)),
    };
    let mut document: Value = serde_json::from_str(form).ok()?;
    *document.pointer_mut(at)? = value;
    Some(document.to_string())
}

/// A command a session's hook prepared, waiting for its `env-exec`.
struct Op {
    secret: String,
    shell: ShellFamily,
    command: String,
    /// When the hook prepared it.
    at: Instant,
}

/// The session request a secret was given for.
#[derive(Clone)]
struct Holder {
    /// The ACP session, once the agent has answered.
    session: Option<String>,
    /// The folder the session works in on this computer, where the agent reads the project's own settings.
    cwd: String,
    channel: PathBuf,
    /// The agent's session started with the hook in force: the plugin's start hook reported.
    started: bool,
}

/// What the accepting thread and its handlers share with the link.
struct Shared {
    env: Arc<Env>,
    /// What the environment said of itself when the take-over began, which a tool call is judged by.
    info: EnvironmentInfo,
    hook: ToolHook,
    /// The agent the hook is for: its vendor reads the person's rules.
    harness: String,
    /// The path rules of each environment server's environment, by the server's name.
    servers: Vec<(String, Rules)>,
    /// The variable that names the folder the agent keeps its own files in, which the forwarder reads where the agent runs it.
    home_variable: Option<String>,
    /// Whether the agent's own folder is looked for where its process names none: the person's, as the agent finds it itself.
    own_folder: AtomicBool,
    modes: Arc<Modes>,
    program: String,
    before: Vec<String>,
    secrets: Mutex<BTreeMap<String, Holder>>,
    /// The secret of each session's latest request, whose plugin is the one in force.
    latest: Mutex<BTreeMap<String, String>>,
    ops: Mutex<BTreeMap<String, Op>>,
    /// The commands of each secret's session as the agent wrote them, which its hook prepared: a call the agent reports done must
    /// be one of these.
    originals: Mutex<BTreeMap<String, VecDeque<String>>>,
    /// The calls the agent announced that are not done, by session and call id: the tool and its command.
    calls: Mutex<BTreeMap<(String, String), (String, Option<String>)>>,
    max_bytes: usize,
    /// How long a forwarder may take for its answer, in all: the environment's `hook_seconds`.
    seconds: u64,
    /// Requests handled now, and how many may be (`hook_handlers`, 0 for any number).
    handlers: AtomicUsize,
    max_handlers: usize,
    /// How long a connection has to send its request before its secret is checked (`hook_read_ms`, 0 for as long as it takes).
    read_ms: u64,
}

/// One connection's take-over of its agent's own tools: a loopback channel, and the plugin of each session request.
pub(crate) struct Interception {
    shared: Arc<Shared>,
    harness: String,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    private: Private,
    /// The plugin of each session request by the request's id, then by its session's once answered, with its secret.
    plugins: Mutex<BTreeMap<String, (Written, String)>>,
}

impl core::fmt::Debug for Interception {
    /// Never a secret.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Interception")
            .field("env", &self.shared.env.name())
            .field("harness", &self.harness)
            .finish_non_exhaustive()
    }
}

/// How long a forwarder may take for its answer: the environment's `hook_seconds`, none being as long as a harness's hook can wait.
fn answer_seconds(env: &Env) -> u64 {
    match env.hook_seconds() {
        0 => LONGEST_HOOK_SECONDS,
        set => set.min(LONGEST_HOOK_SECONDS),
    }
}

impl Interception {
    /// Takes over the own tools of the agent `harness` (its vendor's `agent`) for a connection whose tools are served from `env`,
    /// as `hook` describes them; `program` and `before` run the host's own commands, and `max_bytes` is `[host] max_message_bytes`.
    ///
    /// # Errors
    ///
    /// The channel cannot be opened.
    pub(crate) fn open(
        env: Arc<Env>,
        (harness, hook): (&str, ToolHook),
        (program, before): (&str, &[String]),
        servers: Vec<(String, Rules)>,
        modes: Arc<Modes>,
        max_bytes: usize,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let seconds = answer_seconds(&env);
        // A tool call is judged by this, so an environment that is slow to say what it is never decides how long the hook takes.
        let info = env.info();
        let max_handlers = match env.hook_handlers() {
            0 => usize::MAX,
            set => usize::try_from(set).unwrap_or(usize::MAX),
        };
        let read_ms = env.hook_read_ms();
        let shared = Arc::new(Shared {
            handlers: AtomicUsize::new(0),
            max_handlers,
            read_ms,
            env,
            info,
            hook,
            harness: harness.to_owned(),
            servers,
            own_folder: AtomicBool::new(true),
            home_variable: crate::vendors::by_agent(harness)
                .and_then(|vendor| vendor.home_variable)
                .map(str::to_owned),
            modes,
            program: program.to_owned(),
            before: before.to_vec(),
            secrets: Mutex::new(BTreeMap::new()),
            latest: Mutex::new(BTreeMap::new()),
            ops: Mutex::new(BTreeMap::new()),
            originals: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(BTreeMap::new()),
            max_bytes,
            seconds,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("canoa-env-tools".to_owned())
                .spawn(move || accept(&listener, &shared, &stop))?
        };
        Ok(Self {
            shared,
            harness: harness.to_owned(),
            addr,
            stop,
            thread: Some(thread),
            private: Private::new("canoa-hooks"),
            plugins: Mutex::new(BTreeMap::new()),
        })
    }

    /// The `_meta` a session request (its JSON-RPC id `request`, for a session that works in `cwd`) gains to load the plugin of the
    /// agent's hook, written for it with a secret of its own.
    ///
    /// # Errors
    ///
    /// Why it cannot be given, for the client: the session would run the agent's tools on this computer, so it is refused.
    pub(crate) fn plugin_for(&self, request: &str, cwd: &str) -> Result<Value, String> {
        let refused = |why: String| {
            format!(
                "this session's tools are served from environment {}, and the hook that runs them there could not be given ({why})",
                self.shared.env.name()
            )
        };
        let secret = random_hex().map_err(|error| refused(error.to_string()))?;
        let channel = json!({
            "addr": self.addr.to_string(),
            "secret": &secret,
            "max_bytes": self.shared.max_bytes,
            "seconds": self.shared.seconds,
            "home_variable": self.shared.home_variable,
        })
        .to_string()
        .into_bytes();
        let under = self
            .private
            .ensure()
            .map_err(|error| refused(error.to_string()))?;
        let written = Written::create(under, &BTreeMap::from([(CHANNEL_FILE.to_owned(), channel)]))
            .map_err(|error| refused(error.to_string()))?;
        let at = written.root().join(CHANNEL_FILE);
        let Some(prefix) = forwarder(&self.shared.program, &self.shared.before, &at) else {
            return Err(refused(
                "the host's program is at a path a hook command cannot quote".to_owned(),
            ));
        };
        let request_hooks = HooksRequest::new(
            vec![
                HookCommand::new(HookKind::PreTool, format!("{prefix} {KIND_TOOL}")),
                HookCommand::new(HookKind::Start, format!("{prefix} {KIND_START}")),
            ],
            self.shared.seconds,
        );
        let delivery =
            crate::vendors::behave(&self.harness, |vendor| vendor.hooks_delivery(request_hooks))
                .and_then(Result::ok)
                .flatten()
                .ok_or_else(|| refused("the agent gives no hooks".to_owned()))?;
        for file in &delivery.files {
            written
                .add(&file.path, file.text.as_bytes())
                .map_err(|error| refused(error.to_string()))?;
        }
        let meta = plugin_meta(&delivery.plugin, &written).map_err(refused)?;
        lock(&self.shared.secrets).insert(
            secret.clone(),
            Holder {
                session: None,
                cwd: cwd.to_owned(),
                channel: at,
                started: false,
            },
        );
        lock(&self.plugins).insert(request.to_owned(), (written, secret));
        Ok(meta)
    }

    /// The agent answered the session request `request`: `session` when it opened one, else nothing, and its plugin ends.
    pub(crate) fn answered(&self, request: &str, session: Option<&str>) {
        let Some((written, secret)) = lock(&self.plugins).remove(request) else {
            return;
        };
        match session {
            Some(session) => {
                if let Some(holder) = lock(&self.shared.secrets).get_mut(&secret) {
                    holder.session = Some(session.to_owned());
                }
                lock(&self.shared.latest).insert(session.to_owned(), secret.clone());
                // A session loaded again keeps the plugin it had, beside the new one, until it closes.
                lock(&self.plugins)
                    .insert(format!("session:{session}:{request}"), (written, secret));
            }
            None => self.revoke(&secret),
        }
    }

    /// The session ended: its secrets, the commands they prepared and its plugins end.
    pub(crate) fn closed(&self, session: &str) {
        let held: Vec<String> = {
            let secrets = lock(&self.shared.secrets);
            secrets
                .iter()
                .filter(|(_, holder)| holder.session.as_deref() == Some(session))
                .map(|(secret, _)| secret.clone())
                .collect()
        };
        for secret in held {
            self.revoke(&secret);
        }
        let prefix = format!("session:{session}:");
        lock(&self.plugins).retain(|key, _| !key.starts_with(&prefix));
        lock(&self.shared.latest).remove(session);
    }

    /// Whether the plugin of `session`'s latest request reported that the agent's session started with it: until then its hook is
    /// not known to be in force, so its prompts wait.
    pub(crate) fn started(&self, session: &str) -> bool {
        let Some(secret) = lock(&self.shared.latest).get(session).cloned() else {
            return false;
        };
        lock(&self.shared.secrets)
            .get(&secret)
            .is_some_and(|holder| holder.started)
    }

    /// How long a prompt waits for the start report: the environment's `hook_seconds`.
    pub(crate) fn seconds(&self) -> u64 {
        self.shared.seconds
    }

    /// A test never reads the folder of the person who runs it.
    #[cfg(test)]
    pub(crate) fn own_folder_off(&self) {
        self.shared.own_folder.store(false, Ordering::SeqCst);
    }

    /// The oldest version of the agent the hook is honoured in, as its vendor says.
    pub(crate) fn min_version(&self) -> Option<&str> {
        self.shared.hook.min_version.as_deref()
    }

    /// What the agent announced of a tool call in `update` (a `session/update` of `session`): a call is remembered with its tool and
    /// command, and when it is done it must be one the hook went through.
    ///
    /// # Errors
    ///
    /// Why a call that is done did not go through the hook: it is a tool the take-over does not allow that ran, or a command the
    /// hook did not prepare, so it ran on this computer.
    pub(crate) fn saw(&self, session: &str, update: &Value) -> Result<(), String> {
        let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
            return Ok(());
        };
        let key = (session.to_owned(), id.to_owned());
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("tool_call") => {
                let pointer = crate::vendors::by_agent(&self.harness)
                    .and_then(|vendor| vendor.tool_name);
                let name = pointer
                    .and_then(|pointer| update.get("_meta")?.pointer(pointer))
                    .and_then(Value::as_str);
                let Some(name) = name else {
                    return Ok(());
                };
                let command = self
                    .shared
                    .hook
                    .tools
                    .iter()
                    .find(|tool| tool.name == name)
                    .and_then(|tool| tool.command.as_deref())
                    .and_then(|key| update.pointer("/rawInput")?.get(key)?.as_str())
                    .map(str::to_owned);
                let mut calls = lock(&self.shared.calls);
                while calls.len() >= MAX_SEEN {
                    calls.pop_first();
                }
                calls.insert(key, (name.to_owned(), command));
                Ok(())
            }
            Some("tool_call_update")
                if update.get("status").and_then(Value::as_str) == Some("completed") =>
            {
                let Some((name, command)) = lock(&self.shared.calls).remove(&key) else {
                    return Ok(());
                };
                self.went_through(session, &name, command.as_deref())
            }
            _ => Ok(()),
        }
    }

    /// Whether a call of the tool `name` (with `command`, for a shell) that ran is one the hook let run as it is or prepared.
    fn went_through(&self, session: &str, name: &str, command: Option<&str>) -> Result<(), String> {
        let hook = &self.shared.hook;
        let Some(tool) = hook.tools.iter().find(|tool| tool.name == name) else {
            return if hook
                .pass_prefixes
                .iter()
                .any(|prefix| !prefix.is_empty() && name.starts_with(prefix.as_str()))
            {
                Ok(())
            } else {
                Err(format!(
                    "{name} ran, and it is not a tool the take-over of this session's tools lets run"
                ))
            };
        };
        match tool.role {
            ToolRole::Pass => Ok(()),
            ToolRole::Files => Err(format!(
                "{name} ran on this computer: the hook that refuses it did not"
            )),
            _ => {
                let Some(command) = command else {
                    return Ok(());
                };
                let secrets: Vec<String> = lock(&self.shared.secrets)
                    .iter()
                    .filter(|(_, holder)| holder.session.as_deref() == Some(session))
                    .map(|(secret, _)| secret.clone())
                    .collect();
                let originals = lock(&self.shared.originals);
                let prepared = secrets.iter().any(|secret| {
                    originals
                        .get(secret)
                        .is_some_and(|all| all.iter().any(|each| each == command))
                });
                if prepared {
                    Ok(())
                } else {
                    Err(format!(
                        "{name} ran a command that this session's hook did not prepare, so it ran on this computer"
                    ))
                }
            }
        }
    }

    fn revoke(&self, secret: &str) {
        lock(&self.shared.secrets).remove(secret);
        lock(&self.shared.ops).retain(|_, op| op.secret != secret);
        lock(&self.shared.originals).remove(secret);
    }

    /// The channel file of the plugin written for `request`, for tests.
    #[cfg(test)]
    pub(crate) fn channel_of(&self, request: &str) -> Option<PathBuf> {
        let plugins = lock(&self.plugins);
        let (_, secret) = plugins.get(request)?;
        lock(&self.shared.secrets)
            .get(secret)
            .map(|holder| holder.channel.clone())
    }
}

impl Drop for Interception {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        lock(&self.shared.secrets).clear();
        lock(&self.shared.ops).clear();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The accepting thread: a handler thread for each connection, until `stop`.
fn accept(listener: &TcpListener, shared: &Arc<Shared>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, from)) => {
                // Only this user's own processes are heard where the system says whose a connection is.
                let Ok(local) = stream.local_addr() else {
                    continue;
                };
                if !from.ip().is_loopback()
                    || !crate::loopback::ours(from, local)
                    || stream.set_nonblocking(false).is_err()
                {
                    continue;
                }
                // Past the bound a connection is dropped unread, which refuses that call: idle connections of another user
                // cannot take the host's threads.
                if shared.handlers.fetch_add(1, Ordering::SeqCst) >= shared.max_handlers {
                    shared.handlers.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let own = Arc::clone(shared);
                let spawned = std::thread::Builder::new()
                    .name("canoa-env-tool-call".to_owned())
                    .spawn(move || {
                        handle(&stream, &own);
                        own.handlers.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    shared.handlers.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

fn write_line(mut stream: &TcpStream, value: &Value) {
    let mut out = value.to_string().into_bytes();
    out.push(b'\n');
    let _ = stream.write_all(&out);
}

/// The public name of `secret` that a request carries in its place, so the secret never crosses the channel.
fn key_of(secret: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"canoa environment channel key\0");
    digest.update(secret.as_bytes());
    data_encoding::HEXLOWER.encode(&digest.finalize()[..16])
}

/// HMAC-SHA256 (RFC 2104) of `parts`, each preceded by its length, under `secret`, in hexadecimal: what proves a request is the
/// forwarder's and an answer the host's to whichever end holds the secret, and to nothing that only took the port.
fn mac(secret: &str, parts: &[&str]) -> String {
    const BLOCK: usize = 64;
    let mut key = [0_u8; BLOCK];
    if secret.len() > BLOCK {
        key[..32].copy_from_slice(&Sha256::digest(secret.as_bytes()));
    } else {
        key[..secret.len()].copy_from_slice(secret.as_bytes());
    }
    let mut inner = Sha256::new();
    inner.update(key.map(|byte| byte ^ 0x36));
    for part in parts {
        inner.update((part.len() as u64).to_be_bytes());
        inner.update(part.as_bytes());
    }
    let mut outer = Sha256::new();
    outer.update(key.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    data_encoding::HEXLOWER.encode(&outer.finalize())
}

/// An answer of the host's: `field` (`output`, `refused` or `ok`) holding `text`, signed for the request's `nonce`.
fn answer_line(stream: &TcpStream, secret: &str, nonce: &str, (field, text): (&str, &str)) {
    let signed = mac(secret, &["answer", &key_of(secret), nonce, field, text]);
    let mut line = Map::new();
    line.insert("nonce".to_owned(), json!(nonce));
    line.insert(field.to_owned(), json!(text));
    line.insert("mac".to_owned(), json!(signed));
    write_line(stream, &Value::Object(line));
}

/// One request. Anything that is not one well-formed request with a secret the host holds is dropped unanswered, which the
/// forwarder takes for a refusal.
fn handle(stream: &TcpStream, shared: &Shared) {
    // The forwarder writes its request as soon as it connects; only a connection whose secret holds gets longer.
    let wait = match shared.read_ms {
        0 => Duration::from_secs(LONGEST_HOOK_SECONDS),
        set => Duration::from_millis(set),
    };
    let Some(line) = read_line(
        stream,
        Instant::now() + wait,
        shared.max_bytes.saturating_add(ENVELOPE),
    ) else {
        return;
    };
    let Ok(request) = serde_json::from_slice::<Value>(&line) else {
        return;
    };
    let text = |key: &str| request.get(key).and_then(Value::as_str);
    let (Some(key), Some(nonce), Some(kind), Some(signed)) = (
        text("key"),
        text("nonce").filter(|nonce| (8..=64).contains(&nonce.len())),
        text("kind"),
        text("mac"),
    ) else {
        return;
    };
    let (id, payload) = (text("id"), text("payload"));
    // The request names its secret by a public key and proves it holds it; the secret itself is never sent.
    let found = lock(&shared.secrets)
        .iter()
        .find(|(secret, _)| key_of(secret) == key)
        .map(|(secret, holder)| (secret.clone(), holder.clone()));
    let Some((secret, holder)) = found else {
        return;
    };
    let expected = mac(
        &secret,
        &[
            "request",
            key,
            nonce,
            kind,
            id.unwrap_or(""),
            payload.unwrap_or(""),
        ],
    );
    if !same(&expected, signed) {
        return;
    }
    let secret = secret.as_str();
    let _ = stream.set_write_timeout(Some(Duration::from_secs(shared.seconds)));
    match kind {
        KIND_TOOL => {
            let mode = holder
                .session
                .as_deref()
                .map_or(Mode::Manual, |session| shared.modes.of(session));
            let mut payload = payload.and_then(|text| serde_json::from_str::<Value>(text).ok());
            let home = payload
                .as_mut()
                .and_then(Value::as_object_mut)
                .and_then(|payload| payload.remove(HOME_KEY))
                .and_then(|home| home.as_str().map(str::to_owned));
            let verdict = match &payload {
                Some(payload) => evaluate(
                    &shared.hook,
                    (shared.env.name(), &shared.info),
                    mode,
                    payload,
                    &mut |shell, command| shared.prepare(secret, &holder.channel, shell, command),
                    &mut |name, input| shared.person(&holder, home.as_deref(), (name, input)),
                ),
                None => Verdict::Deny("the forwarder sent no payload".to_owned()),
            };
            match answer(&shared.hook, verdict) {
                Some(output) => answer_line(stream, secret, nonce, ("output", &output)),
                None => answer_line(
                    stream,
                    secret,
                    nonce,
                    ("refused", "the agent's form of an answer is not usable"),
                ),
            }
        }
        KIND_EXEC => shared.exec(stream, (&holder, secret, nonce), id),
        KIND_START => {
            if let Some(held) = lock(&shared.secrets).get_mut(secret) {
                held.started = true;
            }
            answer_line(stream, secret, nonce, ("output", ""));
        }
        _ => {}
    }
}

impl Shared {
    /// What the person's own rules make of the call `name` with `input` in the session `holder` is: only a call to a tool of an
    /// environment is judged, by the rules the agent's vendor reads where the agent reads them (`home` is the agent's own folder
    /// as the agent's process names it, when it does).
    fn person(
        &self,
        holder: &Holder,
        home: Option<&str>,
        call: (&str, &Map<String, Value>),
    ) -> Option<Judged> {
        let load = || {
            let folder = home
                .filter(|home| Path::new(home).is_absolute())
                .map(str::to_owned)
                .or_else(|| {
                    if !self.own_folder.load(Ordering::SeqCst) {
                        return None;
                    }
                    crate::vendors::by_agent(&self.harness)
                        .and_then(|vendor| {
                            vendor.folder(std::env::home_dir().as_deref(), &|name| {
                                std::env::var_os(name)
                            })
                        })
                        .map(|folder| folder.to_string_lossy().into_owned())
                });
            let request = RulesRequest::new(folder, holder.cwd.clone());
            crate::vendors::behave(&self.harness, |vendor| vendor.person_rules(request))
                .and_then(Result::ok)
                .unwrap_or_default()
        };
        person::judge(&self.hook, &self.servers, &load, call)
    }

    /// Keeps `command` for an `env-exec` of the session holding `secret`, and gives the command that runs it.
    fn prepare(
        &self,
        secret: &str,
        channel: &Path,
        shell: ShellFamily,
        command: &str,
    ) -> Result<String, String> {
        let prefix = forwarder(&self.program, &self.before, channel)
            .ok_or("the host's program is at a path a command cannot quote")?;
        let id = random_hex().map_err(|error| error.to_string())?;
        let run = rewritten(&prefix, &id, shell, command)?;
        let mut ops = lock(&self.ops);
        ops.retain(|_, op| !self.expired(op));
        let most = usize::try_from(self.env.max_prepared()).unwrap_or(usize::MAX);
        // Past the bound the oldest waits no more; its env-exec is refused.
        while most != 0 && ops.len() >= most {
            let Some(oldest) = ops
                .iter()
                .min_by_key(|(_, op)| op.at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            ops.remove(&oldest);
        }
        ops.insert(
            id,
            Op {
                secret: secret.to_owned(),
                shell,
                command: command.to_owned(),
                at: Instant::now(),
            },
        );
        let mut originals = lock(&self.originals);
        let seen = originals.entry(secret.to_owned()).or_default();
        while seen.len() >= MAX_SEEN {
            seen.pop_front();
        }
        seen.push_back(command.to_owned());
        Ok(run)
    }

    /// Whether `op` waited longer than the environment's `prepared_seconds`.
    fn expired(&self, op: &Op) -> bool {
        match self.env.prepared_seconds() {
            0 => false,
            set => op.at.elapsed() > Duration::from_secs(set),
        }
    }

    /// Runs the command prepared as `id` in the environment, sending its output as it comes and then how it ended.
    fn exec(
        &self,
        stream: &TcpStream,
        (holder, secret, nonce): (&Holder, &str, &str),
        id: Option<&str>,
    ) {
        let refuse = |why: &str| answer_line(stream, secret, nonce, ("refused", why));
        let op = id.and_then(|id| {
            let mut ops = lock(&self.ops);
            let mine = ops.get(id).is_some_and(|op| same(&op.secret, secret));
            if mine { ops.remove(id) } else { None }
        });
        let Some(op) = op else {
            return refuse(
                "no command of this session's waits under that id: only a command its hook rewrote runs, and once",
            );
        };
        if self.expired(&op) {
            return refuse(&format!(
                "the command waited longer than {} seconds, the prepared_seconds of environment {}, and does not run",
                self.env.prepared_seconds(),
                self.env.name()
            ));
        }
        let mode = holder
            .session
            .as_deref()
            .map_or(Mode::Manual, |session| self.modes.of(session));
        if mode == Mode::Plan {
            return refuse("the session is in plan mode, so no command runs");
        }
        if let Err(why) = self.env.needs(capability::EXEC) {
            return refuse(&why);
        }
        let (program, args) = shell_command(op.shell);
        let mut spec = SpawnSpec::new(program);
        for arg in args {
            spec = spec.arg(*arg);
        }
        spec = spec
            .arg(op.command.as_str())
            .error_output(ErrorOutput::Merged);
        let process = match self.env.start(spec) {
            Ok(process) => Remote::new(process),
            Err(why) => return refuse(&why),
        };
        drop(process.input());
        answer_line(stream, secret, nonce, ("ok", ""));
        let gone = Arc::new(AtomicBool::new(false));
        let reader = {
            let (process, gone) = (process.clone(), Arc::clone(&gone));
            let sink = stream.try_clone();
            std::thread::Builder::new()
                .name("canoa-env-exec-output".to_owned())
                .spawn(move || {
                    let Ok(mut sink) = sink else {
                        gone.store(true, Ordering::SeqCst);
                        return;
                    };
                    let mut output = process.output();
                    let mut buf = [0_u8; 8192];
                    while let Ok(n) = output.read(&mut buf)
                        && n > 0
                    {
                        if frame(&mut sink, TAG_OUTPUT, &buf[..n]).is_err() {
                            gone.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                })
                .ok()
        };
        // The forwarder sends nothing after its request: its end is the agent's harness having stopped the command.
        let watcher = stream.try_clone().ok().and_then(|mut watched| {
            let gone = Arc::clone(&gone);
            let _ = watched.set_read_timeout(None);
            std::thread::Builder::new()
                .name("canoa-env-exec-peer".to_owned())
                .spawn(move || {
                    let mut byte = [0_u8; 1];
                    let _ = watched.read(&mut byte);
                    gone.store(true, Ordering::SeqCst);
                })
                .ok()
        });
        let limit = self.env.exec_seconds();
        let started = Instant::now();
        let mut error = None;
        // The environment's own wait is asked on a thread of its own: a call that does not return (a link that stalled) must not hide
        // the forwarder having left or the time limit, which are looked at every slice here.
        let (ended, waited) = channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        let waiter = {
            let (process, abandoned) = (process.clone(), Arc::clone(&abandoned));
            std::thread::Builder::new()
                .name("canoa-env-exec-wait".to_owned())
                .spawn(move || {
                    while !abandoned.load(Ordering::SeqCst) {
                        match process.exit(WAIT_SLICE_MS) {
                            Ok(exit) if exit.running => {}
                            other => {
                                let _ = ended.send(other);
                                return;
                            }
                        }
                    }
                })
        };
        let stop = |process: &Remote| {
            let process = process.clone();
            // Ending it is a call to the environment too, and may not return either.
            let _ = std::thread::Builder::new()
                .name("canoa-env-exec-kill".to_owned())
                .spawn(move || {
                    let _ = process.kill();
                });
        };
        let mut exit = None;
        let mut stopped = false;
        if waiter.is_err() {
            error = Some("the command's end cannot be waited for".to_owned());
            stop(&process);
            stopped = true;
        }
        while !stopped {
            match waited.recv_timeout(Duration::from_millis(WAIT_SLICE_MS)) {
                Ok(Ok(done)) => {
                    exit = Some(done);
                    break;
                }
                Ok(Err(failed)) => {
                    error = Some(failed.to_string());
                    stop(&process);
                    stopped = true;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if gone.load(Ordering::SeqCst) {
                        stop(&process);
                        stopped = true;
                    } else if limit != 0 && started.elapsed() >= Duration::from_secs(limit) {
                        stop(&process);
                        error = Some(format!(
                            "the command ran longer than {limit} seconds, the exec_seconds of environment {}, and was ended",
                            self.env.name()
                        ));
                        stopped = true;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if stopped {
            // Its status, if the environment says it soon; else none, and the reader is left to end when its own read does.
            exit = waited
                .recv_timeout(END_GRACE)
                .ok()
                .and_then(Result::ok);
        }
        abandoned.store(true, Ordering::SeqCst);
        if !stopped && let Some(reader) = reader {
            let _ = reader.join();
        }
        let code = exit.map_or(-1, |exit| code_of(&exit));
        let signed = mac(
            secret,
            &[
                "end",
                &key_of(secret),
                nonce,
                &code.to_string(),
                error.as_deref().unwrap_or(""),
            ],
        );
        let end = json!({"code": code, "error": error, "mac": signed});
        if let Ok(mut sink) = stream.try_clone() {
            let _ = frame(&mut sink, TAG_END, end.to_string().as_bytes());
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
        if let Some(watcher) = watcher {
            let _ = watcher.join();
        }
    }
}

/// The program and its first arguments that run a command written for `shell`; the command follows them.
fn shell_command(shell: ShellFamily) -> (&'static str, &'static [&'static str]) {
    match shell {
        ShellFamily::Bash => ("bash", &["-c"]),
        ShellFamily::Zsh => ("zsh", &["-c"]),
        ShellFamily::Powershell => ("pwsh", &["-NoProfile", "-Command"]),
        _ => ("sh", &["-c"]),
    }
}

/// Writes one frame: its tag, its length as four bytes, big-endian, and its bytes.
fn frame(sink: &mut impl Write, tag: u8, bytes: &[u8]) -> std::io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
    let mut head = [0_u8; 5];
    head[0] = tag;
    head[1..].copy_from_slice(&length.to_be_bytes());
    sink.write_all(&head)?;
    sink.write_all(bytes)?;
    sink.flush()
}

/// The channel as a session's directory holds it.
struct Channel {
    addr: SocketAddr,
    secret: String,
    max_bytes: usize,
    seconds: u64,
    /// The variable whose value (the agent's own folder, in the agent's process) the forwarder gives the host.
    home_variable: Option<String>,
}

fn read_channel(path: &Path) -> Result<Channel, String> {
    let bytes = std::fs::metadata(path)
        .ok()
        .filter(|meta| meta.len() <= MAX_CHANNEL_FILE)
        .and_then(|_| std::fs::read(path).ok())
        .ok_or("the channel to the host cannot be read")?;
    let read: Value =
        serde_json::from_slice(&bytes).map_err(|_| "the channel to the host is not readable")?;
    let addr = read
        .get("addr")
        .and_then(Value::as_str)
        .and_then(|addr| addr.parse::<SocketAddr>().ok())
        .filter(|addr| addr.ip().is_loopback())
        .ok_or("the channel names no loopback address")?;
    let secret = read
        .get("secret")
        .and_then(Value::as_str)
        .ok_or("the channel holds no secret")?
        .to_owned();
    let max_bytes = read
        .get("max_bytes")
        .and_then(Value::as_u64)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or("the channel names no size")?;
    let seconds = read
        .get("seconds")
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .ok_or("the channel names no time")?;
    let home_variable = read
        .get("home_variable")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Channel {
        addr,
        secret,
        max_bytes,
        seconds,
        home_variable,
    })
}

/// Connects to the host, sends `request` and reads its first answer line, which must carry `nonce`, all by `deadline`: one budget
/// for the connection, the request and every read of the answer, so the forwarder answers its hook before the hook's own timeout,
/// which would let the call go on. What follows the line is left in the stream.
fn ask(
    channel: &Channel,
    mut request: Map<String, Value>,
    nonce: &str,
    deadline: Instant,
) -> Result<(TcpStream, Value), String> {
    let left = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| "the host did not answer in time".to_owned())
    };
    let key = key_of(&channel.secret);
    let field = |name: &str| {
        request
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let signed = mac(
        &channel.secret,
        &[
            "request",
            &key,
            nonce,
            &field("kind"),
            &field("id"),
            &field("payload"),
        ],
    );
    request.insert("key".to_owned(), json!(key));
    request.insert("nonce".to_owned(), json!(nonce));
    request.insert("mac".to_owned(), json!(signed));
    let mut stream = TcpStream::connect_timeout(&channel.addr, left()?)
        .map_err(|error| format!("the host cannot be reached ({error})"))?;
    let mut line = Value::Object(request).to_string().into_bytes();
    line.push(b'\n');
    stream
        .set_write_timeout(Some(left()?))
        .and_then(|()| stream.write_all(&line))
        .map_err(|error| format!("the host cannot be reached ({error})"))?;
    let bound = channel.max_bytes.saturating_add(ENVELOPE);
    let mut answer = Vec::new();
    let mut byte = [0_u8; 1];
    // A byte at a time, so nothing after the line is taken from the stream and every read is held to what is left.
    loop {
        stream
            .set_read_timeout(Some(left()?))
            .map_err(|error| error.to_string())?;
        match stream.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => break,
            Ok(1) if answer.len() < bound => answer.push(byte[0]),
            Ok(_) => return Err("the host did not answer".to_owned()),
            Err(error) => return Err(format!("the host did not answer ({error})")),
        }
    }
    let answer: Value = serde_json::from_slice(&answer)
        .map_err(|_| "the host's answer is not readable".to_owned())?;
    if answer.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err("the answer is not the host's".to_owned());
    }
    // Only the end holding the secret signs an answer: a process that took the port cannot make one.
    let signed = ["output", "refused", "ok"].into_iter().find_map(|field| {
        let text = answer.get(field)?.as_str()?;
        Some(mac(&channel.secret, &["answer", &key, nonce, field, text]))
    });
    let given = answer.get("mac").and_then(Value::as_str).unwrap_or("");
    if !signed.is_some_and(|signed| same(&signed, given)) {
        return Err("the answer is not signed by the host".to_owned());
    }
    Ok((stream, answer))
}

/// What `canoa hook-forward --channel FILE env-tool|env-exec [ID]` does, with the process's streams: answers the agent's hook for
/// one tool call, or runs the command `id` names in the environment. It answers the status to exit with: for `env-tool` 0 with
/// the answer printed, or 2 (refused) with why on standard error; for `env-exec` the command's own status, or 126 when nothing
/// ran.
pub(crate) fn forward(
    kind: &str,
    channel: &Path,
    id: Option<&str>,
    stdin: impl Read,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    match kind {
        KIND_TOOL => match tool(channel, stdin) {
            Ok(output) => {
                if stdout
                    .write_all(output.as_bytes())
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    return REFUSED;
                }
                0
            }
            Err(why) => {
                let _ = writeln!(stderr, "Refused: {why}; nothing ran on this computer.");
                REFUSED
            }
        },
        KIND_EXEC => match id {
            Some(id) => exec(channel, id, stdout, stderr),
            None => {
                let _ = writeln!(stderr, "{PROGRAM}: env-exec names no command");
                NOT_RUN
            }
        },
        KIND_START => match started(channel, stdin) {
            Ok(()) => 0,
            Err(why) => {
                let _ = writeln!(
                    stderr,
                    "{PROGRAM}: the host was not told the session started ({why}); its prompts are refused"
                );
                1
            }
        },
        _ => {
            let _ = writeln!(
                stderr,
                "{PROGRAM}: {kind} is not a kind this forwarder knows"
            );
            REFUSED
        }
    }
}

/// Tells the host the agent's session started with the hook in force.
fn started(channel: &Path, stdin: impl Read) -> Result<(), String> {
    let deadline = Instant::now();
    let channel = read_channel(channel)?;
    let deadline = deadline + Duration::from_secs(channel.seconds);
    // What the agent says of its start is not needed, only that the hook ran.
    let _ = std::io::copy(
        &mut stdin.take(u64::try_from(channel.max_bytes).unwrap_or(u64::MAX)),
        &mut std::io::sink(),
    );
    let nonce = random_hex().map_err(|error| error.to_string())?;
    let mut request = Map::new();
    request.insert("kind".to_owned(), json!(KIND_START));
    ask(&channel, request, &nonce, deadline).map(drop)
}

fn tool(channel: &Path, stdin: impl Read) -> Result<String, String> {
    let deadline = Instant::now();
    let channel = read_channel(channel)?;
    let deadline = deadline + Duration::from_secs(channel.seconds);
    let mut bytes = Vec::new();
    stdin
        .take(
            u64::try_from(channel.max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|error| format!("the call could not be read ({error})"))?;
    if bytes.len() > channel.max_bytes {
        return Err(format!(
            "the call is larger than [host] max_message_bytes ({} bytes)",
            channel.max_bytes
        ));
    }
    let mut payload: Value =
        serde_json::from_slice(&bytes).map_err(|_| "the call is not a JSON document".to_owned())?;
    let Some(object) = payload.as_object_mut() else {
        return Err("the call is not a JSON object".to_owned());
    };
    // The host reads the agent's own settings from the folder this process's environment names, which the agent set for its hook.
    object.remove(HOME_KEY);
    if let Some(home) = channel
        .home_variable
        .as_deref()
        .and_then(|variable| std::env::var(variable).ok())
    {
        object.insert(HOME_KEY.to_owned(), Value::String(home));
    }
    let payload = payload.to_string();
    let nonce = random_hex().map_err(|error| error.to_string())?;
    let mut request = Map::new();
    request.insert("kind".to_owned(), json!(KIND_TOOL));
    request.insert("payload".to_owned(), json!(payload));
    let (_, answer) = ask(&channel, request, &nonce, deadline)?;
    if let Some(why) = answer.get("refused").and_then(Value::as_str) {
        return Err(why.to_owned());
    }
    answer
        .get("output")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "the host gave no answer".to_owned())
}

fn exec(channel: &Path, id: &str, stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    let failed = |stderr: &mut dyn Write, why: &str| {
        let _ = writeln!(stderr, "{PROGRAM}: {why}; nothing ran on this computer.");
        NOT_RUN
    };
    let begun = Instant::now();
    let channel = match read_channel(channel) {
        Ok(channel) => channel,
        Err(why) => return failed(stderr, &why),
    };
    let deadline = begun + Duration::from_secs(channel.seconds);
    let nonce = match random_hex() {
        Ok(nonce) => nonce,
        Err(error) => return failed(stderr, &error.to_string()),
    };
    let mut request = Map::new();
    request.insert("kind".to_owned(), json!(KIND_EXEC));
    request.insert("id".to_owned(), json!(id));
    let (stream, header) = match ask(&channel, request, &nonce, deadline) {
        Ok(answered) => answered,
        Err(why) => return failed(stderr, &why),
    };
    if let Some(why) = header.get("refused").and_then(Value::as_str) {
        return failed(stderr, why);
    }
    // The command may run as long as the environment lets it: the agent's harness bounds the wait.
    if stream.set_read_timeout(None).is_err() {
        return failed(stderr, "the connection to the host failed");
    }
    let mut reader = std::io::BufReader::new(stream);
    loop {
        let mut head = [0_u8; 5];
        if reader.read_exact(&mut head).is_err() {
            let _ = writeln!(
                stderr,
                "{PROGRAM}: the host ended the command's connection before it said how the command ended"
            );
            return 1;
        }
        let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
        let length = usize::try_from(length).unwrap_or(usize::MAX);
        if length > channel.max_bytes.max(ENVELOPE) {
            let _ = writeln!(
                stderr,
                "{PROGRAM}: the host sent a frame larger than the channel allows"
            );
            return 1;
        }
        let mut bytes = vec![0_u8; length];
        if reader.read_exact(&mut bytes).is_err() {
            let _ = writeln!(
                stderr,
                "{PROGRAM}: the host ended the command's connection mid-frame"
            );
            return 1;
        }
        match head[0] {
            TAG_OUTPUT => {
                if stdout
                    .write_all(&bytes)
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    return 1;
                }
            }
            TAG_END => {
                let end: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let code = end.get("code").and_then(Value::as_i64).unwrap_or(-1);
                let error = end.get("error").and_then(Value::as_str).unwrap_or("");
                let signed = mac(
                    &channel.secret,
                    &[
                        "end",
                        &key_of(&channel.secret),
                        &nonce,
                        &code.to_string(),
                        error,
                    ],
                );
                if !same(
                    &signed,
                    end.get("mac").and_then(Value::as_str).unwrap_or(""),
                ) {
                    let _ = writeln!(
                        stderr,
                        "{PROGRAM}: the command's end is not signed by the host"
                    );
                    return 1;
                }
                if let Some(why) = end.get("error").and_then(Value::as_str) {
                    let _ = writeln!(stderr, "{PROGRAM}: {why}");
                }
                return end
                    .get("code")
                    .and_then(Value::as_i64)
                    .and_then(|code| u8::try_from(code).ok())
                    .unwrap_or(1);
            }
            _ => {
                let _ = writeln!(
                    stderr,
                    "{PROGRAM}: the host sent a frame this forwarder does not know"
                );
                return 1;
            }
        }
    }
}

#[cfg(test)]
mod tests;
