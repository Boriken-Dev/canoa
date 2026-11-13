//! A task handed to an agent on another machine, and this host serving A2A to a key it grants: two hosts in this process, joined
//! by a loopback WebSocket, each running scripted stand-ins. Host A's parent hands a task to the source `far`, which is host B's
//! `shared`; B's task is a normal session of B's stand-in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use canoa_host::{Link, LinkError};
use coqui_a2a::model::{Message, Role, SendMessageRequest};
use coqui_a2a::{Client, ClientError};
use coqui_agentic_peer::auth::{self, Identity, Proof, Sealed, Trust};
use coqui_agentic_peer::home::identity_file;
use coqui_agentic_peer::mux::{Limits, Mux, Side};
use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_transport::Channel as _;
use coqui_transport_web::Dialer as _;
use coqui_transport_web::Kind;
use coqui_transport_web::websocket::{self, Frames};
use linked::Hosted;
use serde_json::{Value, json};
use support::{Scratch, WAIT, example, method, quoted, running};

const META: &str = "dev.boriken.canoa/delegate";
const MAX: usize = 16 * 1024 * 1024;

fn agent(name: &str, extra: &str, toml: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nargs = [\"--scripted\", \"--mcp-http\", \"--modes=default,manual\"{extra}]\nlisten = \"127.0.0.1:0\"\n{toml}\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
    )
}

fn marker_arg(marker: &Path) -> String {
    format!(
        ", \"--marker={}\"",
        marker.to_string_lossy().replace('\\', "/")
    )
}

fn directory(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("a directory");
    dir
}

fn toml_path(path: &Path) -> String {
    quoted(&path.to_string_lossy())
}

/// A host's scratch with its identity made, so another host can be configured with its key before it runs.
fn scratch_with_identity() -> (Scratch, Identity) {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    std::fs::create_dir_all(&home).expect("a home");
    let identity = Identity::open_or_create(&home.join(identity_file(method())), method())
        .expect("an identity");
    (scratch, identity)
}

/// Pairs `identity` in the host whose scratch this is, before it runs.
fn pair(scratch: &Scratch, identity: &Identity) {
    let store = TrustStore::open(&scratch.0.join("home").join("peers")).expect("a trust store");
    store
        .accept_key(&identity.key_material())
        .expect("accepted");
}

/// Host B: `shared` (marked, with a working root) and `hidden`, which only `other` may use, granting `a2a` to `granted`.
struct Far {
    host: Hosted,
    root: PathBuf,
    marker: PathBuf,
    id: PeerId,
    url: String,
}

fn far(granted: &Identity, other: &Identity, paired: &[&Identity]) -> Far {
    let (scratch, identity) = scratch_with_identity();
    for key in paired {
        pair(&scratch, key);
    }
    let root = directory(&scratch.0, "root");
    let marker = scratch.0.join("far.pids");
    let config = format!(
        "{}{}[host.delegate]\na2a = [\"{}\"]\n",
        agent(
            "shared",
            &marker_arg(&marker),
            &format!("working_roots = [{}]", toml_path(&root))
        ),
        agent("hidden", "", &format!("peers = [\"{}\"]", other.peer_id())),
        granted.peer_id()
    );
    let host = Hosted::start_in(scratch, &config, &[]);
    let line = host.line_of("shared").expect("shared is listening");
    Far {
        id: identity.peer_id(),
        url: line["url"].as_str().expect("a url").to_owned(),
        host,
        root,
        marker,
    }
}

/// Host A, whose parent may hand a task to `far`, B's `shared`.
fn near(scratch: Scratch, far: &Far, sources: &str) -> Hosted {
    let config = format!(
        "{}[host.delegate]\n{sources}\n[agents.far]\na2a = {{ url = {}, peer = \"{}\", source = \"shared\" }}\n",
        agent("parent", "", ""),
        quoted(&far.url),
        far.id
    );
    Hosted::start_in(scratch, &config, &[])
}

type Socket = websocket::Channel;

