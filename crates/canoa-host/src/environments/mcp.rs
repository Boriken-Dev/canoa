//! Environments offered to an agent as tools: one MCP server for each, `env_<name>`, over loopback HTTP behind a bearer minted for
//! the connection (or, for an agent that takes only stdio, through `PROGRAM mcp-bridge` with the bearer in its environment), the way
//! the host's other tool servers are offered. A server has a tool for each capability its environment offers, so an agent sees
//! what is possible there and nothing else: `exec`, `terminal_*`, `fs_read`, `fs_write`, `fs_list`, and `call_<name>` for a
//! capability of the environment's own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use canoa::environment::{State, capability};
use canoa::store::protect::Protected;
use coqui_acp_api::{ErrorOutput, SpawnSpec, TerminalSize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::intercept::{Interception, Mode, Modes};
use super::remote::Remote;
use super::served::{Served, collect};
use super::{Env, Environments, HERE, Placement, briefing, lock};
use crate::delegate::BridgeCommand;
use crate::loopback::{self, Answer, Caller, Listener, Service};

/// The prefix of an environment's MCP server's name.
pub(crate) const SERVER_PREFIX: &str = "env_";
/// Requests of the client the host remembers an answer for.
const MAX_PENDING: usize = 64;

/// The name of the MCP server for the environment `name`: MCP names are words, so anything else becomes `_`.
pub(crate) fn server_name(name: &str) -> String {
    let word: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{SERVER_PREFIX}{word}")
}

/// The person's protected places as the tools of an environment hold them, and the modes of the connection's sessions.
pub(crate) struct Guard {
    list: Arc<Protected>,
    /// The home directory `~/` names: this computer's for `here`, none elsewhere, whose home the host does not know.
    home: Option<PathBuf>,
    modes: Arc<Modes>,
}

impl Guard {
    /// The places `lines` protect, for sessions whose modes are `modes`.
    #[cfg(test)]
    pub(super) fn listing(lines: &[&str], modes: Arc<Modes>) -> Self {
        Self {
            list: Arc::new(Protected::new(lines.iter().copied())),
            home: None,
            modes,
        }
    }

    /// No place protected, and every session asking.
    #[cfg(test)]
    fn open() -> Self {
        Self {
            list: Arc::new(Protected::new(core::iter::empty::<&str>())),
            home: None,
            modes: Arc::new(Modes::default()),
        }
    }
}

/// What one bearer opens: the tools of one environment, for one session of a connection.
pub(crate) struct Access {
    env: Arc<Env>,
    exec: Served,
    shells: Mutex<BTreeMap<String, Shell>>,
    next: AtomicU64,
    calls: AtomicUsize,
    guard: Guard,
    /// The session the bearer was given to, once the agent has opened it.
    session: Mutex<Option<String>>,
}

impl Caller for Access {
    fn calls(&self) -> &AtomicUsize {
        &self.calls
    }
}

struct Shell {
    process: Remote,
    output: Arc<Mutex<super::served::Output>>,
}

impl Drop for Access {
    fn drop(&mut self) {
        for shell in lock(&self.shells).values() {
            let _ = shell.process.kill();
        }
        self.exec.end_all();
    }
}

fn text_result(text: impl Into<String>, error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}], "isError": error})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
    })
}

impl Access {
    /// The tools of `env`, with no place protected.
    #[cfg(test)]
    pub(super) fn new(env: Arc<Env>) -> Self {
        Self::guarded(env, Guard::open())
    }

    /// The tools of `env`, holding the places `guard` protects.
    pub(super) fn guarded(env: Arc<Env>, guard: Guard) -> Self {
        Self {
            exec: Served::new(Arc::clone(&env)),
            env,
            shells: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            calls: AtomicUsize::new(0),
            guard,
            session: Mutex::new(None),
        }
    }

    /// Whether a file tool may act on `path`: not when the person's list protects it, or it cannot be placed, unless the session
    /// is in Bypass. A path is judged as written: the environment's links are its own and are not followed from here.
    fn may_touch(&self, path: &str) -> Result<(), String> {
        let session = lock(&self.session).clone();
        if session.is_some_and(|session| self.guard.modes.of(&session) == Mode::Bypass) {
            return Ok(());
        }
        if path.is_empty()
            || path.contains('\0')
            || path.starts_with('~')
            || Path::new(path)
                .components()
                .any(|part| part == Component::ParentDir)
            || path.split(['/', '\\']).any(|part| part == "..")
        {
            return Err(format!(
                "{path:?} cannot be placed (empty, starting with ~ or holding a .. part), so it is kept as a protected place is: give the path plainly"
            ));
        }
        if self
            .guard
            .list
            .protects(Path::new(path), self.guard.home.as_deref())
        {
            return Err(format!(
                "{path} is on the person's list of protected places ([permissions] protected), which this session's tools read or change only in the Bypass mode"
            ));
        }
        Ok(())
    }

    /// Binds the bearer to the session it was given to.
    pub(super) fn opened(&self, session: &str) {
        *lock(&self.session) = Some(session.to_owned());
    }

