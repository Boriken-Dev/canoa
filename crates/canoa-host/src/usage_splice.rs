//! Usage for a stdio agent that reports none. coqui-acp's usage extension is the agent's to speak; when its `initialize`
//! answer does not advertise it, this host stands in: it reads the agent's failed `session/prompt` answers and refusals for
//! the entry's limit phrases, keeps what it found as the entry's report, advertises the capability with the source
//! `inferred`, answers `_coqui_acp/usage` itself and tells the clients that asked when the report changes. An agent that
//! advertises the capability is never touched: its messages pass as they came.
//!
//! An entry that names `usage = "antigravity"` also has its agy's quota read ([`agy`]): the buckets join the report beside
//! any inferred hit, and the capability says `reported` as well as `inferred`.
//!
//! One [`Inference`] belongs to an entry and is shared by its connections, since a limit is the account's; one [`Splice`]
//! belongs to a connection and keeps what that client asked and which turns are open. The reader of an agent that reports real
//! figures feeds the same [`Inference`] report, which is why the report and the reading of a turn are apart.
//!
//! What the host reads is the agent's own words, which may echo what a client sent: a client whose prompt makes the agent
//! fail with its own text can set the entry's shared limit.
//!
//! The rule for how long a hit holds: a reset time the text gave ends it then; failing that a retry-after ends it that long
//! after it was seen; with neither it stays until a turn of the agent ends well. A later success does not clear a hit whose
//! reset has not passed, since a gateway may accept one request in a window it refused the one before.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_acp_api::{
    DEFAULT_LIMIT_PHRASES, LimitHit, Source, USAGE_CAPABILITY, USAGE_METHOD, USAGE_NOTIFICATION,
    USAGE_VERSION, UsageReport, infer_limit,
};
use coqui_acp_harness::antigravity::AcpModel;
use coqui_transport::Failure;
use serde_json::{Value, json};

mod agy;
mod members;

pub(crate) use agy::{Antigravity, Quota, Starter};
use agy::{Link, Reading};

/// JSON-RPC's code for a message the receiver cannot place.
const INTERNAL_ERROR: i64 = -32603;
/// How many open turns and sessions' words one connection keeps; a client past it is not read.
const MAX_OPEN: usize = 64;
/// How much of a refused turn's words are kept for the phrases; `infer_limit` reads no more than this, and less here.
const MAX_SAID: usize = 16 * 1024;

/// The current time as Unix seconds, from the host's own clock.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// When `hit` stops holding, in Unix seconds; `None` when only a good turn ends it.
fn lapses_at(hit: &LimitHit) -> Option<i64> {
    hit.resets_at.or_else(|| {
        let wait = hit.retry_after?;
        let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
        Some(
            hit.observed_at
                .saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX)),
        )
    })
}

#[derive(Debug, Default)]
struct Held {
    hit: Option<LimitHit>,
    revision: u64,
}

impl Held {
    /// Lets go of the hit once it lapsed; `true` when that changed the report.
    fn expire(&mut self, now: i64) -> bool {
        if self
            .hit
            .as_ref()
            .and_then(lapses_at)
            .is_some_and(|at| at <= now)
        {
            self.hit = None;
            self.revision += 1;
            return true;
        }
        false
    }
}

/// An entry's phrases and the report its failed turns made, shared by every connection to it.
#[derive(Debug, Default)]
pub(crate) struct Inference {
    /// `None` is [`DEFAULT_LIMIT_PHRASES`]; a list replaces them, and an empty one reads nothing.
    phrases: Option<Vec<String>>,
    held: Mutex<Held>,
    /// Set for an entry whose agy's quota is read, one quota for all its connections.
    quota: Option<Arc<Quota>>,
    /// Set for an entry whose connections may run as different accounts (it has profiles): each connection reads a quota of
    /// its own, so none is answered from another's reading.
    per_connection: Option<Antigravity>,
}

impl Inference {
    pub(crate) fn new(phrases: Option<Vec<String>>) -> Self {
        Self {
            phrases,
            held: Mutex::new(Held::default()),
            quota: None,
            per_connection: None,
        }
    }

    /// The phrases this entry reads failed turns for.
    pub(crate) fn phrases(&self) -> Option<Vec<String>> {
        self.phrases.clone()
    }