/// Dials `url`, proves `identity` to `host`, and runs `body` over the link's streams.
fn over_link<R>(
    url: &str,
    host: PeerId,
    identity: &Identity,
    body: impl FnOnce(&Mux<'_, Sealed<'_, Socket>>) -> R,
) -> R {
    let url = url::Url::parse(url).expect("a url");
    let mut dial = coqui_transport_web::Limits::default();
    dial.max_message = method().record_length(MAX);
    let channel = websocket::Dialer::builder()
        .frames(Frames::Binary)
        .limits(dial)
        .build()
        .expect("a dialer")
        .dial(Kind::WebSocket, &url, WAIT)
        .expect("dialed");
    let proof = Proof::builder(identity.clone())
        .responders(Trust::pinned([host]))
        .build()
        .expect("a proof");
    let established =
        auth::prove_out(&proof, &channel, WAIT, method().record_length(MAX)).expect("proved");
    let sealed = Sealed::new(&channel, established.session);
    let mut limits = Limits::default();
    limits.max_message = MAX;
    let result = Mux::run(&sealed, Side::Dialed, limits, None, body);
    sealed.close();
    channel.close();
    result
}

/// A client of one link: it asks, and answers what the host asks of it as the test says.
struct Convo<'a> {
    link: &'a Link,
    next: u64,
    requests: Vec<Value>,
    updates: Vec<Value>,
}

impl<'a> Convo<'a> {
    fn new(link: &'a Link) -> Self {
        Self {
            link,
            next: 0,
            requests: Vec::new(),
            updates: Vec::new(),
        }
    }

    fn message(&self, wait: Duration) -> Option<Value> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self
                .link
                .receive(Some(left.min(Duration::from_millis(200))))
            {
                Ok(Some(bytes)) => return serde_json::from_slice(&bytes).ok(),
                Err(LinkError::Timeout) => {}
                Ok(None) | Err(_) => return None,
            }
        }
    }

    fn send(&self, message: &Value) {
        self.link
            .send(message.to_string().as_bytes())
            .expect("sent");
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let until = Instant::now() + WAIT;
        loop {
            let message = self
                .message(until.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|| panic!("no answer to {method}"));
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    self.requests.push(message);
                } else {
                    self.updates.push(message);
                }
            } else if message["id"] == id {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    fn open(&mut self, cwd: &Path) -> String {
        self.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": { META: true }}),
        )
        .expect("initialize");
        let session = self
            .request(
                "session/new",
                json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
            )
            .expect("a session");
        session["sessionId"].as_str().expect("an id").to_owned()
    }

    /// One prompt and the text the agent said.
    fn prompt(&mut self, session: &str, text: &str) -> String {
        self.updates.clear();
        let result = self
            .request(
                "session/prompt",
                json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
            )
            .expect("the turn is answered");
        assert_eq!(result["stopReason"], "end_turn", "{text}");
        self.updates
            .iter()
            .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
            .collect()
    }

    fn delegate(&mut self, session: &str, source: &str, task: &str) -> String {
        let said = self.prompt(session, &format!("Delegate {source} {task}"));
        let answer: Value =
            serde_json::from_str(&said).unwrap_or_else(|_| panic!("a task id: {said}"));
        answer["taskId"].as_str().expect("a task id").to_owned()
    }

    /// `task_status`, repeated until the task is in a state it will not leave when `wait` asks for the end.
    fn status(&mut self, session: &str, task: &str, wait_ms: u64) -> Value {
        let limit = Instant::now() + WAIT;
        loop {
            let said = self.prompt(session, &format!("Status {task} {wait_ms}"));
            let state: Value =
                serde_json::from_str(&said).unwrap_or_else(|_| json!({"error": said}));
            let ended = ["completed", "failed", "canceled", "rejected"]
                .iter()
                .any(|name| state["state"] == *name);
            if wait_ms < 20_000 || ended || state.get("error").is_some() || Instant::now() > limit {
                return state;
            }
        }
    }

    fn host_request(&mut self) -> Value {
        let until = Instant::now() + WAIT;
        loop {
            if let Some(found) = self.requests.pop() {
                return found;
            }
            let message = self
                .message(until.saturating_duration_since(Instant::now()))
                .expect("the host asks the client something");
            if message.get("method").is_some() && message.get("id").is_some() {
                return message;
            }
        }
    }
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < limit, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn pids(marker: &Path) -> Vec<u32> {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn text_request(text: &str, source: &str) -> SendMessageRequest {
    SendMessageRequest {
        tenant: Some(source.to_owned()),
        ..SendMessageRequest::new(Message::text(Role::User, "m-1", text))
    }
}

#[test]
fn the_far_hosts_card_lists_the_sources_the_key_may_use_and_no_other() {
    let (near_scratch, near_key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&near_key, &other, &[&near_key, &other]);
    let card = over_link(&far.url, far.id, &near_key, |mux| {
        Client::new(mux).agent_card().expect("a card")
    });
    let ids: Vec<&str> = card.skills.iter().map(|skill| skill.id.as_str()).collect();
    assert_eq!(ids, ["shared"], "hidden lists only `other`");
    drop(near_scratch);
    far.host.stop();
}

#[test]
fn a_parent_hands_a_task_to_the_far_hosts_source_and_reads_its_result() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    let near = near(scratch, &far, "sources = [\"far\"]");
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    let task = convo.delegate(&session, "far", "Verbatim hello  world");
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["source"], "far");
    assert_eq!(status["result"], "blocks=1 text=Verbatim hello  world");
    drop(link);
    near.stop();
    far.host.stop();
}

