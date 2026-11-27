//! A role's session moves by itself: two Claude Code entries on two accounts of their own and a role over them, a session bound to the
//! first member, and a turn that ends the way Claude Code ends one at a plan's usage limit. With no client prompt the host exports the
//! session, takes the limit's words out of it, imports it on the second entry's account and sends the continuation there, then tells
//! the control link (`roles/moved`). A session budget is a limit no other member helps with, so that session stays. Stand-ins for the
//! command lines, throwaway configuration folders and a usage endpoint nobody listens on; nothing of the person's is read.
//! The tests of this file run one at a time, since each sets the environment the providers read their session files and the stand-ins' record from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, ask, next, send};
use serde_json::{Value, json};
use sha2::Digest as _;
use support::{Scratch, WAIT, cwd, example, host_binary, quoted};

const ASKED: &str = "Fix the failing build";
const CONTINUATION: &str = "Carry on with what was asked.";
const LIMIT_WORDS: &str = "usage limit reached";

fn env(name: &str, value: &Path) {
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var(name, value) };
}

fn standin(scratch: &Scratch, name: &str) -> String {
    let path = scratch
        .0
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &path).expect("the stand-in is copied");
    path.to_string_lossy().into_owned()
}

fn canoa(scratch: &Scratch, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(host_binary())
        .args(args)
        .arg("--config")
        .arg(scratch.0.join("canoa.cfg"))
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("canoa runs");
    let mut stdin = child.stdin.take().expect("its input");
    stdin.write_all(input.as_bytes()).expect("input written");
    drop(stdin);
    child.wait_with_output().expect("canoa ends")
}

/// Signs `name` in to Claude Code's stand-in, which gives it a configuration folder of its own.
fn add(scratch: &Scratch, name: &str) {
    let added = canoa(
        scratch,
        &[
            "accounts",
            "add",
            name,
            "--provider",
            "claude_accounts",
            "--method",
            "subscription",
        ],
        "123456\n",
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
}

/// The account's folder in the host's home, from `accounts list`.
fn folder_of(scratch: &Scratch, name: &str) -> PathBuf {
    let listed = canoa(scratch, &["accounts", "list", "--format", "json"], "");
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let account: Value = serde_json::from_str(line).expect("one JSON object a line");
        if account["name"] == name {
            let id = account["id"].as_str().expect("an id");
            // The host names an account's folder by a digest of its id.
            let digest: String = sha2::Sha256::digest(id.as_bytes())
                .iter()
                .take(12)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            return scratch
                .0
                .join("home")
                .join("accounts")
                .join(digest)
                .join("state");
        }
    }
    panic!("no account {name}");
}

/// The control channel, with what the host told it besides answers kept in order.
struct Control {
    link: Link,
    told: std::cell::RefCell<Vec<Value>>,
    next_id: std::cell::Cell<u64>,
}

impl Control {
    fn open(host: &Hosted) -> Self {
        Self {
            link: host.local.open("@control").expect("the control channel"),
            told: std::cell::RefCell::new(Vec::new()),
            next_id: std::cell::Cell::new(100),
        }
    }