    /// `tools/list`: a tool for each capability the environment offers now.
    fn list(&self) -> Value {
        let info = self.env.info();
        let mut tools: Vec<Value> = Vec::new();
        if info.state != State::Ready {
            return json!({"tools": tools});
        }
        let on = format!("in the environment {}", self.env.name());
        if info.offers(capability::EXEC) {
            tools.push(tool(
                "exec",
                &format!("Runs a program {on} and answers its output and exit status."),
                json!({
                    "program": {"type": "string"},
                    "args": {"type": "array", "items": {"type": "string"}},
                    "cwd": {"type": "string"},
                }),
                &["program"],
            ));
        }
        if info.offers(capability::TERMINAL) {
            tools.push(tool(
                "terminal_open",
                &format!("Opens an interactive terminal {on} and answers its id."),
                json!({"shell": {"type": "string"}, "cwd": {"type": "string"}}),
                &[],
            ));
            tools.push(tool(
                "terminal_send",
                "Types text into a terminal (end a command with a newline).",
                json!({"id": {"type": "string"}, "text": {"type": "string"}}),
                &["id", "text"],
            ));
            tools.push(tool(
                "terminal_read",
                "Reads what a terminal wrote since the last read.",
                json!({"id": {"type": "string"}, "wait_ms": {"type": "integer"}}),
                &["id"],
            ));
            tools.push(tool(
                "terminal_close",
                "Ends a terminal.",
                json!({"id": {"type": "string"}}),
                &["id"],
            ));
        }
        if info.offers(capability::FS_READ) {
            tools.push(tool(
                "fs_read",
                &format!("Reads a text file {on}."),
                json!({"path": {"type": "string"}, "line": {"type": "integer"}, "limit": {"type": "integer"}}),
                &["path"],
            ));
        }
        if info.offers(capability::FS_WRITE) {
            tools.push(tool(
                "fs_write",
                &format!("Writes a text file {on}, replacing one there."),
                json!({"path": {"type": "string"}, "content": {"type": "string"}}),
                &["path", "content"],
            ));
        }
        if info.offers(capability::FS_READ) && info.offers(capability::FS_WRITE) {
            tools.push(tool(
                "edit",
                &format!(
                    "Replaces old_string with new_string in a file {on}, once unless replace_all; the file's other bytes stay as they are."
                ),
                json!({
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean"},
                }),
                &["path", "old_string", "new_string"],
            ));
            tools.push(tool(
                "apply_patch",
                &format!(
                    "Applies a patch {on}, written from *** Begin Patch to *** End Patch with *** Add File: and *** Update File: sections and @@ hunks of ' ', '-' and '+' lines."
                ),
                json!({"patch": {"type": "string"}}),
                &["patch"],
            ));
        }
        if info.offers(capability::FS_LIST) {
            tools.push(tool(
                "fs_list",
                &format!("Lists a folder {on}."),
                json!({"path": {"type": "string"}}),
                &["path"],
            ));
        }
        for each in &info.capabilities {
            if ![
                capability::EXEC,
                capability::TERMINAL,
                capability::FS_READ,
                capability::FS_WRITE,
                capability::FS_LIST,
            ]
            .contains(&each.name.as_str())
            {
                tools.push(tool(
                    &format!("call_{}", each.name.replace('.', "_")),
                    each.help
                        .as_deref()
                        .unwrap_or("A capability of this environment's own."),
                    json!({"arguments": {"type": "object"}}),
                    &[],
                ));
            }
        }
        json!({"tools": tools})
    }

    /// `tools/call`: the result of the tool `name`. A refusal is a result with `isError`, which names the missing capability.
    pub(super) fn call(&self, name: &str, arguments: &Value) -> Value {
        let done = match name {
            "exec" => self.run(arguments),
            "terminal_open" => self.open(arguments),
            "terminal_send" => self.send(arguments),
            "terminal_read" => self.read(arguments),
            "terminal_close" => self.close(arguments),
            "fs_read" => self.touching(arguments).and_then(|()| {
                self.exec
                    .answer("fs/read_text_file", arguments)
                    .map(|answer| answer["content"].as_str().unwrap_or_default().to_owned())
                    .map_err(|(_, why)| why)
            }),
            "fs_write" => self.touching(arguments).and_then(|()| {
                self.exec
                    .answer("fs/write_text_file", arguments)
                    .map(|_| "written".to_owned())
                    .map_err(|(_, why)| why)
            }),
            "edit" => self.edit(arguments),
            "apply_patch" => self.apply_patch(arguments),
            "fs_list" => self
                .touching(arguments)
                .and_then(|()| self.list_folder(arguments)),
            other => match other.strip_prefix("call_") {
                Some(word) => self.passthrough(word, arguments),
                None => Err(format!("this environment has no tool {other}")),
            },
        };
        match done {
            Ok(text) => text_result(text, false),
            Err(why) => text_result(why, true),
        }
    }

    /// [`Self::may_touch`] for the `path` of `arguments`.
    fn touching(&self, arguments: &Value) -> Result<(), String> {
        self.may_touch(
            arguments
                .get("path")
                .and_then(Value::as_str)
                .ok_or("path is text")?,
        )
    }

