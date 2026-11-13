//! Secret prompts through the real host: a scripted agent asks for a secret, the person's app (played here with the extension's
//! own sealing and bindings, as the app does) answers sealed, and after the person's yes the agent has the value written into an
//! environment variable for a command. The agent, its transcript, the host's log and everything under the host's directory hold
//! the handle and never the value, in any of the forms an accident could print. The suite runs once per proof method
//! (`CANOA_TEST_METHOD`): the binding is an ECDSA signature under WebCrypto and a Diffie-Hellman MAC under Noise.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use coqui_acp::secret::{
    CAPABILITY, Capability, DEFAULT_LIFETIME, IdentityId, Limits, Method, Prover, Receiver,
    RequestParams, RequestReply, ShowDeliver, ShowDone, ShowOpen, ShowOpened, Verifier,
    unprotected,
};
use coqui_agentic_peer::auth::{self, Identity, Sealed};
use coqui_agentic_peer::home::identity_file;
use coqui_transport::Channel as _;
use coqui_transport_web::Dialer as _;
use coqui_transport_web::websocket::{self, Frames};
use coqui_transport_web::{Kind, Limits as WebLimits};
use data_encoding::{BASE64, BASE64_NOPAD, BASE64URL, BASE64URL_NOPAD, HEXLOWER, HEXUPPER};
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, cwd, example, key, method, quoted};

/// Made up, with characters base64 and percent-encoding treat differently, and that a command line prints as they are.
const PASSWORD: &str = "Tr0ub4dor.3/x+y=?qZ_9";
const MAX: usize = 16 * 1024 * 1024;

/// Every form of a value an accident could print, written here on its own.
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

/// The forms of `value` found in `text`.
fn leaks(text: &str, value: &str) -> Vec<&'static str> {
    forms(value)
        .into_iter()
        .filter(|(_, form)| text.contains(form.as_str()))
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn the_scan_sees_every_form_it_looks_for() {
    // A scan that finds nothing is believed only because it is seen to find a planted value.
    for (name, form) in forms(PASSWORD) {
        let planted = format!("a log line before {form} and after");
        assert!(
            leaks(&planted, PASSWORD).contains(&name),
            "the scan missed {name}"
        );
    }
    assert!(leaks("nothing here", PASSWORD).is_empty());
}

#[test]
fn the_scan_of_a_directory_sees_a_value_a_file_holds_in_any_form() {
    let scratch = Scratch::new();
    std::fs::create_dir_all(scratch.0.join("deep")).expect("a directory");
    std::fs::write(
        scratch.0.join("deep").join("history.jsonl"),
        format!(
            "{{\"text\":\"{}\"}}",
            BASE64URL_NOPAD.encode(PASSWORD.as_bytes())
        ),
    )
    .expect("written");
    let mut found = Vec::new();
    files(&scratch.0, &mut found);
    assert_eq!(found.len(), 1);
    assert!(leaks(&found[0].1, PASSWORD).contains(&"base64url unpadded"));
}

/// Every file under `dir`, as text, with its path: what a host, an agent and a history leave on disk.
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

/// The person's app: the identity it proves with, the host it is paired with, and what the person will do.
struct App {
    prover: Prover,
    verifier: Verifier,
    host: IdentityId,
    /// Whether it says it takes the extension in `initialize`.
    offers: bool,
    typed: VecDeque<(String, Vec<(String, String)>)>,
    /// The option id the person picks on each permission card, in order.
    decisions: VecDeque<&'static str>,
    /// Every permission card the person was shown, as the host sent it.
    cards: Vec<Value>,
    /// What the host sent the app that carries an extension method, as text.
    extension: Vec<String>,
    pending: Option<(Receiver, String)>,
    shown: Vec<(String, String)>,
    /// Everything the host sent, as text.
    received: Vec<String>,
}

