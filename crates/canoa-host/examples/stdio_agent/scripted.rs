//! The scripted mode of `stdio_agent` (`--scripted`): a stand-in for a coding agent speaking ACP v1, so the app's tests can run
//! a whole task without spending a subscription. Each prompt gets one fixed answer: tools asked about, a plan, a stop it
//! obeys. What it changes on disk is not the point; what reaches the screen is. Its conversations live in the file `--state=`
//! names.
//!
//! A prompt starting `Say ` is said back; one starting `Ask ` asks a question as a form, when the client takes them, shaped as
//! Claude's adapter asks one; `Rename` is stopped by a cancel; a prompt with an attachment follows it; one saying `fails`
//! also sends v2's plan_update, tool_call_content_chunk, notice and compaction updates. It offers a mode, a model and an
//! effort as config options with their ACP categories, refuses the model `refuse-me` with -32602, and notes each session
//! request and prompt in order in its state, under the session's `calls`. With `--usage` it also speaks the usage extension: a
//! prompt `quota N` sets its five-hour window to N thousandths used (exhausted at 1000) and tells a client that asked;
//! `--usage-claims=ID` makes it report under that account instead of the one it was given. `--name=` and `--version=` are what
//! it says of itself, and `--no-reopen` leaves out session/load and session/list. A prompt `fail TEXT` is answered with a
//! JSON-RPC error whose message is TEXT, `fail-data TEXT` with a plain message and TEXT in the error's data, and `refuse TEXT`
//! says TEXT and ends the turn `refusal`: how a limit reaches a client from an agent that reports no usage. `--system-prompt`
//! says it takes a system prompt with a new session and notes the one it is given, as the session's `system`.
//!
//! Delegation: `--mcp-http` advertises HTTP MCP servers, and `--modes=a,b,c` lists the modes of its mode option (the first is the
//! default). A session given the host's `canoa-delegate` entry can call its tools: `Delegate SOURCE TEXT`, `DelegateIn FOLDER
//! SOURCE TEXT`, `DelegateAwait SOURCE TEXT` (and waits for the task to end), `Status ID [WAIT_MS]`, `Cancel ID` and `Call TOOL JSON` say what the tool answered. `Mode` says the mode the
//! session is in, `Verbatim` the blocks and text of its own prompt, `Wait` waits for a cancel, `PidFile PATH` writes its process id
//! to PATH and waits for a cancel, `Bearer` says the tool server's address and the session's bearer (a stand-in may; the host never
//! does), `Raw LINE` writes LINE to the client as it is, `SecretBearer` and `SecretBridge` say what `Bearer` and `Bridge` say for the secret tools, `Secret TOOL JSON` calls a tool of the host's secret server (`secret_request`, `secret_use`, `secret_show`) and says what it
//! answered, `Pid` says its process id and `Cwd` the working directory the session was opened in. Given the entry as a stdio server
//! (what the host writes for an agent that says no HTTP), it starts that command and calls the tools through it as an MCP client
//! does; `Bridge` says its process id and the command line it was started with, and `BridgeClose` closes its input and says
//! whether it ended. `--marker=PATH` appends the process id
//! to PATH each time the agent starts, so a test can tell whether one did.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// How the person answered a permission request.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Allowed,
    Refused,
    Cancelled,
}

/// Each session's address and bearer.
type Tools = Mutex<HashMap<String, (String, String)>>;
/// Each session's started stdio server.
type Bridges = Mutex<HashMap<String, Arc<Mutex<Bridge>>>>;

struct Shared {
    state_path: Option<PathBuf>,
    /// The state when no file is named, and the lock that makes a read-change-write one step.
    state: Mutex<Value>,
    agent_info: Option<Value>,
    reopens: bool,
    /// Whether it says it takes a system prompt.
    system_prompt: bool,
    waiting: Mutex<HashMap<u64, Sender<Value>>>,
    next_id: Mutex<u64>,
    /// "<session> <kind>" of a tool the person said to always allow or reject.
    remembered: Mutex<HashMap<String, Answer>>,
    /// What each session has chosen: option id to value.
    chosen: Mutex<HashMap<String, Map<String, Value>>>,
    forms: Mutex<bool>,
    /// The id of the prompt being answered.
    turn: Mutex<Option<Value>>,
    /// The usage extension, when this agent speaks it.
    usage: Option<Usage>,
    /// The modes its mode option lists.
    modes: Vec<String>,
    /// Whether it takes MCP servers over HTTP.
    http: bool,
    /// Whether it reports a profile of its own, in Bypass, in the answer to `session/new` (`--forge-profile`).
    forge: bool,
    /// Each session's address and bearer for the host's delegation tools.
    tools: Mutex<HashMap<String, (String, String)>>,
    /// The stdio server each session was given for them, started.
    bridges: Mutex<HashMap<String, Arc<Mutex<Bridge>>>>,
    /// The same for the secret tools.
    secret_bridges: Mutex<HashMap<String, Arc<Mutex<Bridge>>>>,
    /// Each session's address and bearer for the host's secret tools.
    secret_tools: Mutex<HashMap<String, (String, String)>>,
    /// The working directory each session was opened in.
    cwds: Mutex<HashMap<String, String>>,
}