    /// Replaces text in a file as bytes, so a file that is not UTF-8 keeps every byte the edit does not change.
    fn edit(&self, arguments: &Value) -> Result<String, String> {
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key} is text"))
        };
        let path = text("path")?;
        self.may_touch(path)?;
        let (old, new) = (text("old_string")?, text("new_string")?);
        if old.is_empty() {
            return Err("old_string is empty".to_owned());
        }
        let all = arguments
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let bytes = self.env.read(path)?;
        let count = super::patch::count(&bytes, old.as_bytes());
        match count {
            0 => return Err(format!("old_string is not in {path}")),
            n if n > 1 && !all => {
                return Err(format!(
                    "old_string is in {path} {n} times: give more of the text around it, or set replace_all"
                ));
            }
            _ => {}
        }
        let changed = super::patch::replace(
            &bytes,
            old.as_bytes(),
            new.as_bytes(),
            all,
            self.env.message_bytes(),
        )?;
        self.env.write(path, changed)?;
        Ok(if count == 1 {
            format!("{path}: replaced once")
        } else {
            format!("{path}: replaced {count} times")
        })
    }

    /// Applies a patch through the environment's own reads and writes; every file it names is checked before any is written.
    fn apply_patch(&self, arguments: &Value) -> Result<String, String> {
        let text = arguments
            .get("patch")
            .and_then(Value::as_str)
            .ok_or("patch is text")?;
        let patch = super::patch::parse(text)?;
        for path in patch.paths() {
            self.may_touch(path)?;
        }
        let written = patch.apply(
            |path| self.env.read(path).map(Some),
            |path, bytes| self.env.write(path, bytes),
        )?;
        Ok(written
            .iter()
            .map(|path| format!("written {path}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn list_folder(&self, arguments: &Value) -> Result<String, String> {
        let path = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or("path is text")?;
        let entries = self.env.list(path)?;
        Ok(entries
            .iter()
            .map(|entry| {
                if entry.dir {
                    format!("{}/", entry.name)
                } else {
                    format!("{} {}", entry.name, entry.len.unwrap_or(0))
                }
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn passthrough(&self, word: &str, arguments: &Value) -> Result<String, String> {
        let info = self.env.info();
        let found = info
            .capabilities
            .iter()
            .find(|each| each.name.replace('.', "_") == word)
            .ok_or_else(|| format!("environment {} has no capability {word}", self.env.name()))?;
        let arguments = arguments
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        self.env.call(&found.name.clone(), &arguments.to_string())
    }

    fn run(&self, arguments: &Value) -> Result<String, String> {
        let mut request = arguments.clone();
        let program = arguments
            .get("program")
            .and_then(Value::as_str)
            .ok_or("program is text")?
            .to_owned();
        request["command"] = json!(program);
        let created = self
            .exec
            .answer("terminal/create", &request)
            .map_err(|(_, why)| why)?;
        let id = created["terminalId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let by = json!({"terminalId": id});
        let finished = {
            let exec = &self.exec;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let _ = tx.send(exec.answer("terminal/wait_for_exit", &by));
                });
                let limit = self.env.exec_seconds();
                let waited = if limit == 0 {
                    rx.recv().map_err(|_| ())
                } else {
                    rx.recv_timeout(core::time::Duration::from_secs(limit))
                        .map_err(|_| ())
                };
                match waited {
                    Ok(done) => done,
                    Err(()) => {
                        let _ = exec.answer("terminal/kill", &by);
                        let _ = rx.recv();
                        Err((
                            -32000,
                            format!(
                                "the program ran longer than {limit} seconds, the exec_seconds of environment {}, and was ended",
                                self.env.name()
                            ),
                        ))
                    }
                }
            })
        };
        let output = self.exec.answer("terminal/output", &by);
        let _ = self.exec.answer("terminal/release", &by);
        let exit = finished.map_err(|(_, why)| why)?;
        let output = output.map_err(|(_, why)| why)?;
        Ok(format!(
            "exit: {}\n{}",
            exit["exitCode"]
                .as_i64()
                .map(|code| code.to_string())
                .or_else(|| exit["signal"]
                    .as_i64()
                    .map(|signal| format!("signal {signal}")))
                .unwrap_or_else(|| "unknown".to_owned()),
            output["output"].as_str().unwrap_or_default()
        ))
    }

    fn open(&self, arguments: &Value) -> Result<String, String> {
        let info = self.env.needs(capability::TERMINAL)?;
        let shell = arguments
            .get("shell")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                info.capability(capability::TERMINAL)
                    .and_then(|each| each.facts.get("shell").cloned())
            })
            .unwrap_or_else(|| {
                if info.os == "windows" {
                    "cmd.exe"
                } else {
                    "/bin/sh"
                }
                .to_owned()
            });
        let mut spec = SpawnSpec::new(shell).terminal(TerminalSize::new(40, 120));
        if let Some(cwd) = arguments.get("cwd").and_then(Value::as_str) {
            spec = spec.cwd(cwd);
        }
        spec = spec.error_output(ErrorOutput::Merged);
        let spawner = self.env.terminal_spawner()?;
        let process = {
            Remote::new(
                spawner
                    .spawn(spec)
                    .map_err(|error| format!("environment {}: {error}", self.env.name()))?,
            )
        };
        let output = Arc::new(Mutex::new(super::served::Output::default()));
        let keep = self.env.scrollback_bytes();
        let (reader, kept) = (process.output(), Arc::clone(&output));
        let done = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("canoa-environment-shell".to_owned())
            .spawn(move || collect(reader, &kept, keep, &done))
            .map_err(|error| error.to_string())?;
        let id = format!("shell-{}", self.next.fetch_add(1, Ordering::SeqCst));
        lock(&self.shells).insert(id.clone(), Shell { process, output });
        Ok(id)
    }

    fn shell(
        &self,
        arguments: &Value,
    ) -> Result<(Remote, Arc<Mutex<super::served::Output>>), String> {
        let id = arguments
            .get("id")
            .and_then(Value::as_str)
            .ok_or("id is the text terminal_open answered")?;
        lock(&self.shells)
            .get(id)
            .map(|shell| (shell.process.clone(), Arc::clone(&shell.output)))
            .ok_or_else(|| "no such terminal".to_owned())
    }

    fn send(&self, arguments: &Value) -> Result<String, String> {
        let (process, _) = self.shell(arguments)?;
        let text = arguments
            .get("text")
            .and_then(Value::as_str)
            .ok_or("text is text")?;
        process
            .write(text.as_bytes())
            .map_err(|error| error.to_string())?;
        Ok("sent".to_owned())
    }

    fn read(&self, arguments: &Value) -> Result<String, String> {
        let (_, output) = self.shell(arguments)?;
        let wait = arguments
            .get("wait_ms")
            .and_then(Value::as_u64)
            .unwrap_or(500);
        std::thread::sleep(core::time::Duration::from_millis(wait.min(60_000)));
        let mut held = lock(&output);
        let taken = core::mem::take(&mut held.bytes);
        Ok(String::from_utf8_lossy(&taken).into_owned())
    }

    fn close(&self, arguments: &Value) -> Result<String, String> {
        let (process, _) = self.shell(arguments)?;
        let _ = process.kill();
        if let Some(id) = arguments.get("id").and_then(Value::as_str) {
            lock(&self.shells).remove(id);
        }
        Ok("closed".to_owned())
    }
}

/// The bearers in force, by the digest of the token: the token itself is never kept.
#[derive(Default)]
pub(crate) struct Tokens(Mutex<HashMap<[u8; 32], Arc<Access>>>);

impl Tokens {
    fn find(&self, digest: &[u8; 32]) -> Option<Arc<Access>> {
        lock(&self.0).get(digest).cloned()
    }
}

/// What the host holds for every connection: the environments, how a stdio agent reaches the server, the server itself, and the
/// person's protected places.
pub(crate) struct Hub {
    environments: Arc<Environments>,
    bridge: BridgeCommand,
    tokens: Arc<Tokens>,
    server: Mutex<Option<Listener>>,
    protected: Arc<Protected>,
}

struct ToolServer {
    hub: Weak<Hub>,
    /// `[host] max_message_bytes`: a call's arguments, a file's text among them, ride in one message.
    max_body: usize,
    tokens: Arc<Tokens>,
}

impl Service for ToolServer {
    type Caller = Access;
    const NAME: &'static str = "canoa-environments";
    const MAX_PER_CALLER: usize = 8;

    fn caller(&self, token: &str) -> Option<Arc<Access>> {
        self.tokens.find(&loopback::digest(token))
    }

    fn body_limit(&self) -> usize {
        self.max_body
    }

    fn answer(self: &Arc<Self>, access: &Arc<Access>, message: &Value) -> Answer {
        if self.hub.upgrade().is_none() {
            return Answer::Unavailable;
        }
        loopback::rpc(
            &server_name(access.env.name()),
            message,
            || access.list(),
            |name, arguments| Ok(access.call(name, arguments)),
        )
        .map_or(Answer::Accepted, Answer::Json)
    }
}

impl Hub {
    /// The hub for `environments`; `bridge` runs the stdio bridge for an agent that takes no HTTP MCP server, and `protected` is
    /// `[permissions] protected` (unset: the recommended list).
    pub(crate) fn new(
        environments: Arc<Environments>,
        bridge: BridgeCommand,
        protected: Option<&[String]>,
    ) -> Arc<Self> {
        let protected = protected.map_or_else(
            || Protected::new(crate::config::RECOMMENDED_PROTECTED.lines()),
            Protected::new,
        );
        Arc::new(Self {
            environments,
            bridge,
            tokens: Arc::new(Tokens::default()),
            server: Mutex::new(None),
            protected: Arc::new(protected),
        })
    }

    /// The host's own program and what goes before its subcommand.
    fn program(&self) -> (&str, &[String]) {
        let before = self
            .bridge
            .args
            .strip_suffix(&["mcp-bridge".to_owned()])
            .unwrap_or(&self.bridge.args);
        (&self.bridge.program, before)
    }

    /// What the tools of `env` hold to for a connection whose sessions' modes are `modes`.
    fn guard(&self, env: &Env, modes: &Arc<Modes>) -> Guard {
        Guard {
            list: Arc::clone(&self.protected),
            home: (env.name() == HERE).then(std::env::home_dir).flatten(),
            modes: Arc::clone(modes),
        }
    }

    /// The environments this host has.
    pub(crate) fn environments(&self) -> &Arc<Environments> {
        &self.environments
    }

    fn url(self: &Arc<Self>) -> Option<String> {
        let mut server = lock(&self.server);
        if server.is_none() {
            *server = Listener::start(&Arc::new(ToolServer {
                hub: Arc::downgrade(self),
                max_body: self.environments.message_bytes(),
                tokens: Arc::clone(&self.tokens),
            }))
            .ok();
        }
        server
            .as_ref()
            .map(|listener| format!("http://{}/mcp", listener.addr()))
    }

    /// The link of a connection that runs in `run`, and whose `placement` says what else it uses, or `None` when it uses nothing
    /// but where it runs and says nothing of it. `harness` is the agent's harness provider, when it has one.
    ///
    /// An agent that does its own file and shell work has its tools taken over when they are served from an environment that is
    /// not this computer, and is given that environment's tools; one whose vendor says no way to take them over is refused, since
    /// its tools would act on this computer.
    ///
    /// # Errors
    ///
    /// In words for the client: an environment it names is not there, or its agent's tools cannot be made to act there.
    pub(crate) fn for_connection(
        self: &Arc<Self>,
        run: Option<Arc<Env>>,
        placement: &Placement,
        harness: Option<&str>,
    ) -> Result<Option<Arc<Link>>, String> {
        if run.is_none() && placement.is_plain() {
            return Ok(None);
        }
        let own_tools = harness.is_some_and(crate::bundled::does_own_tool_work);
        let served = placement
            .serve
            .as_deref()
            .map(|name| self.environments.get(name))
            .transpose()?
            .map(Served::new);
        let mut tools = Vec::new();
        for name in &placement.tools {
            let env = self.environments.get(name)?;
            if !tools
                .iter()
                .any(|each: &Arc<Env>| each.name() == env.name())
            {
                tools.push(env);
            }
        }
        let modes = Arc::new(Modes::default());
        // An agent that runs where its tools are served from already acts there.
        let runs_there = |served: &Served| {
            served.env().name() == HERE
                || run
                    .as_ref()
                    .is_some_and(|run| run.name() == served.env().name())
        };
        let interception = match (&served, harness) {
            (Some(served), Some(harness)) if own_tools && !runs_there(served) => {
                let env = Arc::clone(served.env());
                if let Some(run) = &run {
                    // The hook runs this host's program, which is not where the agent runs.
                    return Err(format!(
                        "{harness} runs in environment {} and does its own file and shell work there, so its tools cannot also be served from environment {}",
                        run.name(),
                        env.name()
                    ));
                }
                let hook = crate::vendors::behave(harness, |vendor| vendor.tool_hook())
                    .and_then(Result::ok)
                    .flatten()
                    .ok_or_else(|| {
                        format!(
                            "{harness} does its own file and shell work and its tools cannot be made to act in environment {}: run it there (\"run\") or offer the environment as tools (\"tools\")",
                            env.name()
                        )
                    })?;
                let (program, before) = self.program();
                let interception = Interception::open(
                    Arc::clone(&env),
                    (harness, hook),
                    (program, before),
                    Arc::clone(&modes),
                    self.environments.message_bytes(),
                )
                .map_err(|error| {
                    format!(
                        "the tools of environment {} cannot be given to this agent: {error}",
                        env.name()
                    )
                })?;
                // What the hook turns the agent's file tools to.
                if !tools.iter().any(|each| each.name() == env.name()) {
                    tools.push(env);
                }
                Some(interception)
            }
            _ => None,
        };
        Ok(Some(Arc::new(Link {
            hub: Arc::clone(self),
            run,
            served,
            tools,
            own_tools,
            interception,
            modes,
            plugins_are_hosts: AtomicBool::new(false),
            state: Mutex::new(LinkState::default()),
            queue: Mutex::new(VecDeque::new()),
            held: Mutex::new(None),
            closed: AtomicBool::new(false),
        })))
    }
}

enum Pending {
    Initialize,
    Open { digests: Vec<[u8; 32]> },
}

/// A request of the client's whose answer says something of a session.
enum Asked {
    /// `session/new`, `load` or `resume`: the session it names, none for a new one.
    Session { session: Option<String> },
    /// The client chose a mode for a session.
    Mode { session: String, mode: Option<Mode> },
}

#[derive(Default)]
struct LinkState {
    /// The agent takes MCP servers over HTTP.
    http: bool,
    /// The ACP version the agent answered `initialize` in when it takes stdio servers, else 0.
    stdio: u64,
    pending: HashMap<String, Pending>,
    asked: HashMap<String, Asked>,
    digests: Vec<[u8; 32]>,
    /// The bearers of each session, once the agent has opened it.
    bearers: HashMap<String, Vec<[u8; 32]>>,
    /// The configuration option that carries each session's mode, when the agent lists one.
    mode_options: HashMap<String, String>,
    /// How many session requests of this connection were given the tool servers.
    opened: usize,
    /// The sessions the agent has been told about its environments.
    briefed: BTreeSet<String>,
}

/// One client connection's use of environments.
pub(crate) struct Link {
    hub: Arc<Hub>,
    run: Option<Arc<Env>>,
    served: Option<Served>,
    tools: Vec<Arc<Env>>,
    /// The agent does its own file and shell work, whatever its client serves.
    own_tools: bool,
    /// The take-over of that work, when its tools are served from an environment.
    interception: Option<Interception>,
    /// The mode of each session, for the take-over and for the protected places of the tools.
    modes: Arc<Modes>,
    /// The connection has no profile, so every plugin a session request carries is the host's: one the client put there is taken
    /// out, since a plugin's hooks run without asking.
    plugins_are_hosts: AtomicBool,
    state: Mutex<LinkState>,
    /// What the host answers the agent: its calls served from an environment.
    queue: Mutex<VecDeque<Vec<u8>>>,
    /// A prompt that waits for its session's hook to report.
    held: Mutex<Option<Held>>,
    closed: AtomicBool,
}

/// What becomes of a message from the client.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Said {
    /// It goes on to the agent, as it now is.
    On(Vec<u8>),
    /// The host answered it: this goes back to the client, and nothing to the agent.
    Answered(Vec<u8>),
    /// It waits for its session's hook to report that it is in force ([`Link::release`]).
    Held,
}