    /// This entry's agy reads its quota too, one for its connections.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_quota(mut self, quota: Quota) -> Self {
        self.quota = Some(Arc::new(quota));
        self
    }

    /// This entry's agy reads its quota as `command` says: one shared by the connections, or one for each when
    /// `per_connection`.
    #[must_use]
    pub(crate) fn with_antigravity(mut self, command: Antigravity) -> Self {
        if command.per_connection {
            self.per_connection = Some(command);
        } else {
            self.quota = Some(Arc::new(Quota::new(command)));
        }
        self
    }

    /// The quota a new connection reads into.
    fn quota_for(&self) -> Option<Arc<Quota>> {
        self.quota.clone().or_else(|| {
            self.per_connection
                .clone()
                .map(|command| Arc::new(Quota::new(command)))
        })
    }

    /// Whether anything can be read at all: with no phrases and no quota the host stays out of the agent's way entirely.
    fn active(&self) -> bool {
        self.quota.is_some()
            || self.per_connection.is_some()
            || self
                .phrases
                .as_ref()
                .is_none_or(|phrases| !phrases.is_empty())
    }

    /// Reads `text` from a failed turn; `true` when the report changed.
    pub(crate) fn observe(&self, text: &str, now: i64) -> bool {
        let found = match &self.phrases {
            None => infer_limit(text, DEFAULT_LIMIT_PHRASES),
            Some(phrases) => {
                let phrases: Vec<&str> = phrases.iter().map(String::as_str).collect();
                infer_limit(text, &phrases)
            }
        };
        let Some(mut hit) = found else {
            return false;
        };
        hit.observed_at = now;
        let mut held = lock(&self.held);
        let mut changed = held.expire(now);
        // A reset the text gave that is already past is not a limit now.
        if lapses_at(&hit).is_some_and(|at| at <= now) {
            return changed;
        }
        // The same words with no wait to move are the figures already held; a retry-after moves the end each time.
        let repeat = held.hit.as_ref().is_some_and(|old| {
            old.text == hit.text
                && old.resets_at == hit.resets_at
                && old.retry_after == hit.retry_after
                && old.retry_after.is_none()
        });
        if !repeat {
            held.hit = Some(hit);
            held.revision += 1;
            changed = true;
        }
        changed
    }

    /// A turn ended well: a hit with no end of its own was a limit that has lifted.
    pub(crate) fn succeeded(&self, now: i64) -> bool {
        let mut held = lock(&self.held);
        let mut changed = held.expire(now);
        if held
            .hit
            .as_ref()
            .is_some_and(|hit| lapses_at(hit).is_none())
        {
            held.hit = None;
            held.revision += 1;
            changed = true;
        }
        changed
    }

    /// The entry's report at `now`: numbered by the host, under the entry's `account`.
    #[cfg(test)]
    pub(crate) fn report(&self, now: i64, account: Option<&str>) -> UsageReport {
        self.report_with(now, account, self.quota.as_deref())
    }

    /// The same, with the buckets of `quota`, which is a connection's own where the entry gives each one.
    pub(crate) fn report_with(
        &self,
        now: i64,
        account: Option<&str>,
        quota: Option<&Quota>,
    ) -> UsageReport {
        let mut held = lock(&self.held);
        held.expire(now);
        let mut report = quota.map_or_else(UsageReport::new, |quota| quota.report(now));
        if let Some(hit) = held.hit.clone() {
            let inferred = UsageReport::inferred(hit);
            report.observed_at = report.observed_at.max(inferred.observed_at);
            report.buckets.extend(inferred.buckets);
            report.limit = inferred.limit;
        }
        report.sources = if quota.is_some() {
            vec![Source::Reported, Source::Inferred]
        } else {
            vec![Source::Inferred]
        };
        report.account = account.map(ToOwned::to_owned);
        // Each part's count only grows, so the sum does.
        report.revision = held.revision + quota.map_or(0, Quota::revision);
        report
    }

    /// The report's revision at `now`, after letting go of a hit that lapsed.
    fn revision(&self, now: i64, quota: Option<&Quota>) -> u64 {
        let mut held = lock(&self.held);
        held.expire(now);
        held.revision + quota.map_or(0, Quota::revision)
    }
}

/// How an agent's answer to a `session/prompt` ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// A JSON-RPC error, with its message and data as text.
    Failed(String),
    /// `stopReason: refusal`; the words are what the agent said during the turn.
    Refused,
    /// `stopReason: end_turn`.
    Succeeded,
    /// Cancelled, out of tokens or anything else: neither a limit nor proof there is none.
    Other,
}

