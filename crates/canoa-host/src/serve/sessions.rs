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
use core::sync::atomic::{AtomicBool, Ordering};
use core::sync::atomic::{AtomicU32, AtomicUsize};
use core::time::Duration;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_acp_hub::{
    Action, Client, ClientId, Cursor, Hub, HubError, LogSnapshot, Note, Output, Policy, Verdict,
};
use coqui_peer_webcrypto::PeerId;
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::kept::{Keeper, Kept};
use super::watch::{self, Check, Watch};
use super::{FIRST_MESSAGE, Served, Shared};
use crate::approve::shared::{Answers, Decided};
use crate::control::{self, Grant, SharedSessions};
use crate::local::{self, Link};
use crate::usage_splice::TreeUse;

/// How often the driver and each client's writer look up to see whether anything ended.
const POLL: Duration = Duration::from_millis(50);
/// Bytes one client may have waiting to be sent; a client further behind than this is detached, so that one that stopped
/// reading cannot grow the host's memory. A replay of the longest log the hub keeps (64 MiB) fits.
const MAX_BACKLOG: usize = 256 * 1024 * 1024;
/// The request id the host attaches the opening client to the hub with; its answer is not sent on.
const ATTACH_ID: &str = "canoa-attach";
/// What the ids of a watch's own requests start with.
const WATCH_ID: &str = "canoa-watch-";
/// How long an approver's answer waits for the hub to hold the request it answers.
const HELD_ANSWER: Duration = Duration::from_secs(5);

/// What an entry with `shared = true` keeps.
#[derive(Debug)]
pub(super) struct Keep {
    /// How long a session nobody is attached to is kept; `None` keeps it until it is ended.
    pub(super) idle: Option<Duration>,
    /// How long a permission request may wait with nobody answering before the host logs it; `None` logs nothing.
    pub(super) permission_window: Option<Duration>,
    /// Whether the session's log is kept in the home, so that it survives the host's restart.
    pub(super) keep_log: bool,
    /// The updates the hub keeps for replay, at most; `None` takes its default.
    pub(super) log_entries: Option<usize>,
    /// The serialized bytes the hub keeps for replay, at most; `None` takes its default.
    pub(super) log_bytes: Option<usize>,
}

/// The shared sessions the host holds, across entries.
pub(super) struct Hosted {
    /// This host's own key, which the person's own link speaks as.
    owner: PeerId,
    sessions: Mutex<Vec<Arc<Session>>>,
    /// Where the sessions' logs are kept; `None` for a host with no home.
    kept: Option<Arc<Kept>>,
}

impl Hosted {
    pub(super) fn new(owner: PeerId, kept: Option<Arc<Kept>>) -> Self {
        Self {
            owner,
            sessions: Mutex::new(Vec::new()),
            kept,
        }
    }

    /// `who` as the hub's policy judges it: a key's id, or the host's own key for the host's own link.
    fn asker(&self, who: Option<&str>) -> Client {
        let id = who.map_or_else(|| self.owner.to_string(), str::to_owned);
        let subject = match who {
            None => "owner".to_owned(),
            Some(key) => format!("key:{key}"),
        };
        Client::new(id).role("operator").subject(subject)
    }

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

impl SharedSessions for Hosted {
    fn running(&self, who: Option<&str>) -> Vec<Value> {
        let asker = self.asker(who);
        let sessions: Vec<Arc<Session>> = self.lock().clone();
        newest_first(
            sessions
                .iter()
                .filter_map(|session| session.describe(&asker))
                .collect(),
        )
    }

    fn activity(&self, who: Option<&str>, session: Option<&str>) -> Vec<Value> {
        let asker = self.asker(who);
        let sessions: Vec<Arc<Session>> = self.lock().clone();
        newest_first(
            sessions
                .iter()
                .filter(|each| session.is_none_or(|id| each.has_id(id)))
                .filter_map(|each| each.activity(&asker))
                .collect(),
        )
    }

    fn watch(&self, who: Option<&str>, params: &Value) -> Result<Value, (i32, String)> {
        let id = params
            .get("session")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                (
                    control::BAD_PARAMS,
                    "session names the session to watch".to_owned(),
                )
            })?;
        let sessions: Vec<Arc<Session>> = self.lock().clone();
        let session = sessions
            .into_iter()
            .find(|each| each.has_id(id))
            .ok_or_else(|| {
                (
                    control::NOT_FOUND,
                    format!("no shared session {id} is running"),
                )
            })?;
        session.configure_watch(self.asker(who).label("watch"), params)
    }
}

