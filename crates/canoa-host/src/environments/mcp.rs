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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use canoa::environment::{State, capability};
use coqui_acp_api::{ErrorOutput, SpawnSpec, TerminalSize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::remote::Remote;
use super::served::{Served, collect};
use super::{Env, Environments, Placement, briefing, lock};
use crate::delegate::BridgeCommand;
use crate::loopback::{self, Answer, Caller, Listener, Service};

/// The prefix of an environment's MCP server's name.
pub(crate) const SERVER_PREFIX: &str = "env_";
/// Sessions of one connection that hold a bearer.
const MAX_SESSIONS: usize = 256;
/// Requests of the client the host remembers an answer for.
const MAX_PENDING: usize = 64;
/// How much of an interactive terminal's output is kept for the agent to read.
const TERMINAL_KEEP_BYTES: usize = 4 * 1024 * 1024;

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

/// What one bearer opens: the tools of one environment, for one connection.
pub(crate) struct Access {
    env: Arc<Env>,
    exec: Served,
    shells: Mutex<BTreeMap<String, Shell>>,
    next: AtomicU64,
    calls: AtomicUsize,
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
    pub(super) fn new(env: Arc<Env>) -> Self {
        Self {
            exec: Served::new(Arc::clone(&env)),
            env,
            shells: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            calls: AtomicUsize::new(0),
        }
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
            "fs_read" => self
                .exec
                .answer("fs/read_text_file", arguments)
                .map(|answer| answer["content"].as_str().unwrap_or_default().to_owned())
                .map_err(|(_, why)| why),
            "fs_write" => self
                .exec
                .answer("fs/write_text_file", arguments)
                .map(|_| "written".to_owned())
                .map_err(|(_, why)| why),
            "fs_list" => self.list_folder(arguments),
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
        let (reader, kept) = (process.output(), Arc::clone(&output));
        let done = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("canoa-environment-shell".to_owned())
            .spawn(move || collect(reader, &kept, TERMINAL_KEEP_BYTES, &done))
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

/// What the host holds for every connection: the environments, how a stdio agent reaches the server, and the server itself.
pub(crate) struct Hub {
    environments: Arc<Environments>,
    bridge: BridgeCommand,
    tokens: Arc<Tokens>,
    server: Mutex<Option<Listener>>,
}

struct ToolServer {
    hub: Weak<Hub>,
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
        MAX_BODY
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

/// A call's arguments: a file's text rides in one.
const MAX_BODY: usize = 16 * 1024 * 1024;

impl Hub {
    /// The hub for `environments`; `bridge` runs the stdio bridge for an agent that takes no HTTP MCP server.
    pub(crate) fn new(environments: Arc<Environments>, bridge: BridgeCommand) -> Arc<Self> {
        Arc::new(Self {
            environments,
            bridge,
            tokens: Arc::new(Tokens::default()),
            server: Mutex::new(None),
        })
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
                tokens: Arc::clone(&self.tokens),
            }))
            .ok();
        }
        server
            .as_ref()
            .map(|listener| format!("http://{}/mcp", listener.addr()))
    }

    /// The link of a connection that runs in `run`, and whose `placement` says what else it uses, or `None` when it uses nothing
    /// but where it runs and says nothing of it.
    ///
    /// # Errors
    ///
    /// In words for the client: an environment it names is not there.
    pub(crate) fn for_connection(
        self: &Arc<Self>,
        run: Option<Arc<Env>>,
        placement: &Placement,
        own_tools: bool,
    ) -> Result<Option<Arc<Link>>, String> {
        if run.is_none() && placement.is_plain() {
            return Ok(None);
        }
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
        Ok(Some(Arc::new(Link {
            hub: Arc::clone(self),
            run,
            served,
            tools,
            own_tools,
            state: Mutex::new(LinkState::default()),
            queue: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
        })))
    }
}

enum Pending {
    Initialize,
    Open { digests: Vec<[u8; 32]> },
}

#[derive(Default)]
struct LinkState {
    /// The agent takes MCP servers over HTTP.
    http: bool,
    /// The ACP version the agent answered `initialize` in when it takes stdio servers, else 0.
    stdio: u64,
    pending: HashMap<String, Pending>,
    digests: Vec<[u8; 32]>,
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
    state: Mutex<LinkState>,
    /// What the host answers the agent: its calls served from an environment.
    queue: Mutex<VecDeque<Vec<u8>>>,
    closed: AtomicBool,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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
            let mut way = "the agent's standard file and terminal tools are served from here: files it reads and writes and commands it runs through its client act here".to_owned();
            if self.own_tools {
                way.push_str(
                    ". This agent does its own file and shell work and never asks its client for it, so nothing it does here is served from this environment: use this environment's tools (an MCP server named env_<name>) or run the agent there",
                );
            }
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
    /// the environments' servers, and the first prompt of a session is told what environments it has.
    pub(crate) fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Vec<u8> {
        // JSON may escape any character, so a message with a backslash is read whatever its bytes say.
        if !(contains(&message, b"session/")
            || contains(&message, b"initialize")
            || message.contains(&b'\\'))
        {
            return message;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return message;
        };
        let Some(method) = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return message;
        };
        let id = value.get("id").map(Value::to_string);
        let changed = match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                self.remember(id, Pending::Initialize);
                self.advertise(&mut value)
            }
            ("session/new" | "session/load" | "session/resume", Some(id)) => {
                self.open(&mut value, id)
            }
            ("session/prompt", _) => self.brief(&mut value),
            _ => false,
        };
        if changed {
            serde_json::to_vec(&value).unwrap_or(message)
        } else {
            message
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
        let (http, stdio, count) = {
            let state = lock(&self.state);
            (state.http, state.stdio, state.digests.len())
        };
        if count + self.tools.len() > MAX_SESSIONS || !(http || stdio > 0) {
            return false;
        }
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
            lock(&self.hub.tokens.0).insert(digest, Arc::new(Access::new(Arc::clone(env))));
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
        if !contains(message, b"id") {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return false;
        };
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

    /// The agent's answer to a request of the client's the host remembered.
    fn answered(&self, id: &str, value: &Value) {
        let pending = lock(&self.state).pending.remove(id);
        let ok = value.get("result").filter(|_| value.get("error").is_none());
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
            // A session the agent did not open has no use for its bearers.
            (Some(Pending::Open { digests }), None) => {
                for digest in &digests {
                    lock(&self.hub.tokens.0).remove(digest);
                }
                lock(&self.state)
                    .digests
                    .retain(|held| !digests.contains(held));
            }
            _ => {}
        }
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