fn der_of(identity: &Identity) -> Vec<u8> {
    let pem = match identity {
        Identity::WebCrypto(identity) => identity.to_pkcs8_pem().expect("a PEM"),
        Identity::Noise(identity) => identity.to_pkcs8_pem().expect("a PEM"),
        _ => unreachable!("a method of this suite"),
    };
    coqui_peer::pem::decode(&pem).expect("PEM").bytes
}

impl App {
    fn new(client: &Identity, host: &Identity) -> Self {
        let der = der_of(client);
        let host_spki = host.key_material().bytes;
        let (prover, verifier) = match client {
            Identity::WebCrypto(_) => {
                let prover = unprotected::ecdsa_p256_pkcs8(&der).expect("a P-256 key");
                let verifier = Verifier::builder(prover.id())
                    .trust(IdentityId::of(&host_spki))
                    .build();
                (prover, verifier)
            }
            _ => {
                let prover = unprotected::x25519_pkcs8(&der, &host_spki).expect("an X25519 key");
                let (spki, agree) = unprotected::x25519_agreement(&der[16..]).expect("a key");
                let verifier = Verifier::builder(prover.id())
                    .trust(IdentityId::of(&host_spki))
                    .agreement(&spki, agree)
                    .expect("this identity")
                    .build();
                (prover, verifier)
            }
        };
        Self {
            prover,
            verifier,
            host: IdentityId::of(&host_spki),
            offers: true,
            typed: VecDeque::new(),
            decisions: VecDeque::new(),
            cards: Vec::new(),
            extension: Vec::new(),
            pending: None,
            shown: Vec::new(),
            received: Vec::new(),
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

    /// The answer to one request of the host's, or `None` for a message that is none.
    fn answer(&mut self, message: &Value) -> Option<Value> {
        let method = message["method"].as_str()?;
        let params = &message["params"];
        let now = unix_now();
        match method {
            "_coqui_acp/secret_request" => {
                self.extension.push(message.to_string());
                let params = RequestParams::from_value(params).expect("the host's request");
                let Some((plain, secrets)) = self.typed.pop_front() else {
                    return Some(RequestReply::Decline.to_value());
                };
                let collector = self
                    .verifier
                    .collector(
                        &self.host,
                        Method::Request,
                        &params.context,
                        &params.recipient,
                        now,
                    )
                    .expect("a key bound to the host this app is paired with");
                let sealed = secrets
                    .iter()
                    .map(|(name, value)| {
                        (name.clone(), collector.seal(name, value).expect("sealed"))
                    })
                    .collect();
                let auth = collector
                    .authenticate(&self.prover, &plain, &sealed, now, DEFAULT_LIFETIME)
                    .expect("bound");
                Some(
                    RequestReply::Accept {
                        plain,
                        sealed,
                        auth,
                    }
                    .to_value(),
                )
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
                Some(ShowOpened::Ready(recipient).to_value())
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
                    .expect("values from the host this app is paired with");
                for (name, value) in &opened.secrets {
                    self.shown
                        .push((name.clone(), value.expose_str().to_owned()));
                }
                Some(ShowDone::Shown.to_value())
            }
            "session/request_permission" => {
                self.cards.push(params.clone());
                let choice = self.decisions.pop_front().unwrap_or("reject");
                Some(json!({"outcome": {"outcome": "selected", "optionId": choice}}))
            }
            _ => None,
        }
    }
}

/// An ACP conversation in a sealed session, in which the app answers what the host asks of it.
struct Wire<'a, C: coqui_transport::Channel> {
    channel: &'a Sealed<'a, C>,
    next: u64,
}

impl<C: coqui_transport::Channel> Wire<'_, C> {
    fn message(&self, wait: Duration) -> Value {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "nothing arrived in time");
            match self.channel.receive(Some(left)) {
                Ok(Some(message)) => return serde_json::from_slice(&message).expect("JSON"),
                Ok(None) => panic!("the host closed the link"),
                Err(coqui_transport::Failure::Timeout) => {}
                Err(failure) => panic!("{failure}"),
            }
        }
    }

    /// A request, answered as `app` answers what the host asks meanwhile. The result, and the text of every update.
    fn request(
        &mut self,
        app: &mut App,
        method: &str,
        params: Value,
    ) -> Result<(Value, String), Value> {
        self.next += 1;
        let id = self.next;
        self.channel
            .send(
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
                    .to_string()
                    .as_bytes(),
            )
            .expect("sent");
        let mut said = String::new();
        let until = Instant::now() + WAIT;
        loop {
            let message = self.message(until.saturating_duration_since(Instant::now()));
            app.received.push(message.to_string());
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    let result = app.answer(&message);
                    let reply = match result {
                        Some(result) => {
                            json!({"jsonrpc": "2.0", "id": message["id"], "result": result})
                        }
                        None => json!({"jsonrpc": "2.0", "id": message["id"],
                            "error": {"code": -32601, "message": "no such method"}}),
                    };
                    self.channel
                        .send(reply.to_string().as_bytes())
                        .expect("sent");
                } else if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
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
    }

    fn open(&mut self, app: &mut App, work: &str) -> String {
        let capabilities = if app.offers {
            json!({"_meta": {CAPABILITY: Capability::all().to_value()}})
        } else {
            json!({})
        };
        self.request(
            app,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": capabilities}),
        )
        .expect("initialize");
        let (result, _) = self
            .request(app, "session/new", json!({"cwd": work, "mcpServers": []}))
            .expect("session/new");
        result["sessionId"].as_str().expect("a session").to_owned()
    }

    /// What the agent said answering `text`.
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

