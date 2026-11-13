//! Secret prompts over the link an application inside the host's own process holds, through the C ABI as the desktop app uses it:
//! the application takes the secret the host minted (`canoa_ffi_link_secret`), derives its half of the binding from it, and answers
//! a scripted agent's request sealed. After the person's yes the agent has the value in an environment variable of a command; the
//! agent, the transcript the application was sent, the host's log and everything under the host's directory hold the handle and
//! never the value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use canoa_ffi::exports::{
    LINK_SECRET_LEN, STATUS_BAD_ARGUMENT, STATUS_NO_ENTRY, STATUS_OK, STATUS_UNKNOWN_JOB,
    canoa_ffi_job_free, canoa_ffi_job_read, canoa_ffi_job_start, canoa_ffi_job_stop,
    canoa_ffi_link_close, canoa_ffi_link_open, canoa_ffi_link_read, canoa_ffi_link_secret,
    canoa_ffi_link_send, canoa_ffi_text_free,
};
use coqui_acp::secret::{
    BINDING_LINK, CAPABILITY, Capability, DEFAULT_LIFETIME, IdentityId, Limits, LinkKey, LinkRole,
    Method, Prover, Receiver, RequestParams, RequestReply, ShowDeliver, ShowDone, ShowOpen,
    ShowOpened, Verifier,
};
use data_encoding::{BASE64, BASE64_NOPAD, BASE64URL, BASE64URL_NOPAD, HEXLOWER, HEXUPPER};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(30);
/// Made up, with characters base64 and percent-encoding treat differently, and that a command line prints as they are.
const PASSWORD: &str = "Tr0ub4dor.3/x+y=?qZ_9";
const SCHEMA: &str = r#"{"type":"object","properties":{"user":{"type":"string"},"password":{"type":"string","writeOnly":true}},"required":["user","password"]}"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-ffi-secret-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every form of a value an accident could print.
fn forms(value: &str) -> Vec<(&'static str, String)> {
    let json = serde_json::to_string(value).expect("JSON");
    let percent: String = value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    vec![
        ("raw", value.to_owned()),
        ("base64", BASE64.encode(value.as_bytes())),
        ("base64 unpadded", BASE64_NOPAD.encode(value.as_bytes())),
        ("base64url", BASE64URL.encode(value.as_bytes())),
        (
            "base64url unpadded",
            BASE64URL_NOPAD.encode(value.as_bytes()),
        ),
        ("hex", HEXLOWER.encode(value.as_bytes())),
        ("HEX", HEXUPPER.encode(value.as_bytes())),
        ("percent", percent),
        ("json", json[1..json.len() - 1].to_owned()),
    ]
}

fn leaks(text: &str, value: &str) -> Vec<&'static str> {
    forms(value)
        .into_iter()
        .filter(|(_, form)| text.contains(form.as_str()))
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn the_scan_sees_every_form_it_looks_for() {
    for (name, form) in forms(PASSWORD) {
        let planted = format!("a log line before {form} and after");
        assert!(
            leaks(&planted, PASSWORD).contains(&name),
            "the scan missed {name}"
        );
    }
    assert!(leaks("nothing here", PASSWORD).is_empty());
}