/// The values of `listed`, the one with the latest time first.
fn newest_first(mut listed: Vec<(u64, Value)>) -> Vec<Value> {
    listed.sort_by_key(|entry| core::cmp::Reverse(entry.0));
    listed.into_iter().map(|(_, session)| session).collect()
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
    /// The `cwd` of the request that opens the session.
    cwd: Option<String>,
    /// The `session/load` or `session/resume` as the client sent it, which a restored session's hub answers itself.
    opening_message: Option<Value>,
    /// What the host kept of the session the opening request names, from before its restart.
    restored: Option<LogSnapshot>,
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
    /// The folder the session was opened in, as its opener named it.
    cwd: Option<String>,
    /// What the agent last called the session.
    title: Option<String>,
    /// What `initialize` answers once the agent has said it, with what the hub adds.
    initialize: Option<Value>,
    /// The file the session's log is kept in, while it is kept.
    keeper: Option<Keeper>,
    /// The session's watch, while one is armed.
    watch: Option<Watching>,
    /// The requests a watch sent as an attached client, whose answers that client is not sent.
    watch_asks: BTreeSet<(ClientId, String)>,
    /// The hub's number for each permission request the agent made, by the id the agent made it under, while it is open.
    asked: BTreeMap<String, u64>,
    /// How many prompts watches have sent, for their ids.
    watch_count: u64,
}

/// A watch and the identity its prompts are sent as.
struct Watching {
    watch: Watch,
    /// The key that armed it, as the hub's policy judges it.
    client: Client,
}

/// One agent session, its agent and the clients attached.
pub(super) struct Session {
    entry: String,
    epoch: Instant,
    idle: Option<Duration>,
    /// How long a permission request waits for an answer before the hub says so, in milliseconds.
    permission_window_ms: Option<u64>,
    /// When `epoch` was, in milliseconds since the Unix epoch, so that the hub's clock can be told as a date.
    started_unix_ms: u64,
    /// The driver's end of the link the agent flow serves.
    agent: Link,
    /// What the agent flow's approvers answered for requests the hub holds, and whether a client is attached for them.
    approvers: Arc<Answers>,
    /// Where the log is kept, for a session whose entry keeps it.
    kept: Option<Arc<Kept>>,
    /// The bounds of the hub's log, from the entry.
    log_bounds: (Option<usize>, Option<usize>),
    /// The longest message a client may be sent, `[host] max_message_bytes`.
    max_line: usize,
    access: Arc<dyn Policy>,
    /// The agent's process id once a program entry's agent has started; 0 before, after, and for an agent that is no process
    /// of this computer's own (a harness in the host, or one in an environment).
    agent_pid: Arc<AtomicU32>,
    /// Whether the session that opened it asked to be listed nowhere (`EPHEMERAL_META`).
    ephemeral: AtomicBool,
    /// What the agent's processes use now; it reads the process table, so it is never called with `state` held.
    read_processes: fn(u32) -> TreeUse,
    state: Mutex<State>,
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The hub's clock `now` as milliseconds since the Unix epoch.
    const fn unix_ms(&self, now: u64) -> u64 {
        self.started_unix_ms.saturating_add(now)
    }

    /// What [`Self::describe`] says, with how long the session has been quiet, its watch, and what the agent's processes use,
    /// read once the session's lock is let go.
    fn activity(&self, asker: &Client) -> Option<(u64, Value)> {
        let (active, mut out) = self.describe(asker)?;
        {
            let state = self.lock();
            if let Phase::Live(hub) = &state.phase {
                out["quietMs"] = json!(self.now_ms().saturating_sub(hub.last_activity()));
            }
            if let Some(watching) = &state.watch {
                out["watch"] = watching.watch.status();
            }
        }
        let pid = self.agent_pid.load(Ordering::Relaxed);
        if pid != 0 {
            let used = (self.read_processes)(pid);
            out["processes"] = json!({
                "count": used.processes,
                "cpuMs": used.cpu_ms,
                "memoryBytes": used.memory_bytes,
            });
        }
        Some((active, out))
    }

    /// Arms, changes or takes away the session's watch for `client`, which must be a key the session lets attach and prompt.
    fn configure_watch(&self, client: Client, params: &Value) -> Result<Value, (i32, String)> {
        for action in [Action::Attach, Action::Prompt] {
            if let Verdict::Deny(why) = self.access.check(&client, &action) {
                return Err((control::DENIED, why));
            }
        }
        let config =
            watch::Config::from_params(params).map_err(|why| (control::BAD_PARAMS, why))?;
        let pid = self.agent_pid.load(Ordering::Relaxed);
        let baseline = match &config {
            Some(config) if config.measures_processes() => {
                if pid == 0 {
                    return Err((
                        control::REFUSED,
                        "this session's agent is not a process of this computer's own (it runs inside the host or in an \
                         environment), so whileChildren, cpuBelowMs and memoryNotShrinking cannot be checked"
                            .to_owned(),
                    ));
                }
                Some((self.read_processes)(pid))
            }
            _ => None,
        };
        let now = self.unix_ms(self.now_ms());
        let mut guard = self.lock();
        let state = &mut *guard;
        let Phase::Live(hub) = &state.phase else {
            return Err((control::NOT_FOUND, "the session is not open".to_owned()));
        };
        let session = hub.session_id().to_owned();
        state.watch = match config {
            None => None,
            Some(config) => {
                let watch = Watch::new(config, client.id.as_str().to_owned(), now, baseline)
                    .map_err(|why| (control::BAD_PARAMS, why))?;
                Some(Watching { watch, client })
            }
        };
        tracing::info!(
            entry = self.entry,
            session,
            armed = state.watch.is_some(),
            "a session's watch was set"
        );
        Ok(json!({
            "session": session,
            "watch": state.watch.as_ref().map(|watching| watching.watch.status()),
        }))
    }