    fn call(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        send(
            &self.link,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        loop {
            let message = next(&self.link).expect("answered");
            if message["id"] == id && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => Err((
                        error["code"].as_i64().unwrap_or(0),
                        error["message"].as_str().unwrap_or("").to_owned(),
                    )),
                    None => Ok(message["result"].clone()),
                };
            }
            self.told.borrow_mut().push(message);
        }
    }

    /// The next thing the host told this link of `method`, waiting for it up to `wait`.
    fn heard(&self, method: &str, wait: Duration) -> Option<Value> {
        let until = Instant::now() + wait;
        loop {
            let found = {
                let mut told = self.told.borrow_mut();
                told.iter()
                    .position(|each| each["method"] == method)
                    .map(|at| told.remove(at))
            };
            if found.is_some() {
                return found;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self
                .link
                .receive(Some(left.min(Duration::from_millis(200))))
            {
                Ok(Some(bytes)) => {
                    if let Ok(message) = serde_json::from_slice::<Value>(&bytes) {
                        self.told.borrow_mut().push(message);
                    }
                }
                Ok(None) => return None,
                Err(_) => {}
            }
        }
    }

    /// `roles/served` of the session, once it lists `turns` turns.
    fn served(&self, entry: &str, session: &str, turns: usize) -> Value {
        let until = Instant::now() + WAIT;
        loop {
            let served = self.call("roles/served", &json!({"entry": entry, "session": session}));
            if let Ok(served) = &served
                && served["turns"]
                    .as_array()
                    .is_some_and(|all| all.len() >= turns)
            {
                return served.clone();
            }
            assert!(
                Instant::now() < until,
                "{entry}/{session} has no {turns} served turns: {served:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// One turn of a session, its one permission allowed: what the agent said.
fn turn(link: &Link, session: &str, text: &str) -> String {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": 9, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}}),
    );
    let mut said = String::new();
    loop {
        let message = next(link).expect("the turn goes on");
        if message["method"] == "session/request_permission" {
            let allow = message["params"]["options"]
                .as_array()
                .and_then(|options| options.iter().find(|option| option["kind"] == "allow_once"))
                .expect("an allow option")
                .clone();
            send(
                link,
                &json!({"jsonrpc": "2.0", "id": message["id"],
                    "result": {"outcome": {"outcome": "selected", "optionId": allow["optionId"]}}}),
            );
        } else if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            said.push_str(
                message["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or(""),
            );
        } else if message["id"] == 9 {
            return said;
        }
    }
}

/// A prompt that asks nothing first, answered by how the turn ended.
fn prompt(link: &Link, session: &str, text: &str) -> Value {
    let (answer, _) = ask(
        link,
        7,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
    )
    .expect("the prompt is answered");
    answer
}

fn records(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON line"))
        .collect()
}

/// The prompts the stand-ins were given, each with the folder of the program that got it.
fn prompts(file: &Path) -> Vec<(String, String)> {
    records(file)
        .iter()
        .filter_map(|line| {
            Some((
                line["prompt"].as_str()?.to_owned(),
                line["config"].as_str().unwrap_or("").replace('\\', "/"),
            ))
        })
        .collect()
}

/// The text of every session file under `folder`.
fn sessions_in(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for each in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = each.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                found.push(std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    found
}

fn open(host: &Hosted, entry: &str, dir: &str) -> (Link, String) {
    let link = host.local.open(entry).expect("a link");
    ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    let (opened, _) = ask(
        &link,
        2,
        "session/new",
        json!({"cwd": dir, "mcpServers": []}),
    )
    .expect("a session opens");
    let session = opened["sessionId"].as_str().expect("an id").to_owned();
    (link, session)
}

/// The id of the first member of `role` that `roles/list` gives.
fn first_member(control: &Control, role: &str) -> String {
    let listed = control.call("roles/list", &json!({})).expect("listed");
    listed["roles"]
        .as_array()
        .and_then(|roles| roles.iter().find(|each| each["name"] == role))
        .and_then(|role| role["members"][0]["id"].as_str())
        .unwrap_or_else(|| panic!("no role {role} in {listed}"))
        .to_owned()
}

fn bind(control: &Control, role: &str, entry: &str, session: &str, dir: &str) {
    let member = first_member(control, role);
    control
        .call(
            "roles/bind",
            &json!({"role": role, "member": member, "entry": entry, "session": session, "cwd": dir}),
        )
        .expect("bound");
}

/// The tests of this file set the environment the providers read, so they run one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_session_a_usage_limit_ended_moves_to_the_next_member_by_itself_and_a_session_budget_stays() {
    scenario(false);
}

#[test]
#[ignore = "needs coqui-acp's cleaner to drop every limit turn"]
fn a_limit_turn_from_before_the_role_served_the_session_is_not_carried_either() {
    scenario(true);
}

/// The whole run. With `earlier_limit`, the session had a turn a limit ended before a role served it, and none of the limit's words
/// reach the second member.
fn scenario(earlier_limit: bool) {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new();
    // The login the programs have where they run, and the stand-ins' record, are the test's own.
    let (claude_home, codex_home) = (scratch.0.join("claude-home"), scratch.0.join("codex-home"));
    std::fs::create_dir_all(&claude_home).expect("dir");
    std::fs::create_dir_all(&codex_home).expect("dir");
    std::fs::write(codex_home.join("auth.json"), "{\"fixture\":true}").expect("a login");
    env("CLAUDE_CONFIG_DIR", &claude_home);
    env("CODEX_HOME", &codex_home);
    let record = scratch.0.join("record.jsonl");
    env("STANDIN_RECORD", &record);
    // A session that chose no model runs as `default`, which the accounts must list for a move to be able to go there.
    env("STANDIN_DEFAULT_MODEL", Path::new("1"));
    let (claude, codex) = (standin(&scratch, "claude"), standin(&scratch, "codex"));
    // Nobody listens where the usage of the accounts would be read, so no member reports any and each can serve.
    let entry = |name: &str, harness: &str, program: &str, more: &str| {
        format!(
            "\n[host.agents.{name}]\nharness = \"{harness}\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n{more}",
            quoted(program)
        )
    };
    let usage = "account_settings = { usage_endpoint = \"http://127.0.0.1:9/usage\" }\n";
    std::fs::write(
        scratch.0.join("canoa.cfg"),
        format!(
            "[secrets]\naccounts = \"file\"\n{}{}{}{}",
            entry("Claude1", "claude", &claude, usage),
            entry("Claude2", "claude", &claude, usage),
            entry("Codex1", "codex", &codex, ""),
            entry("Codex2", "codex", &codex, ""),
        ),
    )
    .expect("canoa.cfg is written");
    add(&scratch, "work");
    add(&scratch, "other");
    let (work, other) = (folder_of(&scratch, "work"), folder_of(&scratch, "other"));
    // Only the first account's plan is used up.
    env("STANDIN_LIMIT_IN", &work);
    let cfg = std::fs::read_to_string(scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    let cfg = cfg
        .replace(
            "[host.agents.Claude1]\n",
            "[host.agents.Claude1]\naccounts = [\"work\"]\n",
        )
        .replace(
            "[host.agents.Claude2]\n",
            "[host.agents.Claude2]\naccounts = [\"other\"]\n",
        );
    let cfg = format!(
        "{cfg}\n[host.roles.exec]\nresume_prompt = {}\nmembers = [\n  {{ agent = \"Claude1\", account = \"work\" }},\n  {{ agent = \"Claude2\", account = \"other\" }},\n]\n\n\
         [host.roles.spend]\nresume_prompt = \"Carry on, spending.\"\nmembers = [{{ agent = \"Codex1\" }}, {{ agent = \"Codex2\" }}]\n",
        quoted(CONTINUATION)
    );
    let dir = cwd(&scratch);
    let host = Hosted::start_in(scratch, &cfg, &[]);
    let control = Control::open(&host);

    let work_dir = work.to_string_lossy().replace('\\', "/");
    let other_dir = other.to_string_lossy().replace('\\', "/");
    // A session of the first member: one turn that went well, then the turn the plan's limit ends.
    let (first, session) = open(&host, "Claude1", &dir);
    assert_eq!(turn(&first, &session, ASKED), format!("Done: {ASKED}"));
    if earlier_limit {
        let ended = prompt(&first, &session, "limit now");
        assert_eq!(ended["stopReason"], "refusal", "{ended}");
        assert!(
            control
                .heard("roles/moved", Duration::from_secs(2))
                .is_none(),
            "a session no role serves does not move"
        );
    }
    bind(&control, "exec", "Claude1", &session, &dir);
    let ended = prompt(&first, &session, "limit now");
    assert_eq!(ended["stopReason"], "refusal", "{ended}");

    // Nothing more is said by the client: the host moves the session and tells the link where it went.
    let Some(moved) = control.heard("roles/moved", WAIT) else {
        let served = control.call(
            "roles/served",
            &json!({"entry": "Claude1", "session": session}),
        );
        panic!(
            "the host tells the link the session moved: told {:?}, served {served:?}, record {:?}",
            control.told.borrow(),
            records(&record)
                .iter()
                .map(|line| json!({"prompt": line["prompt"], "config": line["config"], "session": line["session"]}))
                .collect::<Vec<_>>()
        );
    };
    let params = &moved["params"];
    assert_eq!(params["role"], "exec", "{moved}");
    assert_eq!(
        params["from"],
        json!({"entry": "Claude1", "session": session})
    );
    assert_eq!(params["to"]["entry"], "Claude2", "{moved}");
    let arrived = params["to"]["session"]
        .as_str()
        .expect("a session id")
        .to_owned();
    assert_ne!(arrived, session);
    assert_eq!(params["replayed"], false, "{moved}");
    assert!(params["losses"].is_array(), "{moved}");

    // The turn the limit ended had done no work, so the prompt it ended is what the second entry is given, on its own account, as a
    // prompt of the moved session; the limit's words are not.
    let given = prompts(&record);
    assert!(
        given
            .iter()
            .any(|(text, config)| text == "limit now" && config.contains(&work_dir)),
        "{given:?}"
    );
    assert_eq!(
        given
            .iter()
            .filter(|(_, config)| config.contains(&other_dir))
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>(),
        ["limit now"],
        "{given:?}"
    );

    // What went with it is the conversation without the limit's words.
    let carried = sessions_in(&other);
    assert_eq!(carried.len(), 1, "{carried:?}");
    assert!(
        carried[0].contains(ASKED) && carried[0].contains(&format!("Done: {ASKED}")),
        "{}",
        carried[0]
    );
    assert!(!carried[0].contains(LIMIT_WORDS), "{}", carried[0]);

    // Each turn is recorded under the member that ran it, in the columns of the session store's turns.
    let (one, two) = (
        first_member(&control, "exec"),
        control.call("roles/list", &json!({})).expect("listed")["roles"]
            .as_array()
            .and_then(|roles| roles.iter().find(|each| each["name"] == "exec"))
            .and_then(|role| role["members"][1]["id"].as_str())
            .expect("a second member")
            .to_owned(),
    );
    let left = control.served("Claude1", &session, 1);
    let limited = &left["turns"][0];
    assert_eq!(limited["role"], "exec", "{left}");
    assert_eq!(limited["member"], one, "{left}");
    assert_eq!(limited["limited"], true, "{left}");
    assert_eq!(limited["moved_on"], true, "{left}");
    assert_eq!(left["moves"][0]["to"]["session"], arrived, "{left}");
    let there = control.served("Claude2", &arrived, 1);
    let continued = &there["turns"][0];
    assert_eq!(continued["member"], two, "{there}");
    assert_eq!(continued["moved_from"], one, "{there}");
    assert_eq!(continued["why"], "exhausted", "{there}");

    // A session budget is a limit no other member helps with: the session stays on its member, and its turn is still recorded.
    let (budget, spent) = open(&host, "Codex1", &dir);
    bind(&control, "spend", "Codex1", &spent, &dir);
    let ended = prompt(&budget, &spent, "budget now");
    assert_eq!(ended["stopReason"], "refusal", "{ended}");
    assert!(
        control
            .heard("roles/moved", Duration::from_secs(2))
            .is_none(),
        "a session budget moves nothing"
    );
    assert!(
        !prompts(&record)
            .iter()
            .any(|(text, _)| text == "Carry on, spending."),
        "no continuation was sent"
    );
    let kept = control.served("Codex1", &spent, 1);
    assert_eq!(kept["turns"][0]["limited"], true, "{kept}");
    assert_eq!(kept["turns"][0]["moved_on"], false, "{kept}");
    assert_eq!(kept["turns"][0]["member"], first_member(&control, "spend"));
    let unbound = control
        .call(
            "roles/unbind",
            &json!({"entry": "Codex1", "session": spent}),
        )
        .expect("unbound");
    assert_eq!(unbound["unbound"], true, "still the role's: {unbound}");

    host.stop();
}