/// The host's tool server as a stdio MCP server this agent started: a child process and the lines to and from it.
struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    lines: BufReader<ChildStdout>,
    next: u64,
    started: bool,
    /// The command line it was started with.
    command: String,
}

impl Bridge {
    fn start(server: &Value) -> Option<(Self, String, String)> {
        let command = server["command"].as_str()?;
        let args: Vec<String> = server["args"]
            .as_array()?
            .iter()
            .filter_map(|arg| arg.as_str().map(str::to_owned))
            .collect();
        let mut process = Command::new(command);
        process
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let (mut url, mut token) = (String::new(), String::new());
        for pair in server["env"].as_array().into_iter().flatten() {
            let (Some(name), Some(value)) = (pair["name"].as_str(), pair["value"].as_str()) else {
                continue;
            };
            match name {
                "CANOA_DELEGATE_URL" => value.clone_into(&mut url),
                "CANOA_DELEGATE_TOKEN" => value.clone_into(&mut token),
                _ => {}
            }
            process.env(name, value);
        }
        let mut child = process.spawn().ok()?;
        let input = child.stdin.take();
        let lines = BufReader::new(child.stdout.take()?);
        let shown = format!("{command} {}", args.join(" "));
        let bridge = Self {
            child,
            input,
            lines,
            next: 0,
            started: false,
            command: shown,
        };
        Some((bridge, url, token))
    }

    fn write(&mut self, message: &Value) -> std::io::Result<()> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the bridge's input is closed"))?;
        writeln!(input, "{message}")?;
        input.flush()
    }

    /// One request, and the answer with its id.
    fn request(&mut self, method: &str, params: &Value) -> std::io::Result<Value> {
        self.next += 1;
        let id = self.next;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let mut line = String::new();
            if self.lines.read_line(&mut line)? == 0 {
                return Err(std::io::Error::other("the bridge ended"));
            }
            let message: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
            if message["id"] == id {
                return Ok(message);
            }
        }
    }

    /// The MCP handshake, once.
    fn open(&mut self) -> std::io::Result<()> {
        if self.started {
            return Ok(());
        }
        self.request(
            "initialize",
            &json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "stand-in", "version": "0"}}),
        )?;
        self.write(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        self.started = true;
        Ok(())
    }
}

/// What the usage extension keeps: the client's opt-in and the one window it reports.
struct Usage {
    claims: Option<String>,
    state: Mutex<UsageState>,
}

#[derive(Default)]
struct UsageState {
    notify: bool,
    account: Option<String>,
    revision: u64,
    used: Option<u64>,
    exhausted: bool,
}

const USAGE_CAPABILITY: &str = "dev.boriken.coqui.acp/usage";