/// What one message of the agent's said, applied only once it was sent on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The answer to `initialize` `id`: `Some(advertised)` says whether the agent advertised the usage capability, and
    /// `None` that the answer gave no capabilities to read, so the host stays out.
    Initialized {
        id: String,
        advertised: Option<bool>,
    },
    /// A piece of text the agent said in `session`.
    Said { session: String, text: String },
    /// The answer to the `session/prompt` `id`.
    Ended { id: String, outcome: Outcome },
    /// The answer to the `session/new` or `session/load` `id`: whether it opened a session, and the models it listed.
    Opened {
        id: String,
        ok: bool,
        models: Vec<AcpModel>,
    },
}

/// A message of the agent's as it goes on, and what to remember of it once it did.
#[derive(Debug)]
pub(crate) struct Seen {
    /// The message with the host's changes; `None` goes on as it came.
    pub(crate) rewritten: Option<Vec<u8>>,
    pub(crate) event: Option<Event>,
    /// The message is the agent's claim to speak for usage, which only an agent that advertised it may make: it goes no further.
    pub(crate) hush: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stand {
    /// No answer to `initialize` seen.
    #[default]
    Unknown,
    /// The agent advertises usage.
    Agent,
    /// The agent does not, and this host speaks for it.
    Host,
}

#[derive(Debug, Default)]
struct Conn {
    stand: Stand,
    /// The client asked for notifications in its `initialize`.
    notify: bool,
    /// Ids of `initialize` requests waiting for their answer.
    initializes: BTreeSet<String>,
    /// Ids of usage requests this host answered itself, whose answer only it may give.
    answered: BTreeSet<String>,
    /// Ids of `session/prompt` requests waiting for theirs, with the session each is for.
    prompts: BTreeMap<String, String>,
    /// Ids of `session/new` and `session/load` requests waiting for theirs, kept for an entry that reads a quota.
    opens: BTreeSet<String>,
    /// A session has been opened on this connection, which is when its agy serves its quota.
    opened: bool,
    /// What the agent said in a session during its open turn.
    said: BTreeMap<String, String>,
    /// The revision the client was last told.
    told: u64,
}

/// A connection's side of the splice.
#[derive(Debug)]
pub(crate) struct Splice {
    inference: Arc<Inference>,
    account: Option<String>,
    conn: Mutex<Conn>,
    /// What the threads reading for this connection leave for it.
    link: Arc<Link>,
    /// The connection's own agy, once its process is known.
    reading: Option<Arc<Reading>>,
    /// The quota read for this connection: the entry's, or its own where the entry gives each connection one.
    quota: Option<Arc<Quota>>,
    /// Held while a notice is built, sent and noted as told, so notices go out in order.
    notifying: Mutex<()>,
}

impl Drop for Splice {
    fn drop(&mut self) {
        self.link.close();
    }
}

/// What becomes of a message from the client that reached the splice.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Client {
    /// It goes to the agent.
    Forward,
    /// It stops here, and this is the answer.
    Answer(Vec<u8>),
    /// It stops here, and the answer comes later through [`Splice::tick`].
    Held,
}

/// A notice for the client and the revision it carries.
struct Notice {
    bytes: Vec<u8>,
    revision: u64,
}

impl Splice {
    pub(crate) fn new(inference: Arc<Inference>, account: Option<String>) -> Self {
        let quota = inference.quota_for();
        Self {
            inference,
            account,
            conn: Mutex::new(Conn::default()),
            link: Arc::new(Link::default()),
            reading: None,
            quota,
            notifying: Mutex::new(()),
        }
    }

    /// This connection's agent is the process `pid`, which is where its quota is read from, if the entry reads one.
    #[must_use]
    pub(crate) fn agent(mut self, pid: u32) -> Self {
        if let Some(quota) = &self.quota {
            self.reading = Some(Arc::new(Reading::new(
                Arc::clone(&self.inference),
                Arc::clone(quota),
                Arc::clone(&self.link),
                self.account.clone(),
                pid,
            )));
        }
        self
    }

    /// Reads a message from the client: it goes on to the agent, stops here with this host's own answer, or stops here with
    /// its answer to come once a reading ends.
    pub(crate) fn read_client(&self, message: &[u8]) -> Client {
        self.client(message).unwrap_or(Client::Forward)
    }

