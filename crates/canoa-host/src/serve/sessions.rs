//! Sessions that outlive the connection that opened them. An entry with `shared = true` keeps each session's agent running in
//! the host, held by a [`Hub`]: any key the entry admits attaches by `session/load` or `session/resume`, the replay brings it
//! up to date, every attached client sees every update, and a closed link only detaches. The agent ends when the person ends it
//! (`session/close`), when the entry's idle time passes with nobody attached, when it exits, or when the host stops.
//!
//! The agent is served exactly as an unshared connection's is, through a pair of in-process links: the host's end runs the
//! usual agent flow (profile, layers, history) and the other end belongs to the session's driver, which speaks to the hub.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "queues, locks and threads are std's"
)]

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_acp_hub::{Action, Client, ClientId, Hub, HubError, Note, Output, Policy, Verdict};
use coqui_peer_webcrypto::PeerId;
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::{FIRST_MESSAGE, MAX_LINE, Served, Shared};
use crate::control::Grant;
use crate::local::{self, Link};

/// How often the driver and each client's writer look up to see whether anything ended.
const POLL: Duration = Duration::from_millis(50);
/// Bytes one client may have waiting to be sent; a client further behind than this is detached, so that one that stopped
/// reading cannot grow the host's memory. A replay of the longest log the hub keeps (64 MiB) fits.
const MAX_BACKLOG: usize = 256 * 1024 * 1024;
/// The request id the host attaches the opening client to the hub with; its answer is not sent on.
const ATTACH_ID: &str = "canoa-attach";

/// What an entry with `shared = true` keeps.
#[derive(Debug)]
pub(super) struct Keep {
    /// How long a session nobody is attached to is kept; `None` keeps it until it is ended.
    pub(super) idle: Option<Duration>,
}

/// The shared sessions the host holds, across entries.
#[derive(Default)]
pub(super) struct Hosted {
    sessions: Mutex<Vec<Arc<Session>>>,
}

impl Hosted {
    fn lock(&self) -> MutexGuard<'_, Vec<Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The live session of `entry` the agent called `id`.
    fn find(&self, entry: &str, id: &str) -> Option<Arc<Session>> {
        self.lock()
            .iter()
            .find(|session| session.entry == entry && session.has_id(id))
            .cloned()
    }

    /// A live session of `entry` that has an agent's `initialize` answer to give.
    fn any_ready(&self, entry: &str) -> Option<Value> {
        self.lock()
            .iter()
            .filter(|session| session.entry == entry)
            .find_map(|session| session.initialize_answer())
    }

    fn count(&self, entry: &str) -> usize {
        self.lock()
            .iter()
            .filter(|session| session.entry == entry)
            .count()
    }

    fn remove(&self, session: &Arc<Session>) {
        self.lock().retain(|other| !Arc::ptr_eq(other, session));
    }
}

/// What lets an attach: the key is still paired, the entry still lists it, and it is trusted with at least what the key that opened
/// the session was. A session's tools and secrets are the opener's, so a key trusted with less never wields them.
fn permits(
    paired: bool,
    listed: bool,
    opener: &BTreeSet<Grant>,
    have: &BTreeSet<Grant>,
    action: &Action<'_>,
) -> Verdict {
    if !paired {
        return Verdict::Deny("this key is no longer paired".to_owned());
    }
    if !listed {
        return Verdict::Deny("this entry does not list this key".to_owned());
    }
    if matches!(action, Action::Attach) && !opener.is_subset(have) {
        return Verdict::Deny(
            "this key is trusted with less than the key that opened the session".to_owned(),
        );
    }
    Verdict::Allow
}

/// The host's grants as the hub's policy, asked again at each action, so a key unpaired or unlisted meanwhile loses its access
/// to a session already open.
struct Access {
    shared: Weak<Shared>,
    served: Arc<Served>,
    opener: BTreeSet<Grant>,
}