/// Every file under `dir`, as text, with its path.
fn files(dir: &Path, found: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, found);
        } else if let Ok(bytes) = std::fs::read(&path) {
            found.push((
                path.display().to_string(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs()
}

fn start(request: &Value) -> u64 {
    let text = request.to_string();
    let mut job = u64::MAX;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(text.as_ptr(), text.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK, "{request}");
    job
}

/// What the job said since the last call: its output and its log lines.
fn drain(job: u64, log: &mut Vec<String>) -> (Vec<String>, Option<i64>) {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_job_read(job, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value: Value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    let lines = |key: &str| -> Vec<String> {
        value[key]
            .as_array()
            .expect("a list")
            .iter()
            .map(|line| line.as_str().expect("text").to_owned())
            .collect()
    };
    log.extend(lines("err"));
    (lines("out"), value["exit"].as_i64())
}

fn wait_for(job: u64, log: &mut Vec<String>, done: impl Fn(&[String], Option<i64>) -> bool) {
    let deadline = Instant::now() + WAIT;
    let mut out = Vec::new();
    loop {
        let (lines, exit) = drain(job, log);
        out.extend(lines);
        if done(&out, exit) {
            return;
        }
        assert!(Instant::now() < deadline, "waiting: {out:?} {log:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn open_link(job: u64, entry: &str) -> u64 {
    let mut link = u64::MAX;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_link_open(job, entry.as_ptr(), entry.len(), &raw mut link) };
    assert_eq!(status, STATUS_OK);
    link
}

fn send_on(link: u64, message: &Value) {
    let text = message.to_string();
    // SAFETY: the text lives across the call.
    let status = unsafe { canoa_ffi_link_send(link, text.as_ptr(), text.len()) };
    assert_eq!(status, STATUS_OK);
}

fn read_link(link: u64) -> Vec<Value> {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_link_read(link, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value: Value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    value["messages"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|message| serde_json::from_str(message.as_str().expect("text")).expect("ACP"))
        .collect()
}

/// The secret the host of `job` shares with this process.
fn link_secret(job: u64) -> [u8; LINK_SECRET_LEN] {
    let mut out = [0xAA_u8; LINK_SECRET_LEN];
    // SAFETY: `out` is writable for its whole length.
    let status = unsafe { canoa_ffi_link_secret(job, out.as_mut_ptr(), out.len()) };
    assert_eq!(status, STATUS_OK);
    out
}

fn stdio_agent() -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile
        .join("examples")
        .join(format!("stdio_agent{}", std::env::consts::EXE_SUFFIX))
}

/// A command that prints the variable it was given the secret in: the host's scrubbing turns what it prints into the handle.
fn echo_command(variable: &str) -> Value {
    if cfg!(windows) {
        Value::String(format!("echo %{variable}%"))
    } else {
        Value::String(format!("echo \"${variable}\""))
    }
}

/// The application: its half of the link's binding, and what the person will do.
struct App {
    prover: Prover,
    verifier: Verifier,
    host: IdentityId,
    /// What it offers in `initialize`: the bindings it names.
    bindings: Vec<String>,
    typed: VecDeque<(String, Vec<(String, String)>)>,
    decisions: VecDeque<&'static str>,
    cards: Vec<Value>,
    extension: Vec<String>,
    pending: Option<(Receiver, String)>,
    shown: Vec<(String, String)>,
    /// Everything the host sent, as text.
    received: Vec<String>,
    /// Requests of the host's it refused, with why.
    refused: Vec<String>,
}

impl App {
    fn new(secret: &[u8]) -> Self {
        let key = LinkKey::derive(secret).expect("a secret");
        let prover = Prover::linked(&key, LinkRole::App);
        let verifier = Verifier::builder(key.identity(LinkRole::App))
            .link(&key, LinkRole::App)
            .expect("this end")
            .build();
        Self {
            prover,
            verifier,
            host: key.identity(LinkRole::Host),
            bindings: vec![BINDING_LINK.to_owned()],
            typed: VecDeque::new(),
            decisions: VecDeque::new(),
            cards: Vec::new(),
            extension: Vec::new(),
            pending: None,
            shown: Vec::new(),
            received: Vec::new(),
            refused: Vec::new(),
        }
    }

    fn will_type(&mut self, plain: &str, secrets: &[(&str, &str)]) {
        self.typed.push_back((
            plain.to_owned(),
            secrets
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        ));
    }

    /// The answer to one request of the host's: `None` for a message that is none, `Err` for one this app refuses.
    fn answer(&mut self, message: &Value) -> Option<Result<Value, String>> {
        let method = message["method"].as_str()?;
        let params = &message["params"];
        let now = unix_now();
        Some(match method {
            "_coqui_acp/secret_request" => {
                self.extension.push(message.to_string());
                let params = RequestParams::from_value(params).expect("the host's request");
                let Some((plain, secrets)) = self.typed.pop_front() else {
                    return Some(Ok(RequestReply::Decline.to_value()));
                };
                let collector = match self.verifier.collector(
                    &self.host,
                    Method::Request,
                    &params.context,
                    &params.recipient,
                    now,
                ) {
                    Ok(collector) => collector,
                    Err(error) => {
                        self.refused.push(error.to_string());
                        return Some(Err(error.to_string()));
                    }
                };
                let sealed = secrets
                    .iter()
                    .map(|(name, value)| {
                        (name.clone(), collector.seal(name, value).expect("sealed"))
                    })
                    .collect();
                let auth = collector
                    .authenticate(&self.prover, &plain, &sealed, now, DEFAULT_LIFETIME)
                    .expect("bound");
                Ok(RequestReply::Accept {
                    plain,
                    sealed,
                    auth,
                }
                .to_value())
            }
            "_coqui_acp/secret_show_open" => {
                self.extension.push(message.to_string());
                let open = ShowOpen::from_value(params).expect("the host's show");
                let receiver = Receiver::new(
                    Method::Show,
                    &open.context,
                    &Limits::new(),
                    &self.prover,
                    &self.host,
                    now,
                    DEFAULT_LIFETIME,
                )
                .expect("a receiver");
                let recipient = receiver.recipient().clone();
                self.pending = Some((receiver, open.context.values.clone().unwrap_or_default()));
                Ok(ShowOpened::Ready(recipient).to_value())
            }
            "_coqui_acp/secret_show_deliver" => {
                self.extension.push(message.to_string());
                let deliver = ShowDeliver::from_value(params).expect("the host's delivery");
                let (receiver, plain) = self.pending.take().expect("an open step pending");
                let opened = receiver
                    .open_values(
                        &self.verifier,
                        &self.host,
                        &plain,
                        &deliver.sealed,
                        &deliver.auth,
                        now,
                    )
                    .expect("values from the host this app shares the link with");
                for (name, value) in &opened.secrets {
                    self.shown
                        .push((name.clone(), value.expose_str().to_owned()));
                }
                Ok(ShowDone::Shown.to_value())
            }
            "session/request_permission" => {
                self.cards.push(params.clone());
                let choice = self.decisions.pop_front().unwrap_or("reject");
                Ok(json!({"outcome": {"outcome": "selected", "optionId": choice}}))
            }
            _ => return None,
        })
    }
}

/// An ACP conversation on one link, in which the app answers what the host asks of it.
struct Wire {
    link: u64,
    next: u64,
}

impl Wire {
    fn request(
        &mut self,
        app: &mut App,
        method: &str,
        params: Value,
    ) -> Result<(Value, String), Value> {
        self.next += 1;
        let id = self.next;
        send_on(
            self.link,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        let mut said = String::new();
        let until = Instant::now() + WAIT;
        loop {
            for message in read_link(self.link) {
                app.received.push(message.to_string());
                if message.get("method").is_some() {
                    if message.get("id").is_some() {
                        let reply = match app.answer(&message) {
                            Some(Ok(result)) => {
                                json!({"jsonrpc": "2.0", "id": message["id"], "result": result})
                            }
                            Some(Err(why)) => json!({"jsonrpc": "2.0", "id": message["id"],
                                "error": {"code": -32602, "message": why}}),
                            None => json!({"jsonrpc": "2.0", "id": message["id"],
                                "error": {"code": -32601, "message": "no such method"}}),
                        };
                        send_on(self.link, &reply);
                    } else if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
                    {
                        said.push_str(
                            message["params"]["update"]["content"]["text"]
                                .as_str()
                                .unwrap_or(""),
                        );
                    }
                } else if message["id"] == id {
                    return match message.get("error") {
                        Some(error) => Err(error.clone()),
                        None => Ok((message["result"].clone(), said)),
                    };
                }
            }
            assert!(Instant::now() < until, "no answer to {method}");
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn open(&mut self, app: &mut App, work: &str) -> String {
        let mut offer = Capability::all();
        offer.bindings = app.bindings.clone();
        self.request(
            app,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {"_meta": {CAPABILITY: offer.to_value()}}}),
        )
        .expect("initialize");
        let (result, _) = self
            .request(app, "session/new", json!({"cwd": work, "mcpServers": []}))
            .expect("session/new");
        result["sessionId"].as_str().expect("a session").to_owned()
    }

    fn prompt(&mut self, app: &mut App, session: &str, text: &str) -> String {
        let (result, said) = self
            .request(
                app,
                "session/prompt",
                json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
            )
            .expect("session/prompt");
        assert_eq!(result["stopReason"], "end_turn", "{result}");
        said
    }
}

fn json_of(said: &str) -> Value {
    serde_json::from_str(said).unwrap_or_else(|_| panic!("a tool answer that is JSON: {said}"))
}

/// A host in this process serving one scripted agent that starts in `modes`' first; `body` runs on a link of it. Returns what the
/// body returned, the host's log, and every file the host and the agent left.
fn with_link<R>(
    modes: &str,
    body: impl FnOnce(&mut Wire, &mut App, u64, &str) -> R,
    secret_of_app: impl FnOnce([u8; LINK_SECRET_LEN]) -> Vec<u8>,
) -> (R, Vec<String>, Vec<(String, String)>) {
    let agent = stdio_agent();
    assert!(
        agent.exists(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    let scratch = Scratch::new();
    let state = scratch.0.join("agent-state.json");
    let config = scratch.0.join("canoa.cfg");
    let quoted = |path: &Path| serde_json::to_string(&path.to_string_lossy()).expect("quoted");
    std::fs::write(
        &config,
        format!(
            "[host.agents.Scripted]\nprogram = {}\nargs = [\"--scripted\", \"--mcp-http\", \"--modes={modes}\", \"--state={}\"]\nlisten = \"127.0.0.1:0\"\n",
            quoted(&agent),
            state.to_string_lossy().replace('\\', "/"),
        ),
    )
    .expect("written");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).expect("a work directory");
    let job = start(&json!({
        "args": ["serve", "--until-stdin-closes", "--config", config],
        "home": scratch.0.join("home"),
        "holder": "a test",
    }));
    let mut log = Vec::new();
    wait_for(job, &mut log, |out, _| {
        out.iter().any(|line| line.contains("ready"))
    });
    let secret = secret_of_app(link_secret(job));
    let mut app = App::new(&secret);
    let link = open_link(job, "Scripted");
    let mut wire = Wire { link, next: 0 };
    let result = body(&mut wire, &mut app, job, &work.to_string_lossy());
    // Closing the link ends the agent, stopping the job ends the host.
    assert_eq!(canoa_ffi_link_close(link), STATUS_OK);
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    wait_for(job, &mut log, |_, exit| exit.is_some());
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
    let mut found = Vec::new();
    files(&scratch.0, &mut found);
    let mut transcript = app.received.clone();
    transcript.extend(app.extension.clone());
    (result, [log, transcript].concat(), found)
}

#[test]
fn the_desktop_app_with_its_embedded_host_runs_the_scripted_secret_flow_sealed_and_with_only_the_handle_left_behind()
 {
    let ((shown, cards, extension), seen, found) = with_link(
        "default,bypassPermissions",
        |wire, app, _job, work| {
            let session = wire.open(app, work);
            app.will_type("{\"user\":\"ana\"}", &[("password", PASSWORD)]);
            let asked = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_request {}",
                    json!({"message": "Registry login", "schema": SCHEMA})
                ),
            );
            let answer = json_of(&asked);
            assert_eq!(answer["status"], "accepted", "{asked}");
            assert_eq!(answer["plain"]["user"], "ana");
            let handle = answer["handles"]["password"]
                .as_str()
                .expect("a handle")
                .to_owned();
            assert!(handle.starts_with("secret_"), "{handle}");
            assert!(!asked.contains(PASSWORD), "the agent never held the value");

            // A use asks the person, in the mode the session is in; allowed once, it runs with the value in an environment variable.
            app.decisions.push_back("allow-once");
            let used = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_use {}",
                    json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
                ),
            );
            let ran = json_of(&used);
            assert_eq!(ran["status"], "exited", "{used}");
            assert_eq!(ran["exit_code"], 0, "{used}");
            assert!(
                ran["stdout"]
                    .as_str()
                    .is_some_and(|out| out.contains(&handle)),
                "what the command printed came back as the handle: {used}"
            );
            assert_eq!(app.cards.len(), 1, "the person was asked once");

            // Allowed once: the same action is asked again, and a no runs nothing.
            app.decisions.push_back("reject");
            let denied = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_use {}",
                    json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
                ),
            );
            assert!(!denied.contains("exited"), "{denied}");
            assert_eq!(app.cards.len(), 2, "allow once is asked each time");

            // The host shows the person a value the other way: sealed to the app's key for the show.
            let shown = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_show {}",
                    json!({"message": "Your registry password",
                        "schema": r#"{"type":"object","properties":{"password":{"type":"string","writeOnly":true}}}"#,
                        "secrets": {"password": {"handle": handle}}})
                ),
            );
            assert_eq!(json_of(&shown)["status"], "shown", "{shown}");
            assert!(!shown.contains(PASSWORD), "{shown}");
            (
                app.shown.clone(),
                app.cards.iter().map(Value::to_string).collect::<Vec<_>>(),
                app.extension.clone(),
            )
        },
        |secret| secret.to_vec(),
    );
    assert_eq!(shown, [("password".to_owned(), PASSWORD.to_owned())]);
    assert!(
        cards
            .iter()
            .all(|card| card.contains("REGISTRY_TOKEN") && !card.contains(PASSWORD)),
        "{cards:?}"
    );
    assert!(extension.iter().any(|text| text.contains("secret_request")));
    assert!(
        extension.iter().all(|text| !text.contains(PASSWORD)),
        "the extension's messages carry keys and sealed values only"
    );
    // The transcript the app was sent, the host's log, the agent's own record and everything under the host's directory.
    assert!(
        found
            .iter()
            .any(|(path, _)| path.ends_with("agent-state.json")),
        "the agent's record is scanned"
    );
    assert!(
        seen.iter().any(|line| line.contains("a secret was kept")),
        "the host's log is scanned"
    );
    for (path, text) in &found {
        assert!(
            leaks(text, PASSWORD).is_empty(),
            "the value is in {path}: {:?}",
            leaks(text, PASSWORD)
        );
    }
    for text in &seen {
        assert!(
            leaks(text, PASSWORD).is_empty(),
            "the value reached a log or the transcript: {text}"
        );
    }
}

