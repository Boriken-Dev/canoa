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
//! it says of itself, and `--no-reopen` leaves out session/load and session/list.

use std::collections::HashMap;
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
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

struct Shared {
    state_path: Option<PathBuf>,
    /// The state when no file is named, and the lock that makes a read-change-write one step.
    state: Mutex<Value>,
    agent_info: Option<Value>,
    reopens: bool,
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
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": pick("mode", "ask"),
             "options": [{"value": "ask", "name": "Ask first"}, {"value": "edits", "name": "Accept edits"}]},
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

    fn prompt(&self, session: &str, blocks: &Value) -> &'static str {
        let blocks = blocks.as_array().cloned().unwrap_or_default();
        let text = blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
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
                               "updatedAt": now(), "calls": ["session/new"]}),
                    );
                    created
                });
                self.send(json!({"id": id, "result": {
                    "sessionId": created, "configOptions": self.config_options(&created)}}));
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
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        if let Ok(message) = serde_json::from_str::<Value>(&line) {
            shared.handle(&message);
        }
    }
}