#[test]
fn a_key_without_the_a2a_grant_is_refused_the_protocol_whatever_else_it_may_do() {
    let (_scratch, granted) = scratch_with_identity();
    let paired = Identity::generate(method()).expect("a key");
    let far = far(&granted, &paired, &[&granted, &paired]);
    let refused = over_link(&far.url, far.id, &paired, |mux| {
        Client::new(mux).send_message(&text_request("Say hi", "shared"))
    });
    assert!(
        matches!(refused, Err(ClientError::NotGranted(_))),
        "{refused:?}"
    );
    let card = over_link(&far.url, far.id, &paired, |mux| {
        Client::new(mux).agent_card()
    });
    assert!(matches!(card, Err(ClientError::NotGranted(_))), "{card:?}");
    far.host.stop();
}

#[test]
fn a_remote_source_the_sources_setting_does_not_name_is_not_delegable() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    for sources in ["", "sources = []", "sources = [\"parent\"]"] {
        let near = near(Scratch::new(), &far, sources);
        let link = near.local.open("parent").expect("a link");
        let mut convo = Convo::new(&link);
        let session = convo.open(&work);
        let said = convo.prompt(&session, "Delegate far hi");
        assert!(said.starts_with("error:"), "{sources}: {said}");
        assert!(pids(&far.marker).is_empty(), "nothing started there");
        drop(link);
        near.stop();
    }
    far.host.stop();
}

#[test]
fn a_remote_question_reaches_the_person_on_the_parent_and_the_answer_reaches_the_far_agent() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    let near = near(scratch, &far, "sources = [\"far\"]");
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    let task = convo.delegate(&session, "far", "Please edit src/report.js");
    let asked = convo.host_request();
    assert_eq!(asked["method"], "session/request_permission");
    assert_eq!(asked["params"]["sessionId"], session);
    let meta = &asked["params"]["_meta"][META];
    assert_eq!(meta["taskId"], task.as_str());
    assert_eq!(meta["source"], "far");
    assert!(
        asked["params"]["options"]
            .as_array()
            .is_some_and(|options| options.len() == 4)
    );
    assert_eq!(convo.status(&session, &task, 0)["state"], "input_required");
    convo.send(&json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}}));
    let done = convo.status(&session, &task, 20_000);
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(done["result"], "Renamed everywhere.", "the allow reached B");

    // A refusal reaches it too.
    let refused = convo.delegate(&session, "far", "Please edit again");
    let asked = convo.host_request();
    convo.send(&json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"outcome": {"outcome": "selected", "optionId": "no"}}}));
    let said = convo.status(&session, &refused, 20_000);
    assert_eq!(said["state"], "completed", "{said}");
    drop(link);
    near.stop();
    far.host.stop();
}