#[test]
fn an_app_with_another_links_secret_is_refused_and_nothing_is_sealed() {
    let (asked, ..) = with_link(
        "default",
        |wire, app, _job, work| {
            let session = wire.open(app, work);
            app.will_type("{\"user\":\"ana\"}", &[("password", PASSWORD)]);
            let asked = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_request {}",
                    json!({"message": "Registry login", "schema": SCHEMA})
                ),
            );
            assert!(
                !app.refused.is_empty(),
                "the app refused a key not bound under its secret"
            );
            assert!(app.shown.is_empty());
            asked
        },
        |_| vec![0x5A; LINK_SECRET_LEN],
    );
    assert!(!asked.contains("accepted"), "{asked}");
    assert!(!asked.contains(PASSWORD));
}

#[test]
fn an_app_that_does_not_offer_the_link_binding_gets_no_tools() {
    with_link(
        "default",
        |wire, app, _job, work| {
            // Another binding is the offer of a link with a relay, and the in-process link never takes it.
            app.bindings = vec!["ecdsa-p256-sha256".to_owned()];
            let session = wire.open(app, work);
            let said = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_request {}",
                    json!({"message": "x", "schema": SCHEMA})
                ),
            );
            assert!(said.contains("no secret tools"), "{said}");
            assert!(app.extension.is_empty());
        },
        |secret| secret.to_vec(),
    );
}