impl Usage {
    /// The whole report, as the extension spells it.
    fn report(&self) -> Value {
        let state = lock(&self.state);
        let account = self.claims.clone().or_else(|| state.account.clone());
        let mut report = json!({"supported": true, "sources": ["reported"], "revision": state.revision,
            "observed_at": 1_788_900_000, "buckets": []});
        if let Some(account) = account {
            report["account"] = json!(account);
        }
        if let Some(used) = state.used {
            report["buckets"] = json!([{"id": "five_hour", "label": "5 hours", "models": {"kind": "all"},
                "unit": "permille", "used": used, "limit": 1000,
                "refill": {"kind": "fixed_window", "length": [18000, 0], "resets_at": 1_788_915_000},
                "status": if state.exhausted { "exhausted" } else { "ok" },
                "observed_at": 1_788_900_000, "source": "reported"}]);
        }
        report
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The text blocks of a prompt, joined.
fn prompt_text(blocks: &Value) -> String {
    blocks
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn option_of(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    args.iter()
        .find_map(|arg| arg.strip_prefix(prefix.as_str()).map(str::to_owned))
}

/// The current time as `2026-11-09T12:00:00.000Z`.
fn now() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (secs, millis) = (since.as_secs(), since.subsec_millis());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

impl Shared {
    fn send(&self, mut message: Value) {
        if let Some(object) = message.as_object_mut() {
            object.insert("jsonrpc".to_owned(), json!("2.0"));
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    }

    fn update(&self, session: &str, body: Value) {
        self.send(
            json!({"method": "session/update", "params": {"sessionId": session, "update": body}}),
        );
    }

    fn next(&self) -> u64 {
        let mut next = lock(&self.next_id);
        let id = *next;
        *next += 1;
        id
    }

    /// Changes the state as one step, and writes it where `--state=` names.
    fn with_state<R>(&self, change: impl FnOnce(&mut Value) -> R) -> R {
        let mut held = lock(&self.state);
        if let Some(path) = &self.state_path
            && let Some(read) = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            *held = read;
        }
        let result = change(&mut held);
        if let Some(path) = &self.state_path {
            let _ = std::fs::write(path, held.to_string());
        }
        result
    }

    fn note(&self, session: &str, call: &str) {
        self.with_state(|state| {
            if let Some(entry) = state.get_mut(session).and_then(Value::as_object_mut)
                && let Some(list) = entry
                    .entry("calls")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
            {
                list.push(json!(call));
            }
        });
    }

    fn remember(&self, session: &str, role: &str, text: &str, attached: &[String]) {
        self.with_state(|state| {
            if let Some(entry) = state.get_mut(session) {
                if let Some(history) = entry["history"].as_array_mut() {
                    history.push(json!({"role": role, "text": text, "attached": attached}));
                }
                entry["updatedAt"] = json!(now());
            }
        });
    }

    fn say(&self, session: &str, text: &str) {
        self.update(
            session,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}),
        );
        self.remember(session, "agent", text, &[]);
    }

    fn config_options(&self, session: &str) -> Value {
        let chosen = lock(&self.chosen);
        let pick = |id: &str, default: &str| -> Value {
            chosen
                .get(session)
                .and_then(|now| now.get(id))
                .cloned()
                .unwrap_or_else(|| json!(default))
        };
        json!([
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": pick("mode", self.modes.first().map_or("ask", String::as_str)),
             "options": self.modes.iter().map(|mode| json!({"value": mode, "name": mode})).collect::<Vec<_>>()},
            {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": pick("model", "default"),
             "options": [{"value": "default", "name": "Default model"}, {"value": "fast", "name": "Fast"},
                         {"value": "refuse-me", "name": "Refused"}]},
            {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": pick("effort", "low"),
             "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]},
        ])
    }

    /// Sends a request and waits for the client's answer, or None when the client has gone.
    fn request(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next();
        let (sender, receiver) = channel();
        lock(&self.waiting).insert(id, sender);
        self.send(json!({"id": id, "method": method, "params": params}));
        receiver.recv().ok()
    }

    /// A question as Claude's adapter shapes its AskUserQuestion tool: one choice with a header, and a free-text "Other".
    fn question(&self, session: &str, text: &str) -> Value {
        let answer = self.request(
            "elicitation/create",
            json!({
                "mode": "form", "sessionId": session, "message": text,
                "requestedSchema": {"type": "object", "properties": {
                    "q0": {"type": "string", "title": "Runner", "oneOf": [
                        {"const": "node", "title": "node --test", "description": "The runner package.json names"},
                        {"const": "npm", "title": "npm test"},
                    ]},
                    "q0_custom": {"type": "string", "title": "Other",
                        "description": "Type your own answer, or add a note to the option you chose above (optional)."},
                }},
            }),
        );
        answer
            .and_then(|answer| {
                answer
                    .get("result")
                    .filter(|result| !result.is_null())
                    .cloned()
            })
            .unwrap_or_else(|| json!({"action": "cancel"}))
    }

    /// Asks permission for a tool call, with every kind ACP has in the order Claude's adapter sends its own. An "always"
    /// answer is this agent's to remember: it holds for the tool's kind for the rest of the session, which then neither asks
    /// nor tells the client.
    fn ask(&self, session: &str, tool_call: &Value) -> Answer {
        let key = format!("{session} {}", tool_call["kind"].as_str().unwrap_or(""));
        if let Some(always) = lock(&self.remembered).get(&key) {
            return *always;
        }
        let options = [
            ("yes", "Allow", "allow_once"),
            ("yes-always", "Always allow", "allow_always"),
            ("no", "Reject", "reject_once"),
            ("no-always", "Always reject", "reject_always"),
        ];
        let listed: Vec<Value> = options
            .iter()
            .map(|(id, name, kind)| json!({"optionId": id, "name": name, "kind": kind}))
            .collect();
        let Some(answer) = self.request(
            "session/request_permission",
            json!({"sessionId": session, "toolCall": tool_call, "options": listed}),
        ) else {
            return Answer::Cancelled;
        };
        let outcome = &answer["result"]["outcome"];
        let chose = options
            .iter()
            .find(|(id, _, _)| Some(*id) == outcome["optionId"].as_str());
        let Some((_, _, kind)) = chose.filter(|_| outcome["outcome"] != "cancelled") else {
            return Answer::Cancelled;
        };
        let said = if kind.starts_with("allow") {
            Answer::Allowed
        } else {
            Answer::Refused
        };
        if kind.ends_with("always") {
            lock(&self.remembered).insert(key, said);
        }
        said
    }

    fn tool(
        &self,
        session: &str,
        id: &str,
        title: &str,
        kind: &str,
        raw_input: &Value,
        done: &Value,
    ) -> Answer {
        self.update(
            session,
            json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": title, "kind": kind,
                "status": "pending", "rawInput": raw_input}),
        );
        let answer = self.ask(
            session,
            &json!({"toolCallId": id, "title": title, "kind": kind, "rawInput": raw_input}),
        );
        if answer == Answer::Allowed {
            self.update(
                session,
                json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": "completed", "content": done}),
            );
        } else {
            self.update(
                session,
                json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": "failed"}),
            );
        }
        answer
    }

    /// One call of the host's tool server, as an MCP client does it: the text the tool answered, or the failure.
    fn tool_call(&self, session: &str, name: &str, arguments: &Value) -> String {
        self.call_on(
            (&self.tools, &self.bridges),
            "delegation",
            session,
            name,
            arguments,
        )
    }

    /// As [`Shared::tool_call`], on the host's secret tools.
    fn secret_call(&self, session: &str, name: &str, arguments: &Value) -> String {
        self.call_on(
            (&self.secret_tools, &self.secret_bridges),
            "secret",
            session,
            name,
            arguments,
        )
    }

    fn call_on(
        &self,
        (tools, bridges): (&Tools, &Bridges),
        kind: &str,
        session: &str,
        name: &str,
        arguments: &Value,
    ) -> String {
        let Some((url, token)) = lock(tools).get(session).cloned() else {
            return format!("this session has no {kind} tools");
        };
        let bridge = lock(bridges).get(session).cloned();
        if let Some(bridge) = bridge {
            let mut bridge = lock(&bridge);
            let answered = bridge.open().and_then(|()| {
                bridge.request("tools/call", &json!({"name": name, "arguments": arguments}))
            });
            return match answered {
                Ok(answer) if answer.get("error").is_some() => {
                    format!(
                        "rpc error: {}",
                        answer["error"]["message"].as_str().unwrap_or("")
                    )
                }
                Ok(answer) => {
                    let said = answer["result"]["content"][0]["text"]
                        .as_str()
                        .unwrap_or("");
                    if answer["result"]["isError"] == true {
                        format!("error: {said}")
                    } else {
                        said.to_owned()
                    }
                }
                Err(error) => format!("no answer: {error}"),
            };
        }
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        match post(&url, &token, &body.to_string()) {
            Ok((200, text)) => {
                let answer: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let said = answer["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or(&text);
                if answer["result"]["isError"] == true {
                    format!("error: {said}")
                } else {
                    said.to_owned()
                }
            }
            Ok((status, text)) => format!("http {status}: {text}"),
            Err(error) => format!("no answer: {error}"),
        }
    }

    /// The prompts of a session that delegates, or that a task is; `None` for any other.
    fn delegated(&self, session: &str, text: &str, blocks: usize) -> Option<&'static str> {
        let (word, rest) = text.split_once(' ').unwrap_or((text, ""));
        let said = match word {
            "Delegate" => {
                let (source, task) = rest.split_once(' ').unwrap_or((rest, ""));
                self.tool_call(
                    session,
                    "delegate_task",
                    &json!({"source": source, "task": task}),
                )
            }
            "DelegateIn" => {
                let mut words = rest.splitn(3, ' ');
                let (folder, source, task) = (
                    words.next().unwrap_or(""),
                    words.next().unwrap_or(""),
                    words.next().unwrap_or(""),
                );
                self.tool_call(
                    session,
                    "delegate_task",
                    &json!({"source": source, "task": task, "folder": folder}),
                )
            }
            "DelegateAwait" => {
                // Hands a task on and stays in its turn until the task ends, so what the task asks of the person reaches
                // the person through this session, and answers with the task's last state.
                let (source, task) = rest.split_once(' ').unwrap_or((rest, ""));
                let handed = self.tool_call(
                    session,
                    "delegate_task",
                    &json!({"source": source, "task": task}),
                );
                let id = serde_json::from_str::<Value>(&handed)
                    .ok()
                    .and_then(|answer| answer["taskId"].as_str().map(str::to_owned));
                match id {
                    None => handed,
                    Some(id) => {
                        let mut last = handed;
                        for _ in 0..60 {
                            last = self.tool_call(
                                session,
                                "task_status",
                                &json!({"taskId": id, "wait_ms": 5000}),
                            );
                            let ended = serde_json::from_str::<Value>(&last).is_ok_and(|state| {
                                ["completed", "failed", "canceled", "rejected"]
                                    .iter()
                                    .any(|name| state["state"] == *name)
                            });
                            if ended {
                                break;
                            }
                        }
                        last
                    }
                }
            }
            "Status" => {
                let (id, wait) = rest.split_once(' ').unwrap_or((rest, "0"));
                let wait: u64 = wait.trim().parse().unwrap_or(0);
                self.tool_call(
                    session,
                    "task_status",
                    &json!({"taskId": id, "wait_ms": wait}),
                )
            }
            "Cancel" => self.tool_call(session, "cancel", &json!({"taskId": rest.trim()})),
            "Call" => {
                let (name, arguments) = rest.split_once(' ').unwrap_or((rest, "{}"));
                let arguments = serde_json::from_str(arguments).unwrap_or(Value::Null);
                self.tool_call(session, name, &arguments)
            }
            "Secret" => {
                let (name, arguments) = rest.split_once(' ').unwrap_or((rest, "{}"));
                let arguments = serde_json::from_str(arguments).unwrap_or(Value::Null);
                self.secret_call(session, name, &arguments)
            }
            "Mode" => {
                let mode = lock(&self.chosen)
                    .get(session)
                    .and_then(|chosen| chosen.get("mode"))
                    .and_then(Value::as_str)
                    .or_else(|| self.modes.first().map(String::as_str))
                    .unwrap_or("")
                    .to_owned();
                format!("mode: {mode}")
            }
            "Verbatim" => {
                let count = blocks;
                format!("blocks={count} text={text}")
            }
            "Pid" => format!("pid: {}", std::process::id()),
            "Cwd" => format!(
                "cwd: {}",
                lock(&self.cwds).get(session).cloned().unwrap_or_default()
            ),
            "Bridge" => match lock(&self.bridges).get(session) {
                Some(bridge) => {
                    let bridge = lock(bridge);
                    format!("pid={} command={}", bridge.child.id(), bridge.command)
                }
                None => "no bridge".to_owned(),
            },
            "BridgeClose" => match lock(&self.bridges).get(session).cloned() {
                Some(bridge) => {
                    let mut bridge = lock(&bridge);
                    bridge.input = None;
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    loop {
                        match bridge.child.try_wait() {
                            Ok(Some(status)) => break format!("bridge ended: {status}"),
                            Ok(None) if std::time::Instant::now() < until => {
                                std::thread::sleep(std::time::Duration::from_millis(20));
                            }
                            _ => break "bridge still running".to_owned(),
                        }
                    }
                }
                None => "no bridge".to_owned(),
            },
            "Raw" => {
                // One line as the text gives it, with its escapes: what an agent that writes its own JSON can send.
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{rest}");
                let _ = out.flush();
                "sent".to_owned()
            }
            "SecretBearer" => match lock(&self.secret_tools).get(session) {
                Some((url, token)) => format!("{url} {token}"),
                None => "no bearer".to_owned(),
            },
            "SecretBridge" => match lock(&self.secret_bridges).get(session) {
                Some(bridge) => {
                    let bridge = lock(bridge);
                    format!("pid={} command={}", bridge.child.id(), bridge.command)
                }
                None => "no bridge".to_owned(),
            },
            "Bearer" => match lock(&self.tools).get(session) {
                Some((url, token)) => format!("{url} {token}"),
                None => "no bearer".to_owned(),
            },
            "Wait" | "PidFile" => {
                if word == "PidFile" {
                    let _ = std::fs::write(rest.trim(), std::process::id().to_string());
                }
                while lock(&self.turn).is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                return Some("cancelled");
            }
            _ => return None,
        };
        self.update(
            session,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}}),
        );
        Some("end_turn")
    }

    fn prompt(&self, session: &str, blocks: &Value) -> &'static str {
        let text = prompt_text(blocks);
        let blocks = blocks.as_array().cloned().unwrap_or_default();
        let attached: Vec<String> = blocks
            .iter()
            .filter(|block| block["type"] == "resource")
            .map(|block| {
                block["resource"]["uri"]
                    .as_str()
                    .unwrap_or("a file")
                    .to_owned()
            })
            .collect();
        if let Some(stop) = self.delegated(session, &text, blocks.len()) {
            return stop;
        }
        self.remember(session, "user", &text, &attached);
        if let Some(usage) = &self.usage
            && let Some(used) = text
                .strip_prefix("quota ")
                .and_then(|n| n.trim().parse::<u64>().ok())
        {
            let notify = {
                let mut state = lock(&usage.state);
                state.used = Some(used);
                state.exhausted = used >= 1000;
                state.revision += 1;
                state.notify
            };
            if notify {
                self.send(json!({"method": "_coqui_acp/usage_update",
                    "params": {"sessionId": session, "report": usage.report()}}));
            }
            self.say(session, &format!("quota {used}"));
            return "end_turn";
        }
        if let Some(said) = text.strip_prefix("refuse ") {
            self.say(session, said);
            return "refusal";
        }
        if let Some(said) = text.strip_prefix("Say ") {
            self.say(session, said);
            return "end_turn";
        }
        if let Some(asked) = text.strip_prefix("Ask ") {
            if !*lock(&self.forms) {
                self.say(session, "I cannot ask: this client takes no forms.");
                return "end_turn";
            }
            let answer = self.question(session, asked);
            let content = &answer["content"];
            let reply = match answer["action"].as_str() {
                Some("accept") => {
                    let custom = content["q0_custom"]
                        .as_str()
                        .filter(|custom| !custom.is_empty())
                        .map(|custom| format!(" ({custom})"))
                        .unwrap_or_default();
                    format!(
                        "You chose {}{custom}.",
                        content["q0"].as_str().unwrap_or("nothing")
                    )
                }
                Some("decline") => "You declined to answer.".to_owned(),
                _ => "The question was cancelled.".to_owned(),
            };
            self.say(session, &reply);
            return "end_turn";
        }
        self.update(
            session,
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Looking at the repository."}}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "plan", "entries": [
                {"content": "Read the code", "status": "completed", "priority": "high"},
                {"content": "Change it", "status": "in_progress", "priority": "high"},
                {"content": "Run the tests", "status": "pending", "priority": "medium"},
            ]}),
        );
        if !attached.is_empty() {
            self.say(session, &format!("Following {}.", attached.join(", ")));
            self.tool(
                session,
                "readme",
                "Edit README.md",
                "edit",
                &json!({"file_path": "README.md"}),
                &json!([{"type": "diff", "path": "README.md", "oldText": "# tiny\n",
                    "newText": "# tiny\nMaintainer: Ana Rivera\n"}]),
            );
            let answer = self.tool(
                session,
                "rm",
                "rm obsolete.txt",
                "execute",
                &json!({"command": "rm obsolete.txt"}),
                &json!([{"type": "content", "content": {"type": "text", "text": ""}}]),
            );
            match answer {
                Answer::Cancelled => return "cancelled",
                Answer::Allowed => self.say(session, "Deleted obsolete.txt."),
                Answer::Refused => {
                    self.say(session, "You refused, so obsolete.txt is still there.");
                }
            }
        } else if text.contains("fails") {
            self.fix(session);
        } else if text.starts_with("Rename") {
            let answer = self.tool(
                session,
                "rename",
                "Edit src/calc.js",
                "edit",
                &json!({"file_path": "src/calc.js"}),
                &json!([{"type": "diff", "path": "src/calc.js", "oldText": "addNumbers", "newText": "sum"}]),
            );
            if answer == Answer::Cancelled {
                return "cancelled";
            }
        } else {
            self.tool(
                session,
                "rename2",
                "Edit src/report.js",
                "edit",
                &json!({"file_path": "src/report.js"}),
                &json!([{"type": "diff", "path": "src/report.js", "oldText": "addNumbers", "newText": "sum"}]),
            );
            self.say(session, "Renamed everywhere.");
        }
        self.update(
            session,
            json!({"sessionUpdate": "usage_update", "used": 1200, "size": 200_000,
                "cost": {"amount": 0.01, "currency": "USD"}}),
        );
        "end_turn"
    }

    /// The fix turn, which also sends v2's updates: the app shows them whichever version an agent speaks.
    fn fix(&self, session: &str) {
        self.update(
            session,
            json!({"sessionUpdate": "tool_call", "toolCallId": "read", "title": "Read src/calc.js", "kind": "read",
                "status": "completed", "locations": [{"path": "src/calc.js", "line": 1}],
                "content": [{"type": "content", "content": {"type": "text", "text": "return a - b;"}}]}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "tool_call_content_chunk", "toolCallId": "read",
                "content": {"type": "content", "content": {"type": "text", "text": "}"}}}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "notice", "severity": "info", "title": "Using the project's own test runner"}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "compaction_summary_chunk", "compactionId": "c1",
                "content": {"type": "text", "text": "calc.js subtracts where it should add."}}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "compaction_update", "compactionId": "c1", "status": "completed"}),
        );
        self.update(
            session,
            json!({"sessionUpdate": "plan_update", "plan": {"type": "items", "planId": "fix", "entries": [
                {"content": "Read the code", "status": "completed", "priority": "high"},
                {"content": "Change it", "status": "in_progress", "priority": "high"},
                {"content": "Rewrite the test", "status": "cancelled", "priority": "low"},
                {"content": "Run the tests", "status": "pending", "priority": "medium"},
            ]}}),
        );
        self.tool(
            session,
            "fix",
            "Edit src/calc.js",
            "edit",
            &json!({"file_path": "src/calc.js"}),
            &json!([{"type": "diff", "path": "src/calc.js", "oldText": "return a - b;\n", "newText": "return a + b;\n"}]),
        );
        self.tool(
            session,
            "tests",
            "node --test",
            "execute",
            &json!({"command": "node --test"}),
            &json!([{"type": "content", "content": {"type": "text", "text": "# pass 2\n# fail 0"}}]),
        );
        self.say(session, "Fixed: it subtracted. Two tests pass.");
    }

    fn handle(self: &Arc<Self>, message: &Value) {
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            if let Some(waiting) = id
                .and_then(|id| id.as_u64())
                .and_then(|id| lock(&self.waiting).remove(&id))
            {
                let _ = waiting.send(message.clone());
            }
            return;
        };
        let session = params["sessionId"].as_str().unwrap_or("").to_owned();
        match (method, id) {
            ("initialize", Some(id)) => {
                // A client offering v2 says it under capabilities, and v1's under clientCapabilities.
                let capabilities = [&params["capabilities"], &params["clientCapabilities"]]
                    .into_iter()
                    .find(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(Value::Null);
                *lock(&self.forms) = !capabilities["elicitation"]["form"].is_null();
                if let Some(usage) = &self.usage {
                    let asked = &capabilities["_meta"][USAGE_CAPABILITY];
                    let mut state = lock(&usage.state);
                    state.notify = asked["notify"] == true;
                    state.account = asked["account"].as_str().map(str::to_owned);
                }
                let agent = if self.reopens {
                    json!({"loadSession": true, "promptCapabilities": {"embeddedContext": true, "image": true},
                           "sessionCapabilities": {"list": {}}})
                } else {
                    json!({"promptCapabilities": {"embeddedContext": true, "image": true}})
                };
                let mut agent = agent;
                if self.system_prompt {
                    agent["sessionCapabilities"]["systemPrompt"] = json!({});
                }
                if self.http {
                    agent["mcpCapabilities"] = json!({"http": true});
                }
                if self.usage.is_some() {
                    agent["_meta"] = json!({USAGE_CAPABILITY: {"version": 1, "sources": ["reported"], "spend": false}});
                }
                let mut result = json!({"protocolVersion": 1, "agentCapabilities": agent});
                if let Some(info) = &self.agent_info {
                    result["agentInfo"] = info.clone();
                }
                self.send(json!({"id": id, "result": result}));
            }
            ("_coqui_acp/usage", Some(id)) => match &self.usage {
                Some(usage) => self.send(json!({"id": id, "result": usage.report()})),
                None => self.send(
                    json!({"id": id, "error": {"code": -32601, "message": "no such method"}}),
                ),
            },
            ("session/new", Some(id)) => {
                let created = self.with_state(|state| {
                    if !state.is_object() {
                        *state = json!({});
                    }
                    let object = state.as_object_mut().expect("the state is an object");
                    let created = format!("scripted-{}", object.len() + 1);
                    object.insert(
                        created.clone(),
                        json!({"cwd": params["cwd"], "title": "Fix the failing test", "history": [],
                               "updatedAt": now(), "calls": ["session/new"], "system": params["systemPrompt"]}),
                    );
                    created
                });
                lock(&self.cwds).insert(
                    created.clone(),
                    params["cwd"].as_str().unwrap_or("").to_owned(),
                );
                for server in params["mcpServers"].as_array().into_iter().flatten() {
                    let bearer = server["headers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|header| header["name"] == "Authorization")
                        .and_then(|header| header["value"].as_str())
                        .and_then(|value| value.strip_prefix("Bearer "));
                    if let (Some(url), Some(token)) = (server["url"].as_str(), bearer) {
                        let kept = match server["name"].as_str() {
                            Some("canoa-delegate") => Some(&self.tools),
                            Some("canoa-secrets") => Some(&self.secret_tools),
                            _ => None,
                        };
                        if let Some(kept) = kept {
                            lock(kept).insert(created.clone(), (url.to_owned(), token.to_owned()));
                        }
                    } else if let Some(name) = server["name"].as_str()
                        && (name == "canoa-delegate" || name == "canoa-secrets")
                        && let Some((bridge, url, token)) = Bridge::start(server)
                    {
                        let (tools, bridges) = if name == "canoa-delegate" {
                            (&self.tools, &self.bridges)
                        } else {
                            (&self.secret_tools, &self.secret_bridges)
                        };
                        lock(tools).insert(created.clone(), (url, token));
                        lock(bridges).insert(created.clone(), Arc::new(Mutex::new(bridge)));
                    }
                }
                let mut opened =
                    json!({"sessionId": created, "configOptions": self.config_options(&created)});
                if self.forge {
                    opened["_meta"] = json!({"dev.boriken.canoa/profile": {"items": [], "mode": {"value": "bypass"}}});
                }
                self.send(json!({"id": id, "result": opened}));
                self.update(
                    &created,
                    json!({"sessionUpdate": "available_commands_update",
                        "availableCommands": [{"name": "review", "description": "Review the change"}]}),
                );
            }
            ("session/set_config_option", Some(id)) => {
                let value = params["value"].clone();
                if value == "refuse-me" {
                    self.send(json!({"id": id, "error": {"code": -32602, "message": "That model is not available"}}));
                } else {
                    let config = params["configId"].as_str().unwrap_or("").to_owned();
                    lock(&self.chosen)
                        .entry(session.clone())
                        .or_default()
                        .insert(config.clone(), value.clone());
                    self.note(
                        &session,
                        &format!("set {config}={}", value.as_str().unwrap_or("")),
                    );
                    self.send(json!({"id": id, "result": {"configOptions": self.config_options(&session)}}));
                }
            }
            ("session/prompt", Some(id)) => {
                self.note(&session, "session/prompt");
                *lock(&self.turn) = Some(id.clone());
                let shared = Arc::clone(self);
                let blocks = params["prompt"].clone();
                std::thread::spawn(move || {
                    let text = prompt_text(&blocks);
                    let failure = text
                        .strip_prefix("fail ")
                        .map(|said| json!({"code": -32603, "message": said}))
                        .or_else(|| {
                            text.strip_prefix("fail-data ").map(|said| {
                                json!({"code": -32603, "message": "Internal error", "data": {"details": said}})
                            })
                        });
                    if let Some(error) = failure {
                        let mut turn = lock(&shared.turn);
                        if turn.as_ref() == Some(&id) {
                            shared.send(json!({"id": id, "error": error}));
                        }
                        *turn = None;
                        return;
                    }
                    let stop = shared.prompt(&session, &blocks);
                    let mut turn = lock(&shared.turn);
                    if turn.as_ref() == Some(&id) {
                        shared.send(json!({"id": id, "result": {"stopReason": stop}}));
                    }
                    *turn = None;
                });
            }
            ("session/cancel", _) => {
                if let Some(turn) = lock(&self.turn).take() {
                    self.send(json!({"id": turn, "result": {"stopReason": "cancelled"}}));
                }
            }
            ("session/list", Some(id)) => {
                let cwd = params["cwd"].as_str().filter(|cwd| !cwd.is_empty());
                let listed: Vec<Value> = self.with_state(|state| {
                    state
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, entry)| {
                            entry["history"]
                                .as_array()
                                .is_some_and(|history| !history.is_empty())
                                && cwd.is_none_or(|cwd| entry["cwd"] == cwd)
                        })
                        .map(|(session, entry)| {
                            json!({"sessionId": session, "cwd": entry["cwd"], "title": entry["title"],
                                "updatedAt": entry["updatedAt"]})
                        })
                        .collect()
                });
                self.send(json!({"id": id, "result": {"sessions": listed}}));
            }
            ("session/load", Some(id)) => {
                let history = self.with_state(|state| state[&session]["history"].clone());
                for said in history.as_array().into_iter().flatten() {
                    let kind = if said["role"] == "user" {
                        "user_message_chunk"
                    } else {
                        "agent_message_chunk"
                    };
                    self.update(
                        &session,
                        json!({"sessionUpdate": kind, "content": {"type": "text", "text": said["text"]}}),
                    );
                    // What went with a message comes back as its own chunk, a resource with no text of its own.
                    for uri in said["attached"].as_array().into_iter().flatten() {
                        self.update(
                            &session,
                            json!({"sessionUpdate": "user_message_chunk",
                                "content": {"type": "resource", "resource": {"uri": uri, "text": ""}}}),
                        );
                    }
                }
                self.send(json!({"id": id, "result": {}}));
            }
            (_, Some(id)) => self.send(json!({"id": id, "result": {}})),
            _ => {}
        }
    }
}