#[test]
fn cancelling_the_task_ends_the_far_hosts_child_process() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    let near = near(scratch, &far, "sources = [\"far\"]");
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    let task = convo.delegate(&session, "far", "Wait");
    until("the far child started", || pids(&far.marker).len() == 1);
    let pid = pids(&far.marker)[0];
    assert!(running(pid));
    let said = convo.prompt(&session, &format!("Cancel {task}"));
    assert!(said.contains("canceled"), "{said}");
    until("the far child ended", || !running(pid));
    drop(link);
    near.stop();
    far.host.stop();
}

#[test]
fn a_caller_that_leaves_before_its_task_ends_has_the_task_ended() {
    let (_scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    over_link(&far.url, far.id, &key, |mux| {
        let client = Client::new(mux);
        let mut events = client
            .send_streaming_message(&text_request("Wait", "shared"))
            .expect("streaming");
        let _ = events.next().expect("the task").expect("an event");
        until("it started", || pids(&far.marker).len() == 1);
    });
    let pid = pids(&far.marker)[0];
    until("the child of a caller that left ended", || !running(pid));
    far.host.stop();
}

#[test]
fn a_task_for_the_far_host_sends_no_folder_and_runs_where_the_far_host_puts_it() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    let near = near(scratch, &far, "sources = [\"far\"]");
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    // A folder cannot be named for it.
    let said = convo.prompt(&session, "DelegateIn . far hi");
    assert!(said.starts_with("error:"), "{said}");
    let task = convo.delegate(&session, "far", "Cwd");
    let status = convo.status(&session, &task, 20_000);
    let ran = status["result"].as_str().expect("a result").to_owned();
    let root = far.root.canonicalize().expect("canonical");
    assert!(
        ran.to_lowercase().contains(
            &root
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_lowercase()
        ),
        "{ran} is not in {}",
        root.display()
    );
    assert!(
        !ran.to_lowercase()
            .contains(&work.to_string_lossy().to_lowercase()),
        "{ran} is in the parent's folder"
    );
    drop(link);
    near.stop();
    far.host.stop();
}

#[test]
fn a_link_lost_mid_task_ends_the_mirror_failed_and_never_completed() {
    let (scratch, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let work = directory(&scratch.0, "work");
    let near = near(scratch, &far, "sources = [\"far\"]");
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    let task = convo.delegate(&session, "far", "Wait");
    until("the far child started", || pids(&far.marker).len() == 1);
    far.host.stop();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "failed", "{status}");
    assert!(
        status["message"]
            .as_str()
            .is_some_and(|message| message.contains("link")),
        "{status}"
    );
    drop(link);
    near.stop();
}

#[test]
fn a_task_can_name_no_far_source_when_the_far_host_is_unreachable_and_ends_failed() {
    let (scratch, _key) = scratch_with_identity();
    let (_s, key) = scratch_with_identity();
    let other = Identity::generate(method()).expect("a key");
    let far = far(&key, &other, &[&key]);
    let url = far.url.clone();
    let id = far.id;
    far.host.stop();
    drop(far);
    let work = directory(&scratch.0, "work");
    let config = format!(
        "{}[host.delegate]\nsources = [\"far\"]\n[agents.far]\na2a = {{ url = {}, peer = \"{id}\" }}\n",
        agent("parent", "", ""),
        quoted(&url)
    );
    let near = Hosted::start_in(scratch, &config, &[]);
    let link = near.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(&work);
    let task = convo.delegate(&session, "far", "hi");
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "failed", "{status}");
    drop(link);
    near.stop();
}
