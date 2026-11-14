//! A host with the scripted agent as a source of its own and an overlay `team` whose hook logs what it is told: what the
//! hook tests share.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    dead_code,
    reason = "tests name what they use from std, and not every test uses all of it"
)]

pub mod script;

use std::path::{Path, PathBuf};

use canoa_host::Link;
use canoa_host::profile::{APPROVE_META, APPROVE_METHOD};
use serde_json::{Value, json};

use crate::linked::{self, Hosted, ask};
use crate::support::{Scratch, cwd, example, quoted};
pub use script::{Arm, logged};

/// What canoa.cfg says of the overlay's hooks.
pub enum Allow {
    /// Its tree digest: the hooks are on.
    Digest,
    /// Another digest: they were allowed at a version the overlay no longer is.
    Other,
    /// Nothing.
    Nothing,
}

/// A running host, its link, and where the hook and the agent write.
pub struct Setup {
    pub host: Hosted,
    pub link: Link,
    pub log: PathBuf,
    pub state: PathBuf,
    pub digest: String,
    pub cwd: String,
    _logs: Scratch,
}

/// What the overlay holds: its manifest and the one hook script.
fn files(manifest: &str, log: &Path, arms: &[(&str, Arm)]) -> Vec<(String, String)> {
    vec![
        ("overlay.cfg".to_owned(), manifest.to_owned()),
        (script::file_name().to_owned(), script::body(log, arms)),
    ]
}

fn write_all(root: &Path, files: &[(String, String)]) {
    std::fs::create_dir_all(root).expect("a directory");
    for (path, text) in files {
        std::fs::write(root.join(path), text).expect("written");
    }
}

/// Starts a host serving the scripted agent `Scripted` with overlay `team`: `manifest` is its `overlay.cfg`, `arms` what its
/// hook does, `allow` what canoa.cfg says, `flags` more arguments of the agent (`--name=coqui-acp-claude` is always given, so
/// the harness is known).
pub fn start(manifest: &str, arms: &[(&str, Arm)], allow: &Allow, flags: &[&str]) -> Setup {
    let mut args = vec![
        "--scripted".to_owned(),
        "--name=coqui-acp-claude".to_owned(),
    ];
    args.extend(flags.iter().map(|flag| (*flag).to_owned()));
    start_program(
        manifest,
        arms,
        allow,
        &example("stdio_agent", false).to_string_lossy(),
        &args,
        true,
    )
}