impl Policy for Access {
    fn check(&self, client: &Client, action: &Action<'_>) -> Verdict {
        let Some(shared) = self.shared.upgrade() else {
            return Verdict::Deny("the host is stopping".to_owned());
        };
        let Ok(peer) = client.id.as_str().parse::<PeerId>() else {
            return Verdict::Deny("not a key of this host".to_owned());
        };
        let own = peer == shared.method.peer_id();
        permits(
            own || shared.store.is_accepted(&peer),
            own || shared.allows(&peer, &self.served),
            &self.opener,
            &shared.store_grants(&peer, false).unwrap_or_default(),
            action,
        )
    }
}

/// The messages waiting for one client, and how many bytes they are.
struct Outbox {
    queue: Sender<Vec<u8>>,
    pending: Arc<AtomicUsize>,
}

/// The reading end of an [`Outbox`].
struct Inbox {
    queue: Receiver<Vec<u8>>,
    pending: Arc<AtomicUsize>,
}

enum Pushed {
    Queued,
    /// The client is too far behind.
    Behind,
    /// The client's connection is gone.
    Gone,
}

fn mailbox() -> (Outbox, Inbox) {
    let (queue, receiver) = mpsc::channel();
    let pending = Arc::new(AtomicUsize::new(0));
    (
        Outbox {
            queue,
            pending: Arc::clone(&pending),
        },
        Inbox {
            queue: receiver,
            pending,
        },
    )
}

impl Outbox {
    fn push(&self, message: Vec<u8>) -> Pushed {
        let length = message.len();
        if self.pending.load(Ordering::SeqCst).saturating_add(length) > MAX_BACKLOG {
            return Pushed::Behind;
        }
        self.pending.fetch_add(length, Ordering::SeqCst);
        match self.queue.send(message) {
            Ok(()) => Pushed::Queued,
            Err(_) => Pushed::Gone,
        }
    }
}

struct Slot {
    generation: u64,
    outbox: Outbox,
}

/// The messages a client sent before its session was known.
struct Boot {
    creator: ClientId,
    who: Client,
    /// The id of the `initialize` the agent answers.
    initialize: Option<Value>,
    /// What the agent answered it, which the hub gives every later client.
    answer: Option<Value>,
    /// The `session/new`, `session/load` or `session/resume` that opens the session: its id and the session it names.
    opening: Option<(Value, Option<String>)>,
    /// Ids whose answers go nowhere: requests the host already answered for the client.
    swallow: Vec<Value>,
}

enum Phase {
    Boot(Box<Boot>),
    Live(Box<Hub>),
    Over,
}

struct State {
    phase: Phase,
    clients: BTreeMap<ClientId, Slot>,
    next_generation: u64,
    /// The session's id once the agent has opened it.
    id: Option<String>,
    /// What `initialize` answers once the agent has said it, with what the hub adds.
    initialize: Option<Value>,
}

/// One agent session, its agent and the clients attached.
pub(super) struct Session {
    entry: String,
    epoch: Instant,
    idle: Option<Duration>,
    /// The driver's end of the link the agent flow serves.
    agent: Link,
    access: Arc<dyn Policy>,
    state: Mutex<State>,
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn has_id(&self, id: &str) -> bool {
        let state = self.lock();
        state.id.as_deref() == Some(id) && matches!(state.phase, Phase::Live(_))
    }

    fn initialize_answer(&self) -> Option<Value> {
        self.lock().initialize.clone()
    }

    /// Attaches `who`, whose messages `outbox` carries to its link. A key already attached takes the place of its earlier
    /// connection, which is gone or about to be. The generation tells that connection's end from this one's.
    fn join(&self, who: Client, outbox: Outbox) -> Result<u64, String> {
        let mut state = self.lock();
        let State {
            phase,
            clients,
            next_generation,
            ..
        } = &mut *state;
        let Phase::Live(hub) = phase else {
            return Err("the session is not open".to_owned());
        };
        let id = who.id.clone();
        if clients.remove(&id).is_some() {
            // The earlier connection's writer ends with its queue; the hub forgets it so that this one may take the identity.
            let _ = hub.disconnect(&id);
        }
        hub.poll(
            self.epoch
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        );
        match hub.connect(who) {
            Ok(()) => {}
            Err(HubError::Refused(reason)) => return Err(reason),
            Err(error) => return Err(error.to_string()),
        }
        *next_generation += 1;
        clients.insert(
            id,
            Slot {
                generation: *next_generation,
                outbox,
            },
        );
        Ok(*next_generation)
    }