/// A host serving one scripted agent that starts in `modes`' first, a client of it paired and proved, and `body` run on it.
fn with_app<R>(
    modes: &str,
    body: impl FnOnce(&mut Wire<'_, websocket::Channel>, &mut App, &Host, &str) -> R,
) -> (R, Vec<(String, String)>) {
    with_agent(modes, true, body)
}

/// As [`with_app`], the agent taking MCP servers over HTTP or only over stdio.
fn with_agent<R>(
    modes: &str,
    http: bool,
    body: impl FnOnce(&mut Wire<'_, websocket::Channel>, &mut App, &Host, &str) -> R,
) -> (R, Vec<(String, String)>) {
    let mcp = if http { "\"--mcp-http\", " } else { "" };
    let scratch = Scratch::new();
    let state = scratch.0.join("agent-state.json");
    let config = format!(
        "[host.agents.Scripted]\nprogram = {}\nargs = [\"--scripted\", {mcp}\"--modes={modes}\", \"--state={}\"]\nlisten = \"127.0.0.1:0\"\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        state.to_string_lossy().replace('\\', "/"),
    );
    let (mut host, _log) = Host::start_logged(scratch, &config, 1, &[]);
    let client = key();
    host.pair(&client);
    let host_key = Identity::read(&host.home().join(identity_file(method())))
        .expect("the host's key reads")
        .expect("the host has a key");
    let mut app = App::new(&client, &host_key);
    let work = cwd(&host.scratch);
    let url = url::Url::parse(host.url("Scripted")).expect("a url");
    let channel = websocket::Dialer::builder()
        .frames(Frames::Binary)
        .limits({
            let mut limits = WebLimits::default();
            limits.max_message = method().record_length(MAX);
            limits
        })
        .build()
        .expect("a dialer")
        .dial(Kind::WebSocket, &url, WAIT)
        .expect("dialed");
    let proof = auth::Proof::builder(client)
        .responders(auth::Trust::pinned([host.peer]))
        .build()
        .expect("a proof");
    let established =
        auth::prove_out(&proof, &channel, WAIT, method().record_length(MAX)).expect("proved");
    let sealed = Sealed::new(&channel, established.session);
    let mut wire = Wire {
        channel: &sealed,
        next: 0,
    };
    let result = body(&mut wire, &mut app, &host, &work);
    sealed.close();
    channel.close();
    // What the host wrote while it was serving is read whole, so the host is stopped first.
    host.kill();
    let mut found = Vec::new();
    files(&host.scratch.0, &mut found);
    (result, found)
}

const SCHEMA: &str = r#"{"type":"object","properties":{"user":{"type":"string"},"password":{"type":"string","writeOnly":true}},"required":["user","password"]}"#;

fn json_of(said: &str) -> Value {
    serde_json::from_str(said).unwrap_or_else(|_| panic!("a tool answer that is JSON: {said}"))
}

/// A command that prints the variable it was given the secret in: the host's scrubbing turns what it prints into the handle.
fn echo_command(variable: &str) -> Value {
    if cfg!(windows) {
        Value::String(format!("echo %{variable}%"))
    } else {
        Value::String(format!("echo \"${variable}\""))
    }
}

#[test]
fn a_scripted_agent_asks_for_a_secret_and_has_it_written_into_a_command_after_the_persons_yes() {
    let (seen, found) = with_app("default,bypassPermissions", |wire, app, host, work| {
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
        // Only the handle comes back: the agent never held the value.
        assert!(!asked.contains(PASSWORD));

        // A use asks the person, in the mode the session is in.
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
        // The command had the value: what it printed came back as the handle.
        assert!(
            ran["stdout"]
                .as_str()
                .is_some_and(|out| out.contains(&handle)),
            "{used}"
        );
        assert_eq!(app.cards.len(), 1, "the person was asked once");
        let card = app.cards[0].to_string();
        assert!(
            card.contains("REGISTRY_TOKEN") && card.contains("belong to the agent"),
            "{card}"
        );
        assert!(!card.contains(PASSWORD));

        // A second use of the same action is asked again unless the person said "for the session".
        app.decisions.push_back("allow-session");
        let again = json_of(&wire.prompt(
            app,
            &session,
            &format!(
                "Secret secret_use {}",
                json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
            ),
        ));
        assert_eq!(again["status"], "exited");
        let third = json_of(&wire.prompt(
            app,
            &session,
            &format!(
                "Secret secret_use {}",
                json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
            ),
        ));
        assert_eq!(third["status"], "exited");
        assert_eq!(app.cards.len(), 2, "allow for the session is asked once");

        // The extension's messages reached the app and carry only sealed values and keys.
        assert!(
            app.extension
                .iter()
                .any(|text| text.contains("secret_request"))
        );
        assert!(app.extension.iter().all(|text| !text.contains(PASSWORD)));
        // The person said no: nothing runs.
        app.decisions.push_back("reject");
        let denied = wire.prompt(
            app,
            &session,
            &format!(
                "Secret secret_use {}",
                json!({"handle": handle, "env": {"name": "OTHER_TOKEN", "command": echo_command("OTHER_TOKEN")}})
            ),
        );
        assert!(denied.contains("error"), "{denied}");
        assert!(!denied.contains("exited"), "{denied}");
        let _ = host;
        app.received.clone()
    });
    // The transcript the app was sent, the agent's own record, the host's log and everything it kept on disk.
    assert!(
        found.iter().any(|(path, _)| path.ends_with("host.log")),
        "the log is scanned"
    );
    assert!(
        found
            .iter()
            .any(|(path, _)| path.ends_with("agent-state.json")),
        "the agent's record is scanned"
    );
    for (path, text) in found {
        assert!(
            leaks(&text, PASSWORD).is_empty(),
            "the value is in {path}: {:?}",
            leaks(&text, PASSWORD)
        );
    }
    for text in seen {
        assert!(
            leaks(&text, PASSWORD).is_empty(),
            "the value reached the app's transcript: {text}"
        );
    }
}

#[test]
fn in_full_access_a_use_is_approved_without_asking() {
    let (_, _found) = with_app("default,bypassPermissions", |wire, app, _host, work| {
        let session = wire.open(app, work);
        // The person chooses it: only that raises a session to the mode that asks for nothing.
        wire.request(
            app,
            "session/set_config_option",
            json!({"sessionId": session, "configId": "mode", "value": "bypassPermissions"}),
        )
        .expect("the mode is set");
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
        let used = json_of(&wire.prompt(
            app,
            &session,
            &format!(
                "Secret secret_use {}",
                json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
            ),
        ));
        assert_eq!(used["status"], "exited", "{used}");
        assert!(
            app.cards.is_empty(),
            "the mode approves, so the person is not asked"
        );
    });
}

#[test]
fn a_secret_shown_reaches_only_the_app_that_asked_and_the_agent_hears_only_that_it_was_shown() {
    let (seen, found) = with_app("default,bypassPermissions", |wire, app, _host, work| {
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
        assert_eq!(
            app.shown,
            [("password".to_owned(), PASSWORD.to_owned())],
            "the person saw the value"
        );
        assert!(!shown.contains(PASSWORD));
        app.received.clone()
    });
    for (path, text) in found {
        assert!(leaks(&text, PASSWORD).is_empty(), "the value is in {path}");
    }
    for text in seen {
        assert!(
            leaks(&text, PASSWORD).is_empty(),
            "the value reached the app's transcript: {text}"
        );
    }
}

#[test]
fn an_app_that_did_not_offer_the_extension_gets_no_tools() {
    let (_, _found) = with_app("default", |wire, app, _host, work| {
        app.offers = false;
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
    });
}

#[test]
fn an_agent_that_takes_only_stdio_gets_the_tools_through_the_bridge() {
    let (seen, found) = with_agent("default", false, |wire, app, _host, work| {
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
        assert_eq!(asked["status"], "accepted", "{asked}");
        let handle = asked["handles"]["password"]
            .as_str()
            .expect("a handle")
            .to_owned();
        app.decisions.push_back("allow-once");
        let used = json_of(&wire.prompt(
            app,
            &session,
            &format!(
                "Secret secret_use {}",
                json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
            ),
        ));
        assert_eq!(used["status"], "exited", "{used}");
        assert!(
            used["stdout"]
                .as_str()
                .is_some_and(|out| out.contains(&handle)),
            "{used}"
        );
        app.received.clone()
    });
    for (path, text) in found {
        assert!(leaks(&text, PASSWORD).is_empty(), "the value is in {path}");
    }
    for text in seen {
        assert!(
            leaks(&text, PASSWORD).is_empty(),
            "the value reached the app's transcript"
        );
    }
}

/// A secret request answered and a `secret_use` of its handle, in the session `session`: the card the person was shown, if any.
fn use_of_a_secret<C: coqui_transport::Channel>(
    wire: &mut Wire<'_, C>,
    app: &mut App,
    session: &str,
) -> String {
    app.will_type("{\"user\":\"ana\"}", &[("password", PASSWORD)]);
    let asked = json_of(&wire.prompt(
        app,
        session,
        &format!(
            "Secret secret_request {}",
            json!({"message": "Registry login", "schema": SCHEMA})
        ),
    ));
    let handle = asked["handles"]["password"]
        .as_str()
        .expect("a handle")
        .to_owned();
    app.decisions.push_back("reject");
    wire.prompt(
        app,
        session,
        &format!(
            "Secret secret_use {}",
            json!({"handle": handle, "env": {"name": "REGISTRY_TOKEN", "command": echo_command("REGISTRY_TOKEN")}})
        ),
    )
}

#[test]
fn an_agent_that_starts_in_full_access_has_the_person_asked() {
    let (_, _found) = with_app("bypassPermissions,default", |wire, app, _host, work| {
        let session = wire.open(app, work);
        let used = use_of_a_secret(wire, app, &session);
        assert_eq!(app.cards.len(), 1, "the person was asked: {used}");
        assert!(
            used.contains("did not approve"),
            "the person said no: {used}"
        );
    });
}

#[test]
fn an_agents_own_report_cannot_put_its_session_in_full_access() {
    for report in [
        // A mode the session never listed, as a notice.
        json!({"sessionUpdate": "current_mode_update", "currentModeId": "yolo"}),
        // One it listed, which the person never chose.
        json!({"sessionUpdate": "current_mode_update", "currentModeId": "bypassPermissions"}),
        json!({"sessionUpdate": "config_option_update", "configOptions": [{"id": "mode", "name": "Mode", "category": "mode",
            "type": "select", "currentValue": "full-access", "options": [{"value": "full-access", "name": "Full access"}]}]}),
    ] {
        let (_, _found) = with_app("default,bypassPermissions", |wire, app, _host, work| {
            let session = wire.open(app, work);
            let line = json!({"jsonrpc": "2.0", "method": "session/update",
                "params": {"sessionId": session, "update": report}});
            assert_eq!(wire.prompt(app, &session, &format!("Raw {line}")), "sent");
            let used = use_of_a_secret(wire, app, &session);
            assert_eq!(
                app.cards.len(),
                1,
                "the person was asked after {report}: {used}"
            );
        });
    }
}

#[test]
fn an_agents_own_request_in_the_hosts_namespace_never_reaches_the_app_in_any_spelling() {
    let (_, _found) = with_app("default", |wire, app, _host, work| {
        let session = wire.open(app, work);
        for method in [
            "_coqui_acp/secret_show_open",
            r"_coqui_acp\/secret_show_open",
            r"\u005fcoqui_acp/secret_show_open",
            r"_canoa\/profile\/approve",
            "_canoa/profile/approve",
        ] {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":"forged","method":"{method}","params":{{"sessionId":"{session}","requestId":"x"}}}}"#
            );
            assert_eq!(wire.prompt(app, &session, &format!("Raw {line}")), "sent");
        }
        // A plain host-only notification, and lines this host cannot read, which a lenient reader would take for one.
        let deep = format!(
            r#"{{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{}1{}}}"#,
            "[".repeat(200),
            "]".repeat(200)
        );
        for line in [
            r#"{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{"sessionId":"x"}}"#
                .to_owned(),
            r#"{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{"pad":"\ud800"}}"#
                .to_owned(),
            r#"{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{"n":1e400}}"#.to_owned(),
            deep,
        ] {
            assert_eq!(wire.prompt(app, &session, &format!("Raw {line}")), "sent");
        }
        assert!(
            app.extension.is_empty(),
            "the app was asked: {:?}",
            app.extension
        );
        let heard = app.received.join("\n");
        assert!(
            !heard.contains("delegate/event"),
            "the client saw the agent's own notification"
        );
        assert!(
            !heard.contains("secret_show_open") && !heard.contains("profile/approve"),
            "{heard}"
        );
    });
}

/// The command line a running process was started with, as the operating system lists it to any user.
fn command_line(pid: u32) -> String {
    if cfg!(windows) {
        let listed = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine"),
            ])
            .output()
            .expect("powershell runs");
        String::from_utf8_lossy(&listed.stdout).trim().to_owned()
    } else {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .replace(char::from(0), " ")
                    .trim()
                    .to_owned()
            })
            .unwrap_or_default()
    }
}

#[test]
fn the_secret_tools_bearer_is_on_no_command_line_of_the_bridge() {
    let (_, _found) = with_agent("default", false, |wire, app, _host, work| {
        let session = wire.open(app, work);
        let said = wire.prompt(app, &session, "SecretBridge");
        let (pid, line) = said
            .strip_prefix("pid=")
            .and_then(|rest| rest.split_once(" command="))
            .unwrap_or_else(|| panic!("a bridge: {said}"));
        let token = wire
            .prompt(app, &session, "SecretBearer")
            .split(' ')
            .nth(1)
            .expect("a token")
            .to_owned();
        assert_eq!(token.len(), 64);
        assert!(
            line.ends_with("mcp-bridge") && !line.contains(&token),
            "{line}"
        );
        let listed = command_line(pid.parse().expect("a process id"));
        assert!(
            listed.contains("mcp-bridge"),
            "the process is the bridge: {listed}"
        );
        assert!(
            !listed.contains(&token),
            "the bearer is on a command line: {listed}"
        );
    });
}