/// As [`start`], with another program as the agent: `args` are all of its arguments, and the scripted agent's state file is
/// added when `scripted`.
pub fn start_program(
    manifest: &str,
    arms: &[(&str, Arm)],
    allow: &Allow,
    program: &str,
    args: &[String],
    scripted: bool,
) -> Setup {
    let logs = Scratch::new();
    let log = logs.0.join("hook.log");
    let state = logs.0.join("state.json");
    let staged = files(manifest, &log, arms);
    write_all(&logs.0.join("stage"), &staged);
    let (_, digest) = coqui_aos::read_tree(
        &coqui_aos::StdFs,
        &logs.0.join("stage"),
        &coqui_aos::Settings::default(),
    )
    .expect("read");
    let digest = digest.digest.to_string();
    let named = match allow {
        Allow::Digest => Some(digest.clone()),
        Allow::Other => Some(format!("sha256:{}", "0".repeat(64))),
        Allow::Nothing => None,
    };
    let mut args = args.to_vec();
    if scripted {
        args.push(format!("--state={}", state.to_string_lossy()));
    }
    let mut config = format!(
        "[agents.Scripted]\nprogram = {}\nargs = [{}]\n[overlays]\nteam = \"fixture\"\n",
        quoted(program),
        args.iter()
            .map(|arg| quoted(arg))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(named) = named {
        config.push_str(&format!("[agents.Scripted.hooks]\nteam = \"{named}\"\n"));
    }
    let host = Hosted::start_with(&config, &["--local-sources"]);
    write_all(&host.scratch.0.join("overlays").join("team"), &staged);
    std::fs::write(
        host.scratch.0.join("overlays").join("team.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest, "files": {}}).to_string(),
    )
    .expect("written");
    let link = host.local.open("Scripted").expect("a link");
    let cwd = cwd(&host.scratch);
    Setup {
        host,
        link,
        log,
        state,
        digest,
        cwd,
        _logs: logs,
    }
}

/// An `overlay.cfg` with a `[hooks]` table.
pub fn hooks(table: &str) -> String {
    format!("[hooks]\n{table}\n")
}

impl Setup {
    /// `initialize`, saying the client answers the host's questions when `answers`; the profile's report.
    pub fn initialize(&self, answers: bool) -> Value {
        let meta = if answers {
            json!({APPROVE_META: true})
        } else {
            json!({})
        };
        ask(
            &self.link,
            1,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": meta}),
        )
        .expect("initialize is answered")
        .0
    }

    /// Sends a request and answers what the host asks on the way (`result` for each question, or an error when none):
    /// the request's answer or error, the questions asked, and every other message.
    pub fn request(
        &self,
        id: u64,
        method: &str,
        params: Value,
        result: Option<Value>,
    ) -> (Result<Value, Value>, Vec<Value>, Vec<Value>) {
        linked::send(
            &self.link,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        let (mut asked, mut seen) = (Vec::new(), Vec::new());
        loop {
            let message = linked::next(&self.link).expect("a message");
            if message["method"] == APPROVE_METHOD {
                asked.push(message.clone());
                let reply = match &result {
                    Some(result) => {
                        json!({"jsonrpc": "2.0", "id": message["id"], "result": result})
                    }
                    None => json!({"jsonrpc": "2.0", "id": message["id"],
                        "error": {"code": -32601, "message": "no"}}),
                };
                linked::send(&self.link, &reply);
            } else if message["id"] == id && message.get("method").is_none() {
                let outcome = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
                return (outcome, asked, seen);
            } else {
                seen.push(message);
            }
        }
    }

    /// A new session: its answer.
    pub fn new_session(&self, id: u64) -> Result<Value, Value> {
        self.request(
            id,
            "session/new",
            json!({"cwd": self.cwd, "mcpServers": []}),
            None,
        )
        .0
    }

    /// A prompt, with every permission the agent asks for answered with `option` (`""` cancels): the turn's answer and
    /// every other message, the questions among them.
    pub fn turn(
        &self,
        id: u64,
        session: &str,
        text: &str,
        option: &str,
    ) -> (Result<Value, Value>, Vec<Value>) {
        linked::send(
            &self.link,
            &json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
                "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}}),
        );
        let mut seen = Vec::new();
        loop {
            let message = linked::next(&self.link).expect("a message");
            if message["method"] == "session/request_permission" {
                let outcome = if option.is_empty() {
                    json!({"outcome": "cancelled"})
                } else {
                    json!({"outcome": "selected", "optionId": option})
                };
                linked::send(
                    &self.link,
                    &json!({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": outcome}}),
                );
                seen.push(message);
            } else if message["id"] == id && message.get("method").is_none() {
                let outcome = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
                return (outcome, seen);
            } else {
                seen.push(message);
            }
        }
    }

    /// The agent's note of a session.
    pub fn agent_state(&self, session: &str) -> Value {
        std::fs::read_to_string(&self.state)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map_or(Value::Null, |state| state[session].clone())
    }

    /// The payloads the hook logged for `event`, once at least `count` are there.
    pub fn events(&self, event: &str, count: usize) -> Vec<Value> {
        let until = std::time::Instant::now() + crate::support::WAIT;
        loop {
            let found: Vec<Value> = logged(&self.log)
                .into_iter()
                .filter(|(each, _)| each == event)
                .map(|(_, payload)| payload)
                .collect();
            if found.len() >= count || std::time::Instant::now() >= until {
                return found;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// An item of a profile report.
pub fn item<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no {key} in {report}"))
}

/// Whether the report has an item.
pub fn has(report: &Value, key: &str) -> bool {
    report["items"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["key"] == key))
}

/// The text of what the agent said in these messages.
pub fn said(messages: &[Value]) -> String {
    linked::said(messages)
}