    /// What the client `id` sent.
    fn client_said(&self, id: &ClientId, bytes: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
            return tracing::debug!(entry = self.entry, "a client sent what is not JSON");
        };
        let mut state = self.lock();
        let now = self.now_ms();
        let State {
            phase,
            clients,
            id: session_id,
            ..
        } = &mut *state;
        match phase {
            Phase::Boot(boot) => {
                if *id != boot.creator {
                    return;
                }
                if let (Some(method), Some(request)) = (
                    message.get("method").and_then(Value::as_str),
                    message.get("id"),
                ) {
                    match method {
                        "session/new" => boot.opening = Some((request.clone(), None)),
                        "session/load" | "session/resume" => {
                            let named = message
                                .pointer("/params/sessionId")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            boot.opening = Some((request.clone(), named));
                        }
                        _ => {}
                    }
                }
                if self.agent.send(bytes).is_err() {
                    self.agent.close();
                }
            }
            Phase::Live(hub) => {
                hub.poll(now);
                if message.get("method").and_then(Value::as_str) == Some("session/close")
                    && message.pointer("/params/sessionId").and_then(Value::as_str)
                        == session_id.as_deref()
                    && let Some(request) = message.get("id")
                {
                    // The person's choice to end the session; the agent's end follows from the driver.
                    let answer = json!({"jsonrpc": "2.0", "id": request, "result": {}});
                    if let Some(slot) = clients.get(id) {
                        let _ = slot.outbox.push(answer.to_string().into_bytes());
                    }
                    tracing::info!(entry = self.entry, client = %id, "a client ended the session");
                    self.agent.close();
                    return;
                }
                match hub.from_client(id, &message) {
                    Ok(outputs) => self.dispatch(&mut state, outputs),
                    Err(error) => {
                        tracing::debug!(entry = self.entry, "a client was not attached: {error}");
                    }
                }
            }
            Phase::Over => {}
        }
    }

    /// What the agent sent.
    fn agent_said(&self, bytes: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
            return tracing::debug!(entry = self.entry, "the agent sent what is not JSON");
        };
        let mut state = self.lock();
        let now = self.now_ms();
        match &mut state.phase {
            Phase::Boot(_) => self.boot_from_agent(&mut state, bytes, &message, now),
            Phase::Live(hub) => {
                hub.poll(now);
                let outputs = hub.from_agent(&message);
                self.dispatch(&mut state, outputs);
            }
            Phase::Over => {}
        }
    }

    /// The agent's message before the session is open: passed on to the opening client unread, but for the answers that open it.
    fn boot_from_agent(&self, state: &mut State, bytes: &[u8], message: &Value, now: u64) {
        let State {
            phase,
            clients,
            id: session_id,
            initialize,
            ..
        } = state;
        let Phase::Boot(boot) = phase else { return };
        let creator = boot.creator.clone();
        let to_creator = |bytes: &[u8]| {
            if let Some(slot) = clients.get(&creator) {
                let _ = slot.outbox.push(bytes.to_vec());
            }
        };
        let answered = message
            .get("method")
            .is_none()
            .then(|| message.get("id"))
            .flatten();
        let Some(answered) = answered else {
            return to_creator(bytes);
        };
        let swallowed = boot.swallow.contains(answered);
        if boot.initialize.as_ref() == Some(answered) {
            if let Some(result) = message.get("result") {
                boot.answer = Some(result.clone());
            }
            boot.initialize = None;
        }
        if swallowed {
            return;
        }
        let opened = boot
            .opening
            .as_ref()
            .filter(|(request, _)| request == answered)
            .cloned();
        let Some((_, named)) = opened else {
            return to_creator(bytes);
        };
        let Some(result) = message.get("result") else {
            // The agent refused: the client may try again.
            boot.opening = None;
            return to_creator(bytes);
        };
        let Some(opened) = result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(named)
        else {
            boot.opening = None;
            return to_creator(bytes);
        };
        // The answer reaches the client as the agent gave it; from here the hub speaks to it.
        to_creator(bytes);
        let answer = boot
            .answer
            .take()
            .unwrap_or_else(|| json!({"protocolVersion": 1, "agentCapabilities": {}}));
        let mut hub = Hub::builder(opened.clone())
            .initialize_result(answer.clone())
            .policy(Arc::clone(&self.access))
            .build();
        hub.poll(now);
        if let Err(error) = hub.connect(boot.who.clone()) {
            tracing::warn!(
                entry = self.entry,
                "the session's opener was refused: {error}"
            );
            self.agent.close();
            return;
        }
        let attach = json!({"jsonrpc": "2.0", "id": ATTACH_ID, "method": "session/resume",
            "params": {"sessionId": opened}});
        let outputs = hub.from_client(&creator, &attach).unwrap_or_default();
        *initialize = Some(with_loading(answer));
        *session_id = Some(opened.clone());
        *phase = Phase::Live(Box::new(hub));
        tracing::info!(
            entry = self.entry,
            session = opened,
            "a shared session is open"
        );
        self.dispatch(state, outputs);
    }

    /// Sends what the hub returned, in order, and logs its notes. A client that is not keeping up is detached.
    fn dispatch(&self, state: &mut State, outputs: Vec<Output>) {
        let mut work: VecDeque<Output> = outputs.into();
        while let Some(output) = work.pop_front() {
            match output {
                Output::ToClient { client, message } => {
                    if message.get("id").and_then(Value::as_str) == Some(ATTACH_ID) {
                        continue;
                    }
                    let Some(slot) = state.clients.get(&client) else {
                        continue;
                    };
                    let bytes = message.to_string().into_bytes();
                    if bytes.len() > MAX_LINE {
                        tracing::warn!(entry = self.entry, "a message over the limit was not sent");
                        continue;
                    }
                    match slot.outbox.push(bytes) {
                        Pushed::Queued => {}
                        Pushed::Behind => {
                            tracing::info!(
                                entry = self.entry,
                                %client,
                                "a client that is not keeping up was detached"
                            );
                            work.extend(self.evict(state, &client));
                        }
                        Pushed::Gone => work.extend(self.evict(state, &client)),
                    }
                }
                Output::ToAgent(message) => {
                    if self.agent.send(message.to_string().as_bytes()).is_err() {
                        self.agent.close();
                    }
                }
                Output::Note(note) => self.say(&note),
                _ => {}
            }
        }
    }

    /// Drops a client's queue and the hub's record of it, returning what the hub then has to send.
    fn evict(&self, state: &mut State, client: &ClientId) -> Vec<Output> {
        state.clients.remove(client);
        match &mut state.phase {
            Phase::Live(hub) => hub.disconnect(client).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn say(&self, note: &Note) {
        match note {
            Note::Refused {
                client,
                action,
                reason,
            } => tracing::info!(entry = self.entry, %client, action, "refused: {reason}"),
            Note::Ignored { client, reason } => {
                tracing::debug!(entry = self.entry, ?client, "ignored: {reason}");
            }
            Note::PermissionAnswered { request, by } => {
                tracing::info!(
                    entry = self.entry,
                    request,
                    ?by,
                    "a permission was answered"
                );
            }
            Note::PermissionUnanswered {
                request,
                waiting_ms,
            } => tracing::info!(
                entry = self.entry,
                request,
                waiting_ms,
                "a permission is waiting for an answer"
            ),
            Note::Ended => tracing::info!(entry = self.entry, "the agent ended the session"),
            _ => {}
        }
    }

    /// A connection's end: the session goes on, unless it had not opened yet and its only client left.
    fn detach(&self, client: &ClientId, generation: u64) {
        let mut state = self.lock();
        if state
            .clients
            .get(client)
            .is_none_or(|slot| slot.generation != generation)
        {
            return;
        }
        if matches!(state.phase, Phase::Boot(_)) {
            state.clients.remove(client);
            drop(state);
            return self.agent.close();
        }
        let outputs = self.evict(&mut state, client);
        self.dispatch(&mut state, outputs);
    }

    /// What the driver does between messages: the hub's clock, and the idle rule.
    fn tick(&self) {
        let mut state = self.lock();
        let now = self.now_ms();
        let Phase::Live(hub) = &mut state.phase else {
            return;
        };
        let due = hub.poll(now);
        let idle = self.idle.is_some_and(|idle| {
            hub.attached_count() == 0
                && now.saturating_sub(hub.last_activity())
                    >= u64::try_from(idle.as_millis()).unwrap_or(u64::MAX)
        });
        self.dispatch(&mut state, due);
        if idle {
            tracing::info!(
                entry = self.entry,
                "a shared session nobody is attached to timed out"
            );
            self.agent.close();
        }
    }

    /// The agent's end: the hub says so to whoever is attached, and what they are sent is the last of it.
    fn finish(&self) {
        let mut state = self.lock();
        if let Phase::Live(hub) = &mut state.phase {
            let outputs = hub.agent_closed();
            self.dispatch(&mut state, outputs);
        }
        state.phase = Phase::Over;
        // Dropping the queues lets each client's writer deliver what is left and end.
        state.clients.clear();
    }

    /// Reads the agent until it ends.
    fn drive(&self, stopped: &AtomicBool) {
        loop {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            match Channel::receive(&self.agent, Some(POLL)) {
                Ok(Some(bytes)) => self.agent_said(&bytes),
                Err(Failure::Timeout) => self.tick(),
                Ok(None) | Err(_) => break,
            }
        }
        self.finish();
    }
}

/// `initialize`'s answer as the hub gives it.
fn with_loading(mut answer: Value) -> Value {
    if let Some(capabilities) = answer
        .get_mut("agentCapabilities")
        .and_then(Value::as_object_mut)
    {
        capabilities.insert("loadSession".to_owned(), json!(true));
        let session = capabilities
            .entry("sessionCapabilities")
            .or_insert_with(|| json!({}));
        if let Some(session) = session.as_object_mut() {
            session.entry("resume").or_insert_with(|| json!({}));
        }
    }
    answer
}

fn error_to(request: &Value, message: &str) -> Vec<u8> {
    json!({"jsonrpc": "2.0", "id": request, "error": {"code": -32000, "message": message}})
        .to_string()
        .into_bytes()
}

/// Serves one connection of a shared entry: the client opens a session, or attaches to one that is running.
pub(super) fn serve<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    keep: &Keep,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    in_process: bool,
) {
    let who = Client::new(peer.to_string())
        .label(served.name.clone())
        .role("operator")
        .subject(if *peer == shared.method.peer_id() {
            "owner".to_owned()
        } else {
            format!("key:{peer}")
        });
    // The caller's channel hands `first` over again before anything else; it is held already.
    if !matches!(channel.receive(Some(FIRST_MESSAGE)), Ok(Some(_))) {
        return;
    }
    let mut swallow = Vec::new();
    let mut replay: Vec<Vec<u8>> = Vec::new();
    if let Some(answer) = shared.hosted.any_ready(&served.name)
        && let Ok(message) = serde_json::from_slice::<Value>(first)
        && message.get("method").and_then(Value::as_str) == Some("initialize")
        && let Some(request) = message.get("id")
    {
        // Somebody may be attaching to a session that runs: it is told what the agent offers, and whether it opens a session
        // of its own or attaches is its next message.
        let reply = json!({"jsonrpc": "2.0", "id": request, "result": answer});
        if channel.send(reply.to_string().as_bytes()).is_err() {
            return;
        }
        swallow.push(request.clone());
        loop {
            let Ok(Some(next)) = channel.receive(Some(FIRST_MESSAGE)) else {
                return;
            };
            let Ok(message) = serde_json::from_slice::<Value>(&next) else {
                return;
            };
            match (
                message.get("method").and_then(Value::as_str),
                message.get("id"),
            ) {
                (Some("authenticate"), Some(request)) => {
                    let reply = json!({"jsonrpc": "2.0", "id": request, "result": {}});
                    if channel.send(reply.to_string().as_bytes()).is_err() {
                        return;
                    }
                    swallow.push(request.clone());
                    replay.push(next);
                }
                (Some("session/load" | "session/resume"), Some(request)) => {
                    let named = message
                        .pointer("/params/sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(session) = shared.hosted.find(&served.name, named) {
                        return attach(&session, who, channel, &next, request);
                    }
                    replay.push(next);
                    break;
                }
                _ => {
                    replay.push(next);
                    break;
                }
            }
        }
    }
    // Opening a session of its own: what the client sent goes to the agent in order, after the `initialize` it was answered.
    open(
        shared,
        served,
        keep,
        channel,
        (first, peer),
        (who, swallow, replay),
        in_process,
    );
}