#[test]
fn the_secret_is_the_hosts_to_give_and_a_bad_call_gets_none_of_it() {
    // A job that is no `serve` yet has no secret to give.
    let scratch = Scratch::new();
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(&config, "").expect("written");
    let job = start(&json!({
        "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", config],
        "home": scratch.0.join("home"),
        "holder": "a test",
    }));
    let mut log = Vec::new();
    wait_for(job, &mut log, |out, _| {
        out.iter().any(|line| line.contains("ready"))
    });
    let first = link_secret(job);
    assert_ne!(first, [0; LINK_SECRET_LEN]);
    assert_eq!(
        first,
        link_secret(job),
        "one secret for the life of the host"
    );
    // A length that is not the secret's writes nothing at all.
    let mut out = [7_u8; 16];
    // SAFETY: `out` is writable for its length.
    let status = unsafe { canoa_ffi_link_secret(job, out.as_mut_ptr(), out.len()) };
    assert_eq!(status, STATUS_BAD_ARGUMENT, "only 32 bytes are asked for");
    assert_eq!(out, [7; 16], "untouched");
    // SAFETY: a null out is what is being refused.
    let status = unsafe { canoa_ffi_link_secret(job, core::ptr::null_mut(), 32) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);
    assert_eq!(
        secret_status(u64::MAX),
        (STATUS_UNKNOWN_JOB, [0; LINK_SECRET_LEN])
    );

    // A job that is no `serve` has no secret to give, and what it is given to write is zeroed.
    let other = start(&json!({"args": ["--version"]}));
    wait_for(other, &mut log, |_, exit| exit.is_some());
    assert_eq!(
        secret_status(other),
        (STATUS_NO_ENTRY, [0; LINK_SECRET_LEN])
    );
    assert_eq!(canoa_ffi_job_free(other), STATUS_OK);

    // A stopped host gives none either.
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    wait_for(job, &mut log, |_, exit| exit.is_some());
    assert_eq!(secret_status(job), (STATUS_NO_ENTRY, [0; LINK_SECRET_LEN]));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}