/// A prompt waiting for its session's hook.
struct Held {
    message: Vec<u8>,
    session: String,
    id: Option<Value>,
    until: Instant,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether `meta` holds everything `gained` adds: each list's entries, at the same place.
fn holds(meta: &Value, gained: &Value) -> bool {
    match (meta, gained) {
        (Value::Object(meta), Value::Object(gained)) => gained
            .iter()
            .all(|(key, value)| meta.get(key).is_some_and(|held| holds(held, value))),
        (Value::Array(meta), Value::Array(gained)) => gained.iter().all(|each| meta.contains(each)),
        (held, gained) => held == gained,
    }
}

/// Takes out of `meta` every list that `gained` holds at the same place: what the client put where the host's plugins go.
fn without_lists_of(meta: &mut Value, gained: &Value) {
    let (Value::Object(meta), Value::Object(gained)) = (meta, gained) else {
        return;
    };
    for (key, value) in gained {
        match value {
            Value::Array(_) => {
                meta.remove(key);
            }
            Value::Object(_) => {
                if let Some(held) = meta.get_mut(key) {
                    without_lists_of(held, value);
                }
            }
            _ => {}
        }
    }
}

impl Link {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Ends every bearer and terminal of the connection.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        {
            let mut state = lock(&self.state);
            for digest in state.digests.drain(..) {
                lock(&self.hub.tokens.0).remove(&digest);
            }
        }
        if let Some(served) = &self.served {
            served.end_all();
        }
    }