/// Attaches the connection to a session that runs: `message` is its `session/load` or `session/resume`.
fn attach<C: Channel>(
    session: &Arc<Session>,
    who: Client,
    channel: &C,
    message: &[u8],
    request: &Value,
) {
    let id = who.id.clone();
    let (outbox, queue) = mailbox();
    let generation = match session.join(who, outbox) {
        Ok(generation) => generation,
        Err(reason) => {
            let _ = channel.send(&error_to(request, &reason));
            return;
        }
    };
    session.client_said(&id, message);
    pump(session, &id, generation, channel, queue);
}

/// Starts the session's agent for a client that opens one.
fn open<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    keep: &Keep,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    (who, swallow, replay): (Client, Vec<Value>, Vec<Vec<u8>>),
    in_process: bool,
) {
    let request = serde_json::from_slice::<Value>(first)
        .ok()
        .and_then(|message| message.get("id").cloned());
    if shared.hosted.count(&served.name) >= served.running.ceiling() {
        tracing::info!(
            entry = served.name,
            %peer,
            "turned away: max_peers sessions are kept"
        );
        if let Some(request) = request {
            let _ = channel.send(&error_to(
                &request,
                "this entry keeps as many sessions as it may",
            ));
        }
        return;
    }
    let (driver, host) = local::pair(MAX_LINE, Arc::clone(&shared.stopped));
    let access: Arc<dyn Policy> = Arc::new(Access {
        shared: Arc::downgrade(shared),
        served: Arc::clone(served),
        opener: shared.store_grants(peer, false).unwrap_or_default(),
    });
    let id = who.id.clone();
    let session = Arc::new(Session {
        entry: served.name.clone(),
        epoch: Instant::now(),
        idle: keep.idle,
        agent: driver,
        access,
        state: Mutex::new(State {
            phase: Phase::Boot(Box::new(Boot {
                creator: id.clone(),
                who,
                initialize: request,
                answer: None,
                opening: None,
                swallow,
            })),
            clients: BTreeMap::new(),
            next_generation: 0,
            id: None,
            initialize: None,
        }),
    });
    let (outbox, queue) = mailbox();
    session.lock().clients.insert(
        id.clone(),
        Slot {
            generation: 0,
            outbox,
        },
    );
    shared.hosted.lock().push(Arc::clone(&session));
    // The agent runs as an unshared connection's does, over the other end of the link, and ends when the driver closes its end.
    let flow = {
        let (shared, served) = (Arc::clone(shared), Arc::clone(served));
        let (first, peer) = (first.to_vec(), *peer);
        thread::Builder::new()
            .name("canoa-session-agent".to_owned())
            .spawn(move || {
                let prefilled = coqui_agentic_peer::link::Prefilled::new(&host, first.clone());
                super::give_agent_flow(
                    &shared,
                    &served,
                    &prefilled,
                    (&first, &peer),
                    (None, in_process),
                );
                host.close();
            })
    };
    let driving = {
        let (shared, session) = (Arc::clone(shared), Arc::clone(&session));
        thread::Builder::new()
            .name("canoa-session".to_owned())
            .spawn(move || {
                session.drive(&shared.stopped);
                shared.hosted.remove(&session);
            })
    };
    if flow.is_err() || driving.is_err() {
        tracing::warn!(entry = served.name, "a session's threads could not start");
        session.agent.close();
        shared.hosted.remove(&session);
        return;
    }
    for message in replay {
        session.client_said(&id, &message);
    }
    pump(&session, &id, 0, channel, queue);
}