/// The status of asking `job` for the secret into a buffer that held other bytes, and what the buffer holds after.
fn secret_status(job: u64) -> (i32, [u8; LINK_SECRET_LEN]) {
    let mut out = [7_u8; LINK_SECRET_LEN];
    // SAFETY: `out` is writable for its length.
    let status = unsafe { canoa_ffi_link_secret(job, out.as_mut_ptr(), out.len()) };
    (status, out)
}

/// Every form of the link's secret an accident could print.
fn secret_forms(secret: &[u8]) -> Vec<(&'static str, String)> {
    vec![
        ("hex", HEXLOWER.encode(secret)),
        ("HEX", HEXUPPER.encode(secret)),
        ("base64", BASE64.encode(secret)),
        ("base64 unpadded", BASE64_NOPAD.encode(secret)),
        ("base64url", BASE64URL.encode(secret)),
        ("base64url unpadded", BASE64URL_NOPAD.encode(secret)),
    ]
}

fn secret_leaks(text: &str, secret: &[u8]) -> Vec<&'static str> {
    secret_forms(secret)
        .into_iter()
        .filter(|(_, form)| text.contains(form.as_str()))
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn the_link_secret_reaches_no_log_file_transcript_or_agent_environment() {
    let planted = [0x5C_u8; LINK_SECRET_LEN];
    for (name, form) in secret_forms(&planted) {
        assert!(
            secret_leaks(&format!("before {form} after"), &planted).contains(&name),
            "the scan missed {name}"
        );
    }
    let ((secret, printed), seen, found) = with_link(
        "default",
        |wire, app, job, work| {
            let secret = link_secret(job);
            let session = wire.open(app, work);
            app.will_type("{\"user\":\"ana\"}", &[("password", PASSWORD)]);
            let asked = json_of(&wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_request {}",
                    json!({"message": "Registry login", "schema": SCHEMA})
                ),
            ));
            let handle = asked["handles"]["password"]
                .as_str()
                .expect("a handle")
                .to_owned();
            // The command prints its whole environment: what the host started the agent's command with.
            app.decisions.push_back("allow-once");
            let printed = wire.prompt(
                app,
                &session,
                &format!(
                    "Secret secret_use {}",
                    json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN",
                        "command": if cfg!(windows) { "set" } else { "env" }}})
                ),
            );
            (secret, printed)
        },
        |secret| secret.to_vec(),
    );
    assert!(
        printed.to_ascii_uppercase().contains("PATH"),
        "the command printed its environment: {printed}"
    );
    assert!(secret.iter().any(|byte| *byte != 0));
    assert!(
        secret_leaks(&printed, &secret).is_empty(),
        "the secret is in the command's environment"
    );
    for (path, text) in &found {
        assert!(
            secret_leaks(text, &secret).is_empty(),
            "the link secret is in {path}"
        );
    }
    for text in &seen {
        assert!(
            secret_leaks(text, &secret).is_empty(),
            "the link secret is in a log line or the transcript: {text}"
        );
    }
}