    fn client(&self, message: &[u8]) -> Option<Client> {
        if !self.inference.active()
            || !(contains(message, b"initialize")
                || contains(message, b"session/prompt")
                || contains(message, b"session/new")
                || contains(message, b"session/load")
                || contains(message, USAGE_METHOD.as_bytes())
                || !lock(&self.conn).answered.is_empty())
        {
            return None;
        }
        let value = serde_json::from_slice::<Value>(message).ok()?;
        let id = value.get("id")?;
        let key = id.to_string();
        let mut conn = lock(&self.conn);
        let method = value.get("method").and_then(Value::as_str)?;
        // A client may use an id again once its first request is answered.
        conn.answered.remove(&key);
        match method {
            "initialize" => {
                conn.notify = asks_notices(&value);
                if conn.initializes.len() < MAX_OPEN {
                    conn.initializes.insert(key);
                }
                None
            }
            "session/prompt" if conn.stand == Stand::Host => {
                let session = value.pointer("/params/sessionId")?.as_str()?.to_owned();
                if conn.prompts.len() < MAX_OPEN {
                    // What the agent said in an earlier turn of this session is not this turn's.
                    conn.said.remove(&session);
                    conn.prompts.insert(key, session);
                }
                None
            }
            "session/new" | "session/load" if conn.stand == Stand::Host && self.quota.is_some() => {
                if conn.opens.len() < MAX_OPEN {
                    conn.opens.insert(key);
                }
                None
            }
            // Until the agent's own answer shows it does not report usage, the request is the agent's to answer.
            method if method == USAGE_METHOD && conn.stand == Stand::Host => {
                if conn.answered.len() < MAX_OPEN {
                    conn.answered.insert(key);
                }
                let opened = conn.opened;
                drop(conn);
                if let Some(quota) = &self.quota {
                    // The figures are the agent's, which serves them while a session is open; a request after a reading
                    // younger than the cache is answered from it, and one that needs a reading waits for it.
                    if !quota.fresh() {
                        if opened && let Some(reading) = &self.reading {
                            reading.ask(id.clone());
                            return Some(Client::Held);
                        }
                        let link = Arc::clone(&self.link);
                        let inference = Arc::clone(&self.inference);
                        let (waiting, account) = (id.clone(), self.account.clone());
                        let figures = Arc::clone(quota);
                        if quota.read_privately(move || {
                            link.answer(&inference, &figures, &waiting, account.as_deref());
                        }) {
                            return Some(Client::Held);
                        }
                    }
                }
                let report = self.inference.report_with(
                    unix_now(),
                    self.account.as_deref(),
                    self.quota.as_deref(),
                );
                answer_of(&report, id).map(Client::Answer)
            }
            _ => None,
        }
    }

    /// Reads a message from the agent without changing anything: what to send in its place, and what to remember of it.
    pub(crate) fn inspect(&self, message: &[u8]) -> Seen {
        let pass = Seen {
            rewritten: None,
            event: None,
            hush: false,
        };
        if !self.inference.active() {
            return pass;
        }
        let (stand, open, waiting) = {
            let conn = lock(&self.conn);
            (
                conn.stand,
                !conn.prompts.is_empty(),
                !conn.initializes.is_empty()
                    || !conn.prompts.is_empty()
                    || !conn.opens.is_empty()
                    || !conn.answered.is_empty(),
            )
        };
        if stand == Stand::Agent {
            return pass;
        }
        let notice = contains(message, USAGE_NOTIFICATION.as_bytes());
        let chunk = open && contains(message, b"agent_message_chunk");
        let answer = waiting && contains(message, b"\"id\"");
        if !(notice || chunk || answer) {
            return pass;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return pass;
        };
        if notice && value.get("method").and_then(Value::as_str) == Some(USAGE_NOTIFICATION) {
            return Seen { hush: true, ..pass };
        }
        if chunk && value.get("method").and_then(Value::as_str) == Some("session/update") {
            let update = &value["params"]["update"];
            if update["sessionUpdate"] == "agent_message_chunk"
                && let (Some(session), Some(text)) = (
                    value["params"]["sessionId"].as_str(),
                    update["content"]["text"].as_str(),
                )
            {
                return Seen {
                    rewritten: None,
                    event: Some(Event::Said {
                        session: session.to_owned(),
                        text: text.to_owned(),
                    }),
                    hush: false,
                };
            }
            return pass;
        }
        if value.get("method").is_some() {
            return pass;
        }
        let Some(key) = value.get("id").map(ToString::to_string) else {
            return pass;
        };
        let conn = lock(&self.conn);
        if conn.answered.contains(&key) {
            return Seen { hush: true, ..pass };
        }
        if conn.initializes.contains(&key) {
            drop(conn);
            return Self::initialized(&mut value, key, self.quota.is_some());
        }
        if conn.opens.contains(&key) {
            drop(conn);
            let ok = value.get("error").is_none();
            let models = value.get("result").map(agy::models_of).unwrap_or_default();
            return Seen {
                rewritten: None,
                event: Some(Event::Opened {
                    id: key,
                    ok,
                    models,
                }),
                hush: false,
            };
        }
        if conn.prompts.contains_key(&key) {
            drop(conn);
            return Seen {
                rewritten: None,
                event: Some(Event::Ended {
                    outcome: outcome_of(&value),
                    id: key,
                }),
                hush: false,
            };
        }
        pass
    }