/// Runs the scripted agent on the standard streams until its input closes.
pub fn run(args: &[String]) {
    let agent_info = option_of(args, "name").map(|name| {
        json!({"name": name, "version": option_of(args, "version").unwrap_or_else(|| "0.0.0".to_owned())})
    });
    let shared = Arc::new(Shared {
        state_path: option_of(args, "state").map(PathBuf::from),
        state: Mutex::new(json!({})),
        agent_info,
        reopens: !args.iter().any(|arg| arg == "--no-reopen"),
        system_prompt: args.iter().any(|arg| arg == "--system-prompt"),
        waiting: Mutex::new(HashMap::new()),
        next_id: Mutex::new(1000),
        remembered: Mutex::new(HashMap::new()),
        chosen: Mutex::new(HashMap::new()),
        forms: Mutex::new(false),
        turn: Mutex::new(None),
        usage: args.iter().any(|arg| arg == "--usage").then(|| Usage {
            claims: option_of(args, "usage-claims"),
            state: Mutex::new(UsageState::default()),
        }),
        modes: option_of(args, "modes").map_or_else(
            || vec!["ask".to_owned(), "edits".to_owned()],
            |list| list.split(',').map(str::to_owned).collect(),
        ),
        http: args.iter().any(|arg| arg == "--mcp-http"),
        forge: args.iter().any(|arg| arg == "--forge-profile"),
        secret_tools: Mutex::new(HashMap::new()),
        tools: Mutex::new(HashMap::new()),
        bridges: Mutex::new(HashMap::new()),
        secret_bridges: Mutex::new(HashMap::new()),
        cwds: Mutex::new(HashMap::new()),
    });
    if let Some(marker) = option_of(args, "marker")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(marker)
    {
        let _ = writeln!(file, "{}", std::process::id());
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        if let Ok(message) = serde_json::from_str::<Value>(&line) {
            shared.handle(&message);
        }
    }
}

/// One HTTP POST to the host's tool server with a bearer, on a connection of its own: the status and the body.
fn post(url: &str, token: &str, body: &str) -> std::io::Result<(u16, String)> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let mut stream = std::net::TcpStream::connect(authority)?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    let status = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = answer.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    Ok((status, body.to_owned()))
}