/// Carries one connection's messages both ways until it ends, then detaches it.
fn pump<C: Channel>(
    session: &Arc<Session>,
    id: &ClientId,
    generation: u64,
    channel: &C,
    queue: Inbox,
) {
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        let done = &done;
        scope.spawn(move || {
            loop {
                match queue.queue.recv_timeout(POLL) {
                    Ok(message) => {
                        let length = message.len();
                        let sent = send(channel, &message, done);
                        queue.pending.fetch_sub(length, Ordering::SeqCst);
                        if sent.is_err() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) if done.load(Ordering::SeqCst) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            done.store(true, Ordering::SeqCst);
        });
        while !done.load(Ordering::SeqCst) {
            match channel.receive(Some(POLL)) {
                Ok(Some(message)) => session.client_said(id, &message),
                Ok(None) => break,
                Err(Failure::Timeout) => {}
                Err(_) => break,
            }
        }
        done.store(true, Ordering::SeqCst);
    });
    session.detach(id, generation);
}

/// Sends one message, waiting on a client that is slow to take it.
fn send<C: Channel>(channel: &C, message: &[u8], done: &AtomicBool) -> Result<(), Failure> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match channel.send(message) {
            Ok(()) => return Ok(()),
            Err(Failure::NoProgress) if Instant::now() < until && !done.load(Ordering::SeqCst) => {
                thread::sleep(POLL);
            }
            Err(failure) => return Err(failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_grants() -> BTreeSet<Grant> {
        Grant::all().into_iter().collect()
    }

    #[test]
    fn a_key_that_is_unpaired_or_unlisted_is_refused_every_action() {
        let none = BTreeSet::new();
        for action in [Action::Attach, Action::Prompt, Action::Cancel] {
            assert!(matches!(
                permits(false, true, &none, &none, &action),
                Verdict::Deny(_)
            ));
            assert!(matches!(
                permits(true, false, &none, &none, &action),
                Verdict::Deny(_)
            ));
            assert!(matches!(
                permits(true, true, &none, &none, &action),
                Verdict::Allow
            ));
        }
    }

    #[test]
    fn a_key_trusted_with_less_than_the_opener_does_not_attach() {
        let opener = all_grants();
        let none = BTreeSet::new();
        assert!(matches!(
            permits(true, true, &opener, &none, &Action::Attach),
            Verdict::Deny(_)
        ));
        assert!(matches!(
            permits(true, true, &opener, &opener, &Action::Attach),
            Verdict::Allow
        ));
        // Already attached, a key is held to the same pairing and listing, not to the opener's grants again.
        assert!(matches!(
            permits(true, true, &opener, &none, &Action::Prompt),
            Verdict::Allow
        ));
    }

    #[test]
    fn what_the_hub_adds_to_initialize_is_loading_and_resuming() {
        let answer = with_loading(json!({"protocolVersion": 1, "agentCapabilities": {}}));
        assert_eq!(answer["agentCapabilities"]["loadSession"], true);
        assert!(answer["agentCapabilities"]["sessionCapabilities"]["resume"].is_object());
    }
}