    /// The answer to `initialize`: left alone when it advertises usage, else with the capability added.
    fn initialized(value: &mut Value, id: String, reported: bool) -> Seen {
        let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) else {
            return Seen {
                rewritten: None,
                event: Some(Event::Initialized {
                    id,
                    advertised: None,
                }),
                hush: false,
            };
        };
        let key = if result.get("protocolVersion").and_then(Value::as_u64) >= Some(2) {
            "capabilities"
        } else {
            "agentCapabilities"
        };
        let advertised = ["agentCapabilities", "capabilities"].iter().any(|key| {
            result
                .get(*key)
                .and_then(|capabilities| capabilities.get("_meta")?.get(USAGE_CAPABILITY))
                .is_some()
        });
        if advertised {
            return Seen {
                rewritten: None,
                event: Some(Event::Initialized {
                    id,
                    advertised: Some(true),
                }),
                hush: false,
            };
        }
        let capabilities = result.entry(key).or_insert_with(|| json!({}));
        let added = capabilities.as_object_mut().and_then(|capabilities| {
            let meta = capabilities.entry("_meta").or_insert_with(|| json!({}));
            meta.as_object_mut()?.insert(
                USAGE_CAPABILITY.to_owned(),
                if reported {
                    json!({"version": USAGE_VERSION, "sources": ["reported", "inferred"], "spend": false,
                        "needs_session": true})
                } else {
                    json!({"version": USAGE_VERSION, "sources": ["inferred"], "spend": false})
                },
            );
            Some(())
        });
        let rewritten = added.and_then(|()| serde_json::to_vec(&*value).ok());
        Seen {
            event: Some(Event::Initialized {
                id,
                advertised: rewritten.as_ref().map(|_| false),
            }),
            rewritten,
            hush: false,
        }
    }

    /// Records what a turn's end says of the entry's limit. It runs before the answer reaches the client, which may ask for
    /// the report at once and must find the limit in it.
    fn learn(&self, event: &Event) {
        let Event::Ended { id, outcome } = event else {
            return;
        };
        let now = unix_now();
        match outcome {
            Outcome::Failed(text) => self.inference.observe(text, now),
            Outcome::Refused => {
                let said = {
                    let conn = lock(&self.conn);
                    conn.prompts
                        .get(id)
                        .and_then(|session| conn.said.get(session))
                        .cloned()
                };
                said.is_some_and(|said| self.inference.observe(&said, now))
            }
            Outcome::Succeeded => self.inference.succeeded(now),
            Outcome::Other => false,
        };
    }

    /// Applies what [`Splice::inspect`] saw, once the message was sent on.
    pub(crate) fn commit(&self, event: Event) {
        let mut conn = lock(&self.conn);
        match event {
            Event::Initialized { id, advertised } => {
                conn.initializes.remove(&id);
                match advertised {
                    Some(true) => conn.stand = Stand::Agent,
                    Some(false) => conn.stand = Stand::Host,
                    None => {}
                }
            }
            Event::Said { session, text } => {
                // Only a session with a turn open: a replay of another is not words of this turn.
                if conn.prompts.values().any(|open| *open == session)
                    && (conn.said.len() < MAX_OPEN || conn.said.contains_key(&session))
                {
                    let said = conn.said.entry(session).or_default();
                    if said.len() < MAX_SAID {
                        let room = MAX_SAID - said.len();
                        let cut = (0..=room.min(text.len()))
                            .rev()
                            .find(|at| text.is_char_boundary(*at))
                            .unwrap_or(0);
                        said.push_str(&text[..cut]);
                    }
                }
            }
            Event::Opened { id, ok, models } => {
                conn.opens.remove(&id);
                if ok {
                    conn.opened = true;
                    drop(conn);
                    if let Some(reading) = &self.reading {
                        reading.opened(models);
                    }
                }
            }
            Event::Ended { id, .. } => {
                if let Some(session) = conn.prompts.remove(&id) {
                    conn.said.remove(&session);
                }
                let opened = conn.opened;
                drop(conn);
                if opened && let Some(reading) = &self.reading {
                    reading.prompted();
                }
            }
        }
    }

    /// The `_coqui_acp/usage_update` this client is owed, if it asked for them and the report moved since it was told.
    fn notice(&self, now: i64) -> Option<Notice> {
        {
            let conn = lock(&self.conn);
            if !(conn.notify && conn.stand == Stand::Host) {
                return None;
            }
            if self.inference.revision(now, self.quota.as_deref()) == conn.told {
                return None;
            }
        }
        let report =
            self.inference
                .report_with(now, self.account.as_deref(), self.quota.as_deref());
        let revision = report.revision;
        let message = json!({"jsonrpc": "2.0", "method": USAGE_NOTIFICATION,
            "params": {"report": serde_json::to_value(&report).ok()?}});
        Some(Notice {
            bytes: serde_json::to_vec(&message).ok()?,
            revision,
        })
    }

    /// Sends one of the agent's messages with `send`, as this host stands to it: rewritten when it must be, remembered once
    /// it went, followed by a notice when the client is owed one. A message `send` could not take is not remembered, so
    /// the caller may offer it again.
    pub(crate) fn relay(
        &self,
        message: &[u8],
        send: impl Fn(&[u8]) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        if !self.inference.active() {
            return send(message);
        }
        let seen = self.inspect(message);
        if seen.hush {
            return Ok(());
        }
        if let Some(event) = &seen.event {
            self.learn(event);
        }
        send(seen.rewritten.as_deref().unwrap_or(message))?;
        if let Some(event) = seen.event {
            self.commit(event);
        }
        self.notify_if_owed(&send);
        Ok(())
    }

    /// Delivers what a reading left for the client: the answers to its usage requests, then a notice if one is owed. Called
    /// when the connection has a moment, since a reading ends on a thread that cannot write to the client. What does not go
    /// now stays for the next call.
    pub(crate) fn tick(&self, send: impl Fn(&[u8]) -> Result<(), Failure>) {
        if !self.inference.active() {
            return;
        }
        let mut answers = self.link.take().into_iter();
        let mut unsent = Vec::new();
        for answer in answers.by_ref() {
            if send(&answer).is_err() {
                unsent.push(answer);
                break;
            }
        }
        unsent.extend(answers);
        if !unsent.is_empty() {
            self.link.restore(unsent);
            return;
        }
        self.notify_if_owed(&send);
    }

    /// Sends the notice the client is owed, if any. One that does not go is owed still, since the client's last-told
    /// revision did not move.
    fn notify_if_owed(&self, send: &impl Fn(&[u8]) -> Result<(), Failure>) {
        // The relay's thread and the connection's tick both come here: one at a time, so a notice built from an older
        // report is never sent after a newer one, and `told` only grows.
        let _one = lock(&self.notifying);
        if let Some(notice) = self.notice(unix_now())
            && send(&notice.bytes).is_ok()
        {
            let mut conn = lock(&self.conn);
            conn.told = conn.told.max(notice.revision);
        }
    }
}

