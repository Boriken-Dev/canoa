//! The scripted agent calls a tool and gets a prompt of any HTTP MCP server its session was given, by name, with the server's
//! bearer; over `https` it trusts only the certificates in `--mcp-ca`. The browser tests rely on it to show what reached a
//! server that a page named.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use support::{Scratch, example};

const BEARER: &str = "Bearer canary-token";

/// The `mcp_http` stand-in, listening, with the file it logs requests to.
struct Standin {
    child: Child,
    url: String,
    log: std::path::PathBuf,
}

impl Standin {
    /// `https` with its certificate written to `ca`, or `http` when `ca` is none.
    fn start(scratch: &Scratch, name: &str, ca: Option<&Path>) -> Self {
        let log = scratch.0.join(format!("{name}.log"));
        let mut command = Command::new(example("mcp_http", false));
        command.arg(format!("--log={}", log.display()));
        match ca {
            Some(ca) => command.arg(format!("--ca={}", ca.display())),
            None => command.arg("--plain"),
        };
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the stand-in starts");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("its output"))
            .read_line(&mut line)
            .expect("it says it is ready");
        let port = serde_json::from_str::<Value>(&line).expect("JSON")["port"]
            .as_u64()
            .expect("a port");
        let scheme = if ca.is_some() { "https" } else { "http" };
        Self {
            child,
            url: format!("{scheme}://127.0.0.1:{port}/mcp"),
            log,
        }
    }

    /// The authorization header and method of each request it served.
    fn seen(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let entry: Value = serde_json::from_str(line).expect("JSON");
                (
                    entry["authorization"].as_str().unwrap_or("").to_owned(),
                    entry["method"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect()
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Agent {
    child: Child,
    input: ChildStdin,
    lines: BufReader<ChildStdout>,
    next: u64,
    session: String,
}

impl Agent {
    fn start(extra: &[String], server: &Standin) -> Self {
        let mut child = Command::new(example("stdio_agent", false))
            .args(["--scripted", "--mcp-http"])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the scripted agent starts");
        let input = child.stdin.take().expect("its input");
        let lines = BufReader::new(child.stdout.take().expect("its output"));
        let mut agent = Self {
            child,
            input,
            lines,
            next: 0,
            session: String::new(),
        };
        agent.call("initialize", &json!({"protocolVersion": 1}));
        let opened = agent.call(
            "session/new",
            &json!({"cwd": "/", "mcpServers": [
                {"type": "http", "name": "office", "url": server.url,
                 "headers": [{"name": "Authorization", "value": BEARER}]},
            ]}),
        );
        agent.session = opened["result"]["sessionId"]
            .as_str()
            .expect("a session")
            .to_owned();
        agent
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next += 1;
        let id = self.next;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input, "{line}").expect("it is written");
        loop {
            let mut text = String::new();
            assert!(
                self.lines.read_line(&mut text).expect("a line") > 0,
                "the agent ended"
            );
            let message: Value = serde_json::from_str(&text).expect("JSON");
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
    }

    /// What the agent said in answer to one prompt.
    fn say(&mut self, text: &str) -> String {
        let session = self.session.clone();
        self.next += 1;
        let id = self.next;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}});
        writeln!(self.input, "{line}").expect("it is written");
        let mut said = String::new();
        loop {
            let mut text = String::new();
            assert!(
                self.lines.read_line(&mut text).expect("a line") > 0,
                "the agent ended"
            );
            let message: Value = serde_json::from_str(&text).expect("JSON");
            if message["id"] == id && message.get("method").is_none() {
                return said;
            }
            let update = &message["params"]["update"];
            if update["sessionUpdate"] == "agent_message_chunk" {
                said.push_str(update["content"]["text"].as_str().unwrap_or(""));
            }
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_tool_and_a_prompt_of_an_http_server_reach_it_with_its_bearer() {
    let scratch = Scratch::new();
    let server = Standin::start(&scratch, "plain", None);
    let mut agent = Agent::start(&[], &server);
    assert_eq!(agent.say("CallOn office marker {}"), "the marker");
    assert_eq!(agent.say("PromptOn office skill"), "the skill");
    assert_eq!(
        server.seen(),
        vec![
            (BEARER.to_owned(), "tools/call".to_owned()),
            (BEARER.to_owned(), "prompts/get".to_owned()),
        ]
    );
    assert_eq!(
        agent.say("CallOn nobody marker {}"),
        "this session has no server nobody"
    );
}

#[test]
fn an_https_server_is_reached_only_through_the_certificates_it_is_given() {
    let scratch = Scratch::new();
    let ca = scratch.0.join("ca.pem");
    let server = Standin::start(&scratch, "secure", Some(&ca));

    let mut trusting = Agent::start(&[format!("--mcp-ca={}", ca.display())], &server);
    assert_eq!(trusting.say("CallOn office marker {}"), "the marker");
    assert_eq!(trusting.say("PromptOn office skill"), "the skill");
    assert_eq!(server.seen().len(), 2);

    // No file, and a file of another certificate: not reached, and nothing arrives.
    let mut trusting_nothing = Agent::start(&[], &server);
    assert!(
        trusting_nothing
            .say("CallOn office marker {}")
            .starts_with("no answer")
    );
    let other = scratch.0.join("other.pem");
    let _elsewhere = Standin::start(&scratch, "elsewhere", Some(&other));
    let mut trusting_another = Agent::start(&[format!("--mcp-ca={}", other.display())], &server);
    assert!(
        trusting_another
            .say("CallOn office marker {}")
            .starts_with("no answer")
    );
    assert_eq!(
        server.seen().len(),
        2,
        "a refused certificate sent no request"
    );
}