    /// What this session says of itself to `asker`, with when it was last active, or `None` when it is not open or the policy
    /// would turn `asker` away from attaching.
    fn describe(&self, asker: &Client) -> Option<(u64, Value)> {
        if self.ephemeral.load(Ordering::SeqCst) {
            return None;
        }
        if !matches!(self.access.check(asker, &Action::Attach), Verdict::Allow) {
            return None;
        }
        let state = self.lock();
        let Phase::Live(hub) = &state.phase else {
            return None;
        };
        let active = self.started_unix_ms.saturating_add(hub.last_activity());
        let clients: Vec<Value> = hub
            .attached()
            .map(|client| {
                json!({"id": client.id.as_str(), "label": client.label, "role": client.role})
            })
            .collect();
        let mut out = json!({
            "entry": self.entry,
            "session": hub.session_id(),
            "clients": clients,
            "lastActivity": active,
            "cursor": hub.head().to_string(),
            "turnRunning": hub.turn_running(),
            "permissionsWaiting": hub.pending_permissions().count(),
        });
        if let Some(title) = &state.title {
            out["title"] = json!(title);
        }
        if let Some(cwd) = &state.cwd {
            out["cwd"] = json!(cwd);
        }
        Some((active, out))
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
            watch,
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
                    boot.cwd = message
                        .pointer("/params/cwd")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| boot.cwd.take());
                    match method {
                        "session/new" => {
                            self.ephemeral
                                .store(crate::history::is_ephemeral(&message), Ordering::SeqCst);
                            boot.opening = Some((request.clone(), None));
                        }
                        "session/load" | "session/resume" => {
                            let named = message
                                .pointer("/params/sessionId")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            boot.restored = named.as_deref().and_then(|named| {
                                self.kept.as_ref()?.load(&self.entry, named, self.idle)
                            });
                            boot.opening_message = Some(message.clone());
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
                if message.get("method").and_then(Value::as_str) == Some("session/prompt")
                    && let Some(watching) = watch
                {
                    watching.watch.person_wrote();
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
        if let Some(title) = named_by(&message) {
            state.title = Some(title);
        }
        match &mut state.phase {
            Phase::Boot(_) => self.boot_from_agent(&mut state, bytes, &message, now),
            Phase::Live(hub) => {
                hub.poll(now);
                let before: BTreeSet<u64> = hub.pending_permissions().map(|ask| ask.id).collect();
                let outputs = hub.from_agent(&message);
                let ask = (message.get("method").and_then(Value::as_str)
                    == Some("session/request_permission"))
                .then(|| message.get("id"))
                .flatten()
                .map(ToString::to_string);
                let open: BTreeSet<u64> = hub.pending_permissions().map(|ask| ask.id).collect();
                let State { asked, .. } = &mut *state;
                asked.retain(|_, number| open.contains(number));
                if let Some(ask) = ask
                    && let Some(number) = open.difference(&before).next()
                {
                    asked.insert(ask, *number);
                }
                self.dispatch(&mut state, outputs);
            }
            Phase::Over => {}
        }
    }

    /// What the approvers answered for requests the hub holds: the hub answers the agent with it, and every client's card closes.
    /// An answer for a request the hub does not hold yet (the agent's message is still on its way here) is kept in `late` for a
    /// few seconds; one for a request a client answered first settles nothing.
    fn settle_approvals(&self, late: &mut Vec<(Decided, Instant)>) {
        let now = Instant::now();
        late.extend(self.approvers.take().into_iter().map(|each| (each, now)));
        if late.is_empty() {
            return;
        }
        let mut state = self.lock();
        for (decided, since) in core::mem::take(late) {
            let outputs = {
                let State { phase, asked, .. } = &mut *state;
                let Phase::Live(hub) = phase else {
                    continue;
                };
                let Some(number) = asked.remove(&decided.request.to_string()) else {
                    if now.duration_since(since) < HELD_ANSWER {
                        late.push((decided, since));
                    }
                    continue;
                };
                hub.poll(self.now_ms());
                match hub.decide(number, json!({"outcome": decided.outcome})) {
                    Ok(outputs) => outputs,
                    Err(error) => {
                        tracing::debug!(
                            entry = self.entry,
                            "an approver's answer settled nothing: {error}"
                        );
                        continue;
                    }
                }
            };
            tracing::info!(
                entry = self.entry,
                "a permission request was answered by an approver"
            );
            self.dispatch(&mut state, outputs);
        }
    }

    /// The agent's message before the session is open: passed on to the opening client unread, but for the answers that open it.
    fn boot_from_agent(&self, state: &mut State, bytes: &[u8], message: &Value, now: u64) {
        let State {
            phase,
            clients,
            id: session_id,
            initialize,
            cwd,
            keeper,
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
            // What the agent replays while it loads a session the host kept the log of is in that log already.
            let replay = message.get("method").and_then(Value::as_str) == Some("session/update");
            if replay && boot.restored.is_some() && boot.opening.is_some() {
                return;
            }
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
            .or_else(|| named.clone())
        else {
            boot.opening = None;
            return to_creator(bytes);
        };
        let answer = boot
            .answer
            .take()
            .unwrap_or_else(|| json!({"protocolVersion": 1, "agentCapabilities": {}}));
        let make = || {
            let mut builder = Hub::builder(opened.clone())
                .initialize_result(answer.clone())
                .policy(Arc::clone(&self.access))
                .permission_window_ms(self.permission_window_ms);
            if let Some(entries) = self.log_bounds.0 {
                builder = builder.max_log_entries(entries);
            }
            if let Some(bytes) = self.log_bounds.1 {
                builder = builder.max_log_bytes(bytes);
            }
            builder
        };
        // A session loaded by name goes on from the log the host kept of it, if it did; with none, its log starts past every
        // cursor an earlier log of it can have issued, so that a client holding one is refused rather than replayed from.
        let named_here = named.as_deref() == Some(opened.as_str());
        let kept_log = boot.restored.take().filter(|_| named_here);
        let mut restoring = kept_log.is_some();
        let log = kept_log
            .or_else(|| named_here.then(|| fresh_log(self.started_unix_ms.saturating_add(now))));
        let mut hub = match log.map(|log| make().restore_log(log)) {
            Some(Ok(builder)) => builder.build(),
            Some(Err(error)) => {
                restoring = false;
                tracing::warn!(
                    entry = self.entry,
                    session = opened,
                    "the log kept of a shared session could not be used, so it starts afresh: {error}"
                );
                make().build()
            }
            None => make().build(),
        };
        if restoring {
            tracing::info!(
                entry = self.entry,
                session = opened,
                head = %hub.head(),
                updates = hub.log_len(),
                "a shared session goes on from the log the host kept"
            );
        } else {
            // The answer reaches the client as the agent gave it; from here the hub speaks to it.
            to_creator(bytes);
        }
        hub.poll(now);
        if let Err(error) = hub.connect(boot.who.clone()) {
            tracing::warn!(
                entry = self.entry,
                "the session's opener was refused: {error}"
            );
            self.agent.close();
            return;
        }
        // A restored session's hub answers the client's own request, so its cursor is honoured; any other attaches the opener
        // live and its answer is the agent's.
        let attach = match (restoring, boot.opening_message.take()) {
            (true, Some(mut message)) => {
                message["id"] = json!(ATTACH_ID);
                message
            }
            _ => json!({"jsonrpc": "2.0", "id": ATTACH_ID, "method": "session/resume",
                "params": {"sessionId": opened}}),
        };
        let mut outputs = hub.from_client(&creator, &attach).unwrap_or_default();
        if restoring {
            for output in &mut outputs {
                if let Output::ToClient { client, message } = output
                    && *client == creator
                    && message.get("id").and_then(Value::as_str) == Some(ATTACH_ID)
                {
                    *message = load_answer(message, answered, result);
                }
            }
        }
        *initialize = Some(with_loading(answer));
        *keeper = self.kept.as_ref().and_then(|kept| match kept.begin(&self.entry, &hub) {
            Ok(keeper) => Some(keeper),
            Err(error) => {
                tracing::warn!(
                    entry = self.entry,
                    session = opened,
                    "the log of a shared session could not be kept, so it will not survive a restart: {error}"
                );
                None
            }
        });
        *session_id = Some(opened.clone());
        *cwd = boot.cwd.clone();
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
                    let answered = message
                        .get("method")
                        .is_none()
                        .then(|| message.get("id").and_then(Value::as_str))
                        .flatten();
                    if answered == Some(ATTACH_ID)
                        || answered.is_some_and(|id| {
                            state.watch_asks.remove(&(client.clone(), id.to_owned()))
                        })
                    {
                        continue;
                    }
                    let Some(slot) = state.clients.get(&client) else {
                        continue;
                    };
                    let bytes = marked_for(&client, message).to_string().into_bytes();
                    if bytes.len() > self.max_line {
                        tracing::warn!(
                            entry = self.entry,
                            "a message of {} bytes was not sent: it is over host.max_message_bytes ({})",
                            bytes.len(),
                            self.max_line
                        );
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
        if let Phase::Live(hub) = &state.phase {
            self.approvers.attach(hub.attached_count());
        }
        if let (Phase::Live(hub), Some(keeper)) = (&state.phase, &mut state.keeper) {
            keeper.sync(hub);
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
                "a permission request has waited for an answer past the entry's window; it stays open"
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

    /// What the driver does between messages: the hub's clock, the idle rule, and a watch whose check is due.
    fn tick(&self) {
        let watch_due = {
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
                return;
            }
            let at = self.unix_ms(now);
            state
                .watch
                .as_ref()
                .filter(|watching| watching.watch.due(at))
                .map(|watching| watching.watch.measures_processes())
        };
        if let Some(measure) = watch_due {
            self.watch_check(measure);
        }
    }

    /// A watch's due check. The agent's processes, when its conditions read them, are read before the session's lock is taken.
    fn watch_check(&self, measure: bool) {
        let pid = self.agent_pid.load(Ordering::Relaxed);
        let processes = (measure && pid != 0).then(|| (self.read_processes)(pid));
        let mut guard = self.lock();
        let state = &mut *guard;
        let now = self.now_ms();
        let at = self.unix_ms(now);
        let Phase::Live(hub) = &mut state.phase else {
            return;
        };
        let mut outputs = hub.poll(now);
        let sample = watch::Sample {
            quiet_ms: now.saturating_sub(hub.last_activity()),
            turn_running: hub.turn_running(),
            permissions_waiting: hub.pending_permissions().count(),
            processes,
        };
        // Armed again or taken away while the processes were read.
        let Some(watching) = state
            .watch
            .as_mut()
            .filter(|watching| watching.watch.due(at))
        else {
            return self.dispatch(state, outputs);
        };
        match watching.watch.check(at, &sample) {
            Check::Skip(why) => {
                tracing::debug!(entry = self.entry, "a watch did not prompt: {why}");
            }
            Check::Prompt(text) => {
                let (sent, refused) = Self::watch_prompt(state, &text);
                outputs.extend(sent);
                match refused {
                    Some(why) => {
                        tracing::info!(entry = self.entry, "a watch's prompt was refused: {why}");
                        if let Some(watching) = state.watch.as_mut() {
                            watching
                                .watch
                                .stop(format!("its prompt was refused: {why}"));
                        }
                    }
                    None => tracing::info!(entry = self.entry, "a watch prompted the session"),
                }
            }
        }
        self.dispatch(state, outputs);
    }

    /// Sends a watch's prompt as the key that armed it: through that key's attachment when it is attached, else attached for this
    /// one message and detached again, so the turn goes on for whoever is there. Returns what the hub has to send and, when the
    /// hub or the session's policy refused the prompt, why.
    fn watch_prompt(state: &mut State, text: &str) -> (Vec<Output>, Option<String>) {
        let Some(client) = state.watch.as_ref().map(|watching| watching.client.clone()) else {
            return (Vec::new(), None);
        };
        let Phase::Live(hub) = &mut state.phase else {
            return (Vec::new(), None);
        };
        state.watch_count = state.watch_count.saturating_add(1);
        let id = format!("{WATCH_ID}{}", state.watch_count);
        let session = hub.session_id().to_owned();
        let prompt = json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}});
        let mut outputs = Vec::new();
        if hub.attached().any(|each| each.id == client.id) {
            state.watch_asks.insert((client.id.clone(), id.clone()));
            let before = hub.head();
            match hub.from_client(&client.id, &prompt) {
                Ok(sent) => outputs = sent,
                Err(error) => return (outputs, Some(error.to_string())),
            }
            // The hub does not echo a prompt to the client that sent it, and this one's person did not type it: it is shown
            // the prompt as the others are.
            let echoed: Vec<Value> = hub.log_after(before).cloned().collect();
            outputs.extend(echoed.into_iter().map(|message| Output::ToClient {
                client: client.id.clone(),
                message,
            }));
        } else {
            if let Err(error) = hub.connect(client.clone()) {
                let why = match error {
                    HubError::Refused(why) => why,
                    other => other.to_string(),
                };
                return (outputs, Some(why));
            }
            let resume = json!({"jsonrpc": "2.0", "id": format!("{id}-attach"),
                "method": "session/resume", "params": {"sessionId": session}});
            for sent in [
                hub.from_client(&client.id, &resume),
                hub.from_client(&client.id, &prompt),
                hub.disconnect(&client.id),
            ] {
                outputs.extend(sent.unwrap_or_default());
            }
        }
        // A refusal is answered at once, under the prompt's id.
        let refused = outputs.iter().find_map(|output| match output {
            Output::ToClient {
                client: to,
                message,
            } if *to == client.id
                && message.get("id").and_then(Value::as_str) == Some(id.as_str()) =>
            {
                message
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            _ => None,
        });
        if refused.is_some() {
            state.watch_asks.remove(&(client.id, id));
        }
        (outputs, refused)
    }

    /// The agent's end: the hub says so to whoever is attached, and what they are sent is the last of it.
    fn finish(&self, host_stopping: bool) {
        let mut state = self.lock();
        // A session the host's stopping interrupted is kept to go on with; one that ended on its own, or by a client's word or
        // the idle rule, is over, and so is what was kept of it. What follows (the hub telling the clients) is not written.
        if let Some(keeper) = state.keeper.take()
            && !host_stopping
        {
            keeper.remove();
        }
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
        let mut late = Vec::new();
        loop {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            self.settle_approvals(&mut late);
            match Channel::receive(&self.agent, Some(POLL)) {
                Ok(Some(bytes)) => self.agent_said(&bytes),
                Err(Failure::Timeout) => self.tick(),
                Ok(None) | Err(_) => break,
            }
        }
        self.finish(stopped.load(Ordering::SeqCst));
    }
}

/// A presence message as one client reads it: its own entry says so (`"self": true`), since the hub's list is the same for all.
fn marked_for(client: &ClientId, mut message: Value) -> Value {
    if message.get("method").and_then(Value::as_str) != Some(coqui_acp_hub::PRESENCE) {
        return message;
    }
    if let Some(clients) = message
        .pointer_mut("/params/clients")
        .and_then(Value::as_array_mut)
    {
        for each in clients {
            if each.get("id").and_then(Value::as_str) == Some(client.as_str()) {
                each["self"] = json!(true);
            }
        }
    }
    message
}

/// The name an agent's `session_info_update` gives its session, cleaned for a person to read: control characters out, cut to 200.
fn named_by(message: &Value) -> Option<String> {
    if message.get("method").and_then(Value::as_str) != Some("session/update") {
        return None;
    }
    let update = message.pointer("/params/update")?;
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("session_info_update") {
        return None;
    }
    let title: String = update
        .get("title")?
        .as_str()?
        .chars()
        .filter(|ch| !ch.is_control())
        .take(200)
        .collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// A log with nothing in it that starts after every cursor an earlier log of the session can have issued: the time in
/// milliseconds, which no log reaches by counting one at a time.
fn fresh_log(unix_ms: u64) -> LogSnapshot {
    let head = unix_ms.to_string().parse::<Cursor>().unwrap_or_default();
    LogSnapshot::new(head, Vec::new())
}

/// The answer to a `session/load` or `session/resume` the hub took in the agent's place: the agent's own result (its modes,
/// models and options) with the hub's cursor, or the hub's refusal, under the client's request id.
fn load_answer(hub: &Value, request: &Value, agent: &Value) -> Value {
    if hub.get("error").is_some() {
        let mut refusal = hub.clone();
        refusal["id"] = request.clone();
        return refusal;
    }
    let mut result = if agent.is_object() {
        agent.clone()
    } else {
        json!({})
    };
    if let Some(own) = hub
        .pointer("/result/_meta")
        .and_then(|meta| meta.get(coqui_acp_hub::META_KEY))
    {
        if !result.get("_meta").is_some_and(Value::is_object) {
            result["_meta"] = json!({});
        }
        result["_meta"][coqui_acp_hub::META_KEY] = own.clone();
    }
    json!({"jsonrpc": "2.0", "id": request, "result": result})
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
    reach: super::Reach,
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
                (Some(method), Some(request)) if method.starts_with('_') => {
                    // An extension asked for before the client has said which session it takes part in (an app asks for the
                    // agent's usage as soon as it is connected): nothing answers it yet, and no client needs it to go on.
                    if channel
                        .send(&error_to(request, "not available until a session is open"))
                        .is_err()
                    {
                        return;
                    }
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
        reach,
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
    reach: super::Reach,
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
    let (driver, host) = local::pair(shared.max_line, Arc::clone(&shared.stopped));
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
        permission_window_ms: keep
            .permission_window
            .map(|window| u64::try_from(window.as_millis()).unwrap_or(u64::MAX)),
        started_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0)),
        agent: driver,
        approvers: Arc::default(),
        kept: keep.keep_log.then(|| shared.hosted.kept.clone()).flatten(),
        log_bounds: (keep.log_entries, keep.log_bytes),
        max_line: shared.max_line,
        access,
        agent_pid: Arc::new(AtomicU32::new(0)),
        ephemeral: AtomicBool::new(false),
        read_processes: crate::usage_splice::tree_use,
        state: Mutex::new(State {
            phase: Phase::Boot(Box::new(Boot {
                creator: id.clone(),
                who,
                initialize: request,
                answer: None,
                opening: None,
                swallow,
                cwd: None,
                opening_message: None,
                restored: None,
            })),
            clients: BTreeMap::new(),
            next_generation: 0,
            id: None,
            cwd: None,
            title: None,
            initialize: None,
            keeper: None,
            watch: None,
            watch_asks: BTreeSet::new(),
            asked: BTreeMap::new(),
            watch_count: 0,
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
        let pid = Arc::clone(&session.agent_pid);
        let approvers = Arc::clone(&session.approvers);
        thread::Builder::new()
            .name("canoa-session-agent".to_owned())
            .spawn(move || {
                let prefilled = cabuya::link::Prefilled::new(&host, first.clone());
                super::connection::give_agent_flow(
                    &shared,
                    &served,
                    &prefilled,
                    (&first, &peer),
                    (None, reach, Some(&pid), Some(&approvers)),
                );
                pid.store(0, Ordering::Relaxed);
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

    /// Lets every action: these tests are about the watch, not the policy.
    #[derive(Debug)]
    struct Anyone;

    impl Policy for Anyone {
        fn check(&self, _: &Client, _: &Action) -> Verdict {
            Verdict::Allow
        }
    }

    /// The session the probing reader looks at, and what it found: whether `state` was free while it read.
    static PROBED: Mutex<Option<Arc<Session>>> = Mutex::new(None);
    static FREE_WHILE_READING: AtomicBool = AtomicBool::new(false);
    static READS: AtomicUsize = AtomicUsize::new(0);

    fn probing_reader(_: u32) -> TreeUse {
        READS.fetch_add(1, Ordering::SeqCst);
        let probed = PROBED.lock().unwrap_or_else(PoisonError::into_inner);
        let session = probed.as_ref().expect("a session to probe");
        FREE_WHILE_READING.store(session.state.try_lock().is_ok(), Ordering::SeqCst);
        TreeUse::default()
    }

    fn unopened_session(read_processes: fn(u32) -> TreeUse) -> Arc<Session> {
        session_with_agent(read_processes).0
    }

    /// A session whose agent has not opened it, and the agent's end of the link.
    fn session_with_agent(read_processes: fn(u32) -> TreeUse) -> (Arc<Session>, Link) {
        let (driver, host) = local::pair(1024, Arc::new(AtomicBool::new(false)));
        let who = Client::new("key".to_owned());
        let session = Arc::new(Session {
            entry: "shared".to_owned(),
            epoch: Instant::now(),
            idle: None,
            permission_window_ms: None,
            started_unix_ms: 1_000_000,
            agent: driver,
            approvers: Arc::default(),
            kept: None,
            log_bounds: (None, None),
            max_line: 1024,
            access: Arc::new(Anyone),
            agent_pid: Arc::new(AtomicU32::new(4242)),
            ephemeral: AtomicBool::new(false),
            read_processes,
            state: Mutex::new(State {
                phase: Phase::Boot(Box::new(Boot {
                    creator: who.id.clone(),
                    who,
                    initialize: None,
                    answer: None,
                    opening: None,
                    swallow: Vec::new(),
                    cwd: None,
                    opening_message: None,
                    restored: None,
                })),
                clients: BTreeMap::new(),
                next_generation: 0,
                id: None,
                cwd: None,
                title: None,
                initialize: None,
                keeper: None,
                watch: None,
                watch_asks: BTreeSet::new(),
                asked: BTreeMap::new(),
                watch_count: 0,
            }),
        });
        (session, host)
    }

    /// What `inbox` holds now.
    fn read(inbox: &Inbox) -> Vec<Value> {
        inbox
            .queue
            .try_iter()
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    fn ask(id: u64) -> Vec<u8> {
        json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": {
            "sessionId": "s",
            "toolCall": {"toolCallId": "t", "title": "Write", "kind": "edit"},
            "options": [{"optionId": "yes", "name": "Allow", "kind": "allow_once"}],
        }})
        .to_string()
        .into_bytes()
    }

    /// An open session with the clients `a` and `b` attached.
    fn open_with_two_clients() -> (Arc<Session>, Link, [Inbox; 2]) {
        let (session, host) = session_with_agent(probing_reader);
        session.lock().phase =
            Phase::Live(Box::new(Hub::builder("s").policy(Arc::new(Anyone)).build()));
        let mut inboxes = Vec::new();
        for name in ["a", "b"] {
            let (outbox, inbox) = mailbox();
            session
                .join(Client::new(name.to_owned()), outbox)
                .expect("attached");
            let resume = json!({"jsonrpc": "2.0", "id": 1, "method": "session/resume",
                "params": {"sessionId": "s"}});
            session.client_said(&ClientId::from(name), resume.to_string().as_bytes());
            read(&inbox);
            inboxes.push(inbox);
        }
        let [a, b]: [Inbox; 2] = inboxes.try_into().ok().expect("two");
        (session, host, [a, b])
    }

    fn agent_got(host: &Link) -> Vec<Value> {
        let mut got = Vec::new();
        while let Ok(Some(bytes)) = Channel::receive(host, Some(Duration::from_millis(100))) {
            got.extend(serde_json::from_slice::<Value>(&bytes).ok());
        }
        got
    }

    #[test]
    fn an_approvers_answer_closes_every_clients_card_and_answers_the_agent_once() {
        let (session, host, [a, b]) = open_with_two_clients();
        session.agent_said(&ask(41));
        for inbox in [&a, &b] {
            let asked = read(inbox)
                .into_iter()
                .find(|message| message["method"] == "session/request_permission")
                .expect("every client is asked");
            assert!(
                asked["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("hub-perm-"))
            );
        }
        let chosen = json!({"outcome": "selected", "optionId": "yes"});
        session.approvers.decide(json!(41), chosen.clone());
        let mut late = Vec::new();
        session.settle_approvals(&mut late);
        assert!(late.is_empty());
        for inbox in [&a, &b] {
            let closed = read(inbox)
                .into_iter()
                .find(|message| message["method"] == "_dev.boriken.coqui.acp/hub_permission_closed")
                .expect("every card closes");
            assert_eq!(closed["params"]["outcome"], chosen);
        }
        let answers: Vec<Value> = agent_got(&host)
            .into_iter()
            .filter(|message| message["id"] == 41)
            .collect();
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["result"]["outcome"], chosen);
        // A client's answer after it is the hub's to drop: the agent is answered once.
        session.client_said(
            &ClientId::from("a"),
            json!({"jsonrpc": "2.0", "id": "hub-perm-1",
                "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}})
            .to_string()
            .as_bytes(),
        );
        assert!(agent_got(&host).iter().all(|message| message["id"] != 41));
    }

    #[test]
    fn an_answer_that_comes_before_the_hub_holds_the_request_waits_for_it() {
        let (session, host, [a, _b]) = open_with_two_clients();
        session
            .approvers
            .decide(json!(42), json!({"outcome": "cancelled"}));
        let mut late = Vec::new();
        session.settle_approvals(&mut late);
        assert_eq!(late.len(), 1, "kept for the request that is on its way");
        session.agent_said(&ask(42));
        session.settle_approvals(&mut late);
        assert!(late.is_empty());
        assert!(
            read(&a)
                .iter()
                .any(|message| message["method"] == "_dev.boriken.coqui.acp/hub_permission_closed")
        );
        assert!(agent_got(&host).iter().any(|message| message["id"] == 42));
    }

    #[test]
    fn an_answer_for_a_request_a_client_answered_first_settles_nothing() {
        let (session, host, [a, _b]) = open_with_two_clients();
        session.agent_said(&ask(43));
        let hub_id = read(&a)
            .into_iter()
            .find(|message| message["method"] == "session/request_permission")
            .expect("asked")["id"]
            .clone();
        session.client_said(
            &ClientId::from("a"),
            json!({"jsonrpc": "2.0", "id": hub_id,
                "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}})
            .to_string()
            .as_bytes(),
        );
        session
            .approvers
            .decide(json!(43), json!({"outcome": "cancelled"}));
        let mut late = Vec::new();
        session.settle_approvals(&mut late);
        let answers: Vec<Value> = agent_got(&host)
            .into_iter()
            .filter(|message| message["id"] == 43)
            .collect();
        assert_eq!(answers.len(), 1, "the client's answer alone: {answers:?}");
        assert_eq!(answers[0]["result"]["outcome"]["outcome"], "selected");
    }

    #[test]
    fn the_agents_processes_are_read_with_the_sessions_state_free_and_only_for_a_watch_that_reads_them()
     {
        let session = unopened_session(probing_reader);
        *PROBED.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&session));
        let arm = |params: Value| {
            let config = watch::Config::from_params(&params)
                .expect("a watch")
                .expect("enabled");
            let client = Client::new("key".to_owned());
            session.lock().watch = Some(Watching {
                watch: Watch::new(config, "key".to_owned(), 0, None).expect("armed"),
                client,
            });
        };

        // A watch that reads processes has them read while the lock is free.
        arm(json!({"everyMs": 1, "whileChildren": true}));
        let measure = session
            .lock()
            .watch
            .as_ref()
            .map(|w| w.watch.measures_processes());
        assert_eq!(measure, Some(true));
        session.watch_check(true);
        assert_eq!(READS.load(Ordering::SeqCst), 1);
        assert!(
            FREE_WHILE_READING.load(Ordering::SeqCst),
            "the session's state was held while the process table was read"
        );

        // One that does not read them never has them read.
        arm(json!({"everyMs": 1, "quietMs": 10}));
        let measure = session
            .lock()
            .watch
            .as_ref()
            .map(|w| w.watch.measures_processes());
        assert_eq!(measure, Some(false));
        session.watch_check(false);
        assert_eq!(READS.load(Ordering::SeqCst), 1);

        // Arming one that reads them takes its baseline, also with the lock free.
        FREE_WHILE_READING.store(false, Ordering::SeqCst);
        session
            .configure_watch(
                Client::new("key".to_owned()),
                &json!({"session": "s", "whileChildren": true}),
            )
            .expect_err("the session is not open yet");
        assert_eq!(READS.load(Ordering::SeqCst), 2);
        assert!(FREE_WHILE_READING.load(Ordering::SeqCst));
        PROBED.lock().unwrap_or_else(PoisonError::into_inner).take();
    }

    #[test]
    fn what_the_hub_adds_to_initialize_is_loading_and_resuming() {
        let answer = with_loading(json!({"protocolVersion": 1, "agentCapabilities": {}}));
        assert_eq!(answer["agentCapabilities"]["loadSession"], true);
        assert!(answer["agentCapabilities"]["sessionCapabilities"]["resume"].is_object());
    }
}