/// The answer to the usage request `id`: `report` as the result, or an error when it cannot be written.
fn answer_of(report: &UsageReport, id: &Value) -> Option<Vec<u8>> {
    let answer = match serde_json::to_value(report) {
        Ok(report) => json!({"jsonrpc": "2.0", "id": id, "result": report}),
        Err(_) => json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": INTERNAL_ERROR, "message": "the report could not be written"}}),
    };
    serde_json::to_vec(&answer).ok()
}

/// How an agent's answer to a prompt ended.
fn outcome_of(answer: &Value) -> Outcome {
    if let Some(error) = answer.get("error") {
        let mut text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(data) = error.get("data") {
            text.push('\n');
            match data {
                Value::String(data) => text.push_str(data),
                other => text.push_str(&other.to_string()),
            }
        }
        return Outcome::Failed(text);
    }
    match answer["result"]["stopReason"].as_str() {
        Some("refusal") => Outcome::Refused,
        Some("end_turn") => Outcome::Succeeded,
        _ => Outcome::Other,
    }
}

/// Whether an `initialize` that passed the scope asks for usage notices.
fn asks_notices(request: &Value) -> bool {
    ["clientCapabilities", "capabilities"].iter().any(|key| {
        request["params"][*key]["_meta"][USAGE_CAPABILITY]["notify"] == Value::Bool(true)
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests;