    /// Whether the agent's sessions are given a plugin of the host's, which its harness must be started to take.
    pub(crate) fn gives_plugins(&self) -> bool {
        self.interception.is_some()
    }

    /// The connection has no profile: a plugin in a session request is the host's only.
    pub(crate) fn plugins_are_hosts(&self) {
        self.plugins_are_hosts.store(true, Ordering::SeqCst);
    }

    /// The oldest answer the host has for the agent, for the stream the agent reads.
    pub(crate) fn next_answer(&self) -> Option<Vec<u8>> {
        lock(&self.queue).pop_front()
    }

    fn remember(&self, id: String, pending: Pending) {
        let mut state = lock(&self.state);
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(id, pending);
        }
    }

    /// The environments the agent has and how each is attached, for what it is told.
    fn attached(&self) -> Vec<(Arc<Env>, Vec<String>)> {
        let mut all: Vec<(Arc<Env>, Vec<String>)> = Vec::new();
        let mut add = |env: &Arc<Env>, way: String| match all
            .iter_mut()
            .find(|(each, _)| each.name() == env.name())
        {
            Some((_, ways)) => ways.push(way),
            None => all.push((Arc::clone(env), vec![way])),
        };
        if let Some(env) = &self.run {
            add(
                env,
                "the agent's program runs here, so its own tools (shell, files) act here"
                    .to_owned(),
            );
        }
        if let Some(served) = &self.served {
            let way = if self.interception.is_some() {
                format!(
                    "the agent's own tools are turned here: each command it runs runs here, starting in this environment's own working folder (so name paths in full or begin with a cd), and its file tools are refused with the way to do the same with the MCP server {}",
                    server_name(served.env().name())
                )
            } else if self.own_tools {
                "the agent's standard file and terminal tools are served from here, and its own work acts on this computer, where this environment is".to_owned()
            } else {
                "the agent's standard file and terminal tools are served from here: files it reads and writes and commands it runs through its client act here".to_owned()
            };
            add(served.env(), way);
        }
        for env in &self.tools {
            add(
                env,
                format!(
                    "offered as tools: the MCP server {} has a tool for each capability below",
                    server_name(env.name())
                ),
            );
        }
        all
    }

    /// What the agent is told of its environments.
    pub(crate) fn briefing(&self) -> String {
        briefing(&self.attached())
    }

    /// A message from the client, after the scope has checked it: `initialize` says what the host serves, a session request gets
    /// the environments' servers and the plugin that takes the agent's own tools over, a mode the client chooses is noted, and the
    /// first prompt of a session is told what environments it has.
    pub(crate) fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Said {
        // JSON may escape any character, so a message with a backslash is read whatever its bytes say.
        if !(contains(&message, b"session/")
            || contains(&message, b"initialize")
            || message.contains(&b'\\'))
        {
            return Said::On(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return Said::On(message);
        };
        let Some(method) = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Said::On(message);
        };
        let id = value.get("id").map(Value::to_string);
        let session = value
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let changed = match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                self.remember(id, Pending::Initialize);
                self.advertise(&mut value)
            }
            ("session/new" | "session/load" | "session/resume", Some(id)) => {
                let taken = match self.take_over(&mut value, &id) {
                    Ok(taken) => taken,
                    Err(why) => {
                        let answer = json!({"jsonrpc": "2.0", "id": value.get("id"),
                            "error": {"code": crate::control::BAD_PARAMS, "message": why}});
                        return Said::Answered(answer.to_string().into_bytes());
                    }
                };
                lock(&self.state)
                    .asked
                    .insert(id.clone(), Asked::Session { session });
                let opened = self.open(&mut value, id);
                taken || opened
            }
            ("session/set_mode", Some(id)) => {
                if let Some(session) = session {
                    let mode = value
                        .pointer("/params/modeId")
                        .and_then(Value::as_str)
                        .and_then(Mode::of_id);
                    lock(&self.state)
                        .asked
                        .insert(id, Asked::Mode { session, mode });
                }
                false
            }
            ("session/set_config_option", Some(id)) => {
                if let Some(session) = session {
                    let mut state = lock(&self.state);
                    let option = value.pointer("/params/configId").and_then(Value::as_str);
                    if option.is_some()
                        && state.mode_options.get(&session).map(String::as_str) == option
                    {
                        let mode = value
                            .pointer("/params/value")
                            .and_then(Value::as_str)
                            .and_then(Mode::of_id);
                        state.asked.insert(id, Asked::Mode { session, mode });
                    }
                }
                false
            }
            ("session/close", _) => {
                if let Some(session) = session {
                    self.end(&session);
                }
                false
            }
            ("session/prompt", _) => {
                let briefed = self.brief(&mut value);
                if let (Some(interception), Some(session)) = (&self.interception, session)
                    && !interception.started(&session)
                {
                    let message = if briefed {
                        serde_json::to_vec(&value).unwrap_or(message)
                    } else {
                        message
                    };
                    *lock(&self.held) = Some(Held {
                        message,
                        session,
                        id: value.get("id").cloned(),
                        until: Instant::now() + Duration::from_secs(interception.seconds()),
                    });
                    return Said::Held;
                }
                briefed
            }
            _ => false,
        };
        if changed {
            Said::On(serde_json::to_vec(&value).unwrap_or(message))
        } else {
            Said::On(message)
        }
    }

    /// Whether a prompt waits for its session's hook.
    pub(crate) fn holding(&self) -> bool {
        lock(&self.held).is_some()
    }

    /// The prompt that waited, once its session's hook has reported (it goes on), or once the wait is over (it is refused, since
    /// the session's commands would run on this computer); `None` while it still waits, or when none does.
    pub(crate) fn release(&self) -> Option<Said> {
        let mut held = lock(&self.held);
        let waiting = held.as_ref()?;
        let started = self
            .interception
            .as_ref()
            .is_some_and(|interception| interception.started(&waiting.session));
        if started {
            return held.take().map(|held| Said::On(held.message));
        }
        if Instant::now() < waiting.until {
            return None;
        }
        let held = held.take()?;
        let why = format!(
            "the hook that runs this session's tools in environment {} did not report that it is in force, so the prompt is not sent: the agent's commands would run on this computer (Claude Code's hooks may be turned off by disableAllHooks or a managed policy)",
            self.served
                .as_ref()
                .map_or("", |served| served.env().name())
        );
        let answer = json!({"jsonrpc": "2.0", "id": held.id,
            "error": {"code": crate::control::BAD_PARAMS, "message": why}});
        Some(Said::Answered(answer.to_string().into_bytes()))
    }

    /// Gives a session request the plugin that takes the agent's own tools over, when they are; whether the request changed.
    ///
    /// # Errors
    ///
    /// Why the plugin cannot be given: the session would run the agent's tools on this computer, so the request is refused.
    fn take_over(&self, value: &mut Value, id: &str) -> Result<bool, String> {
        let Some(interception) = &self.interception else {
            return Ok(false);
        };
        if self.is_closed() {
            return Err("the connection is closing".to_owned());
        }
        let gained = interception.plugin_for(id)?;
        let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
            interception.answered(id, None);
            return Err("a session request with no parameters".to_owned());
        };
        let meta = params.entry("_meta").or_insert_with(|| json!({}));
        if !meta.is_object() {
            *meta = json!({});
        }
        if self.plugins_are_hosts.load(Ordering::SeqCst) {
            without_lists_of(meta, &gained);
        }
        crate::profile::hosted::merge_json(meta, &gained);
        // A merge keeps a value of the client's where the host's object or list would go, and the agent would then load no
        // plugin: the request is refused rather than opened without the hook.
        if !holds(meta, &gained) {
            interception.answered(id, None);
            return Err(
                "the request's _meta holds a value where the agent reads the host's plugin, so the hook that runs this session's tools in its environment cannot be given".to_owned(),
            );
        }
        Ok(true)
    }

    /// The session ended: its plugin, the commands it prepared and its bearers end.
    fn end(&self, session: &str) {
        if let Some(interception) = &self.interception {
            interception.closed(session);
        }
        self.modes.forget(session);
        let mut state = lock(&self.state);
        state.mode_options.remove(session);
        if let Some(digests) = state.bearers.remove(session) {
            for digest in &digests {
                lock(&self.hub.tokens.0).remove(digest);
            }
            state.digests.retain(|held| !digests.contains(held));
        }
    }

    /// Says in `clientCapabilities` what the environment served offers, and nothing the client's own claims add: the host answers.
    fn advertise(&self, value: &mut Value) -> bool {
        let Some(served) = &self.served else {
            return false;
        };
        let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
            return false;
        };
        let capabilities = params
            .entry("clientCapabilities")
            .or_insert_with(|| json!({}));
        let Some(capabilities) = capabilities.as_object_mut() else {
            return false;
        };
        let offered = served.client_capabilities();
        capabilities.insert("fs".to_owned(), offered["fs"].clone());
        capabilities.insert("terminal".to_owned(), offered["terminal"].clone());
        true
    }

    /// Adds a server and its bearer for each environment offered as tools.
    fn open(self: &Arc<Self>, value: &mut Value, id: String) -> bool {
        if self.tools.is_empty() || self.is_closed() {
            return false;
        }
        let (http, stdio) = {
            let state = lock(&self.state);
            (state.http, state.stdio)
        };
        if !(http || stdio > 0) {
            return false;
        }
        let opened = lock(&self.state).opened;
        if let Some(env) = self
            .tools
            .iter()
            .find(|env| opened >= env.max_tool_sessions())
        {
            tracing::warn!(
                "environment {} is not given to another session of this connection: max_tool_sessions is {}",
                env.name(),
                env.max_tool_sessions()
            );
            return false;
        }
        lock(&self.state).opened += 1;
        let Some(url) = self.hub.url() else {
            return false;
        };
        let mut entries: Vec<Value> = Vec::new();
        let mut digests: Vec<[u8; 32]> = Vec::new();
        for env in &self.tools {
            let mut secret = [0_u8; 32];
            if getrandom::fill(&mut secret).is_err() {
                return false;
            }
            let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
            let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            let name = server_name(env.name());
            let entry = if http {
                json!({
                    "type": "http",
                    "name": name,
                    "url": url,
                    "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}],
                })
            } else {
                let mut entry = json!({
                    "name": name,
                    "command": self.hub.bridge.program,
                    "args": self.hub.bridge.args,
                    "env": [
                        {"name": crate::delegate::bridge::URL_VAR, "value": url},
                        {"name": crate::delegate::bridge::TOKEN_VAR, "value": token},
                    ],
                });
                if stdio >= 2 {
                    entry["type"] = json!("stdio");
                }
                entry
            };
            let access = Access::guarded(Arc::clone(env), self.hub.guard(env, &self.modes));
            lock(&self.hub.tokens.0).insert(digest, Arc::new(access));
            digests.push(digest);
            entries.push(entry);
        }
        let Some(servers) = value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .map(|params| params.entry("mcpServers").or_insert_with(|| json!([])))
            .and_then(Value::as_array_mut)
        else {
            for digest in &digests {
                lock(&self.hub.tokens.0).remove(digest);
            }
            return false;
        };
        servers.retain(|server| {
            server
                .get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| !name.starts_with(SERVER_PREFIX))
        });
        servers.extend(entries);
        lock(&self.state).digests.extend(digests.iter().copied());
        self.remember(id, Pending::Open { digests });
        true
    }

    /// Puts what the agent is to know about its environments in front of the first prompt of each session.
    fn brief(&self, value: &mut Value) -> bool {
        let Some(session) = value
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return false;
        };
        if !lock(&self.state).briefed.insert(session) {
            return false;
        }
        let text = self.briefing();
        let Some(prompt) = value
            .pointer_mut("/params/prompt")
            .and_then(Value::as_array_mut)
        else {
            return false;
        };
        prompt.insert(0, json!({"type": "text", "text": text}));
        true
    }

    /// A message from the agent, before it goes to the client. Answers a call the host serves and says so by returning `true`:
    /// the message then goes no further.
    pub(crate) fn agent_said(self: &Arc<Self>, message: &[u8]) -> bool {
        let marked =
            contains(message, b"current_mode_update") || contains(message, b"config_option_update");
        if !contains(message, b"id") && !marked {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return false;
        };
        if marked && value.get("method").and_then(Value::as_str) == Some("session/update") {
            self.mode_notice(&value);
        }
        let Some(id) = value.get("id").cloned() else {
            return false;
        };
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            if self.served.is_none() || !super::served::serves(method) {
                return false;
            }
            let (method, params) = (
                method.to_owned(),
                value.get("params").cloned().unwrap_or(Value::Null),
            );
            let link = Arc::clone(self);
            // A call may take long (a terminal's wait), and the agent may make others meanwhile: each has its own thread.
            let spawned = std::thread::Builder::new()
                .name("canoa-environment-call".to_owned())
                .spawn(move || {
                    let Some(served) = &link.served else { return };
                    let answer = match served.answer(&method, &params) {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        Err((code, why)) => json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": code, "message": why}}),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&answer) {
                        lock(&link.queue).push_back(bytes);
                    }
                });
            return spawned.is_ok();
        }
        self.answered(&id.to_string(), &value);
        false
    }

    /// What the agent says of a session's mode, which can only make it stricter.
    fn mode_notice(&self, value: &Value) {
        let Some(session) = value.pointer("/params/sessionId").and_then(Value::as_str) else {
            return;
        };
        let update = &value["params"]["update"];
        let reported = match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("current_mode_update") => update
                .get("currentModeId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Some("config_option_update") => crate::delegate::privilege::modes_of(update).current,
            _ => None,
        };
        if let Some(mode) = reported {
            self.modes
                .reported(session, Mode::of_id(&mode).unwrap_or(Mode::Manual));
        }
    }

    /// The agent's answer to a request of the client's the host remembered.
    fn answered(&self, id: &str, value: &Value) {
        let (pending, asked) = {
            let mut state = lock(&self.state);
            (state.pending.remove(id), state.asked.remove(id))
        };
        let ok = value.get("result").filter(|_| value.get("error").is_none());
        let opened = match (&asked, ok) {
            (Some(Asked::Session { session }), Some(result)) => result
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| session.clone()),
            _ => None,
        };
        if let Some(interception) = &self.interception
            && matches!(asked, Some(Asked::Session { .. }))
        {
            interception.answered(id, opened.as_deref());
        }
        if let (Some(session), Some(result)) = (&opened, ok) {
            let modes = crate::delegate::privilege::modes_of(result);
            if let Some(option) = modes.option {
                lock(&self.state)
                    .mode_options
                    .insert(session.clone(), option);
            }
            // What the agent says its session starts in can only make it stricter.
            if let Some(current) = modes.current {
                self.modes
                    .reported(session, Mode::of_id(&current).unwrap_or(Mode::Manual));
            }
        }
        if let (Some(Asked::Mode { session, mode }), Some(_)) = (&asked, ok) {
            self.modes.chosen(session, mode.unwrap_or(Mode::Manual));
        }
        match (pending, ok) {
            (Some(Pending::Initialize), Some(result)) => {
                let takes = |capability: Option<&Value>| {
                    capability.is_some_and(|said| said == &json!(true) || said.is_object())
                };
                let version = result
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let mut state = lock(&self.state);
                state.http = takes(
                    result
                        .pointer("/agentCapabilities/mcpCapabilities/http")
                        .or_else(|| result.pointer("/capabilities/session/mcp/http")),
                );
                let stdio = version < 2 || takes(result.pointer("/capabilities/session/mcp/stdio"));
                state.stdio = if stdio { version } else { 0 };
            }
            (Some(Pending::Open { digests }), Some(_)) => match &opened {
                Some(session) => {
                    for digest in &digests {
                        if let Some(access) = self.hub.tokens.find(digest) {
                            access.opened(session);
                        }
                    }
                    lock(&self.state)
                        .bearers
                        .entry(session.clone())
                        .or_default()
                        .extend(digests);
                }
                None => self.forget_bearers(&digests),
            },
            // A session the agent did not open has no use for its bearers.
            (Some(Pending::Open { digests }), None) => self.forget_bearers(&digests),
            _ => {}
        }
    }

    fn forget_bearers(&self, digests: &[[u8; 32]]) {
        for digest in digests {
            lock(&self.hub.tokens.0).remove(digest);
        }
        lock(&self.state)
            .digests
            .retain(|held| !digests.contains(held));
    }

    /// The take-over of the agent's tools, for tests.
    #[cfg(test)]
    pub(crate) fn interception_for_tests(&self) -> Option<&Interception> {
        self.interception.as_ref()
    }

    /// The mode the host holds for `session`, for tests.
    #[cfg(test)]
    pub(crate) fn mode_of(&self, session: &str) -> Mode {
        self.modes.of(session)
    }
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link")
            .field("run", &self.run.as_ref().map(|env| env.name().to_owned()))
            .field(
                "served",
                &self
                    .served
                    .as_ref()
                    .map(|served| served.env().name().to_owned()),
            )
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|env| env.name().to_owned())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}
