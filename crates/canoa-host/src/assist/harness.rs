//! The backend made of an agent harness the person has (`[assist_harness.NAME]`): Claude Code or Codex under their own login, for
//! when no API key is at hand. It completes a prompt in a session of the harness that can use no tool.
//!
//! Starting an agent costs more than a request, so the sessions are kept warm in one [`Pool`] for all the harness backends, keyed
//! by the harness, its account and its profile: started the first time they are asked for, reused by the next request, ended after
//! `[assist_pool] idle_seconds` unasked and never more than `max_sessions` at once. Each request is a **new conversation** in the
//! running agent (`session/new`), so no request sees another's text. A session that fails, times out or answers wrongly is ended and
//! the next request starts another.
//!
//! The session is given nothing of the person's: no loadout (so no overlay, skill or server of theirs), no tools of the host, no
//! file or terminal of the client, an empty private working folder, the mode of the agent that asks for the least, and every
//! permission request is refused. The harness's own system prompt stays; what the request says goes in the user turn.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use canoa::network::Network;
use canoa::sdk::{ChatReply, ChatRequest, capability};
use coqui_acp_api::{CapabilitiesSource, ModelInfo, ModelListing};
use serde_json::{Value, json};

use super::{Answer, Backend, Call, Failure, Operation};
use crate::config::{AssistPool, HarnessTable};
use crate::delegate::{ChildContext, Host, Principal, privilege};
use crate::local::{Link, LinkError};

/// How long a session nobody asked is kept, when `[assist_pool] idle_seconds` does not say.
pub(crate) const DEFAULT_IDLE: Duration = Duration::from_secs(120);
/// How many sessions are kept at once, when `[assist_pool] max_sessions` does not say.
pub(crate) const DEFAULT_SESSIONS: usize = 2;
/// How long a read waits before it looks at the deadline.
const POLL: Duration = Duration::from_millis(100);
/// How long a session has to answer `initialize` and `session/new`, whatever the request's own time.
const SETUP: Duration = Duration::from_secs(60);
/// The modes that ask for the least, nearest first: what the session is put in when its agent lists one. A permission request is
/// refused whatever the mode, so this only keeps the agent from trying.
const LEAST: [&str; 7] = [
    "plan",
    "dont_ask",
    "dontAsk",
    "read-only",
    "readonly",
    "default",
    "manual",
];

/// Where the backends reach the host to open a harness: set once the host has its agents.
pub(crate) type Opener = Arc<OnceLock<Arc<dyn Host>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a session could not answer.
struct Fail {
    why: String,
    /// Whether the connection to the agent is gone, so that a session taken from the pool was already dead.
    lost: bool,
    /// Whether the session is still good for the next request.
    keep: bool,
}

impl Fail {
    /// The agent refused this request, but its session is as it was.
    fn refused(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            lost: false,
            keep: true,
        }
    }

    /// The session is not to be trusted again.
    fn broken(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            lost: false,
            keep: false,
        }
    }

    /// This failure as the session's own: an agent that answers a request with an error, or not at all, is not trusted again.
    fn spoiled(self) -> Self {
        Self {
            keep: false,
            ..self
        }
    }

    /// This failure as the request's: an agent that answered with an error is as it was, but one that did not answer in time or
    /// went away is not.
    fn refused_if_answered(self) -> Self {
        if self.lost || !self.keep {
            self
        } else {
            Self::refused(self.why)
        }
    }

    fn gone(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            lost: true,
            keep: false,
        }
    }
}

/// One request to a session.
struct Ask<'a> {
    text: &'a str,
    model: Option<&'a str>,
    effort: Option<&'a str>,
}

/// What a session answered.
#[derive(Debug)]
pub(crate) struct Completion {
    pub(crate) text: String,
}

/// What a session of one harness is started with.
#[derive(Clone)]
struct Start {
    source: String,
    account: Option<String>,
    profile: Option<String>,
}

impl Start {
    fn key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.source,
            self.account.as_deref().unwrap_or(""),
            self.profile.as_deref().unwrap_or("")
        )
    }

    /// What the agent is sent in `initialize`: the account and profile, and the empty loadout, so none of the person's overlays,
    /// skills and servers apply.
    fn initialize(&self) -> Value {
        let mut meta = serde_json::Map::new();
        meta.insert(crate::profile::hosted::LOADOUT_META.to_owned(), json!(""));
        for (key, value) in [
            (crate::ACCOUNT_META, &self.account),
            (crate::profile::hosted::PROFILE_META, &self.profile),
        ] {
            if let Some(value) = value {
                meta.insert(key.to_owned(), json!(value));
            }
        }
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "canoa-assist", "version": env!("CARGO_PKG_VERSION")},
            "_meta": Value::Object(meta),
        })
    }
}

/// A running agent whose connection is kept between requests.
struct Conn {
    link: Link,
    next: u64,
    folder: PathBuf,
    /// How many permission requests the agent has made, each refused.
    refused: usize,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.link.close();
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

impl Conn {
    fn start(host: &dyn Host, start: &Start, name: &str) -> Result<Self, Fail> {
        let folder = std::env::temp_dir().join(format!("canoa-assist-{name}"));
        std::fs::create_dir_all(&folder)
            .map_err(|error| Fail::broken(format!("no working folder could be made: {error}")))?;
        let context = ChildContext {
            principal: Principal::Owner,
            roots: Vec::new(),
            depth: 1,
            parent_session: None,
        };
        let link = match host.open(&start.source, context) {
            Ok(link) => link,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                return Err(Fail::broken(format!(
                    "{} could not be opened: {error}",
                    start.source
                )));
            }
        };
        let mut conn = Self {
            link,
            next: 0,
            folder,
            refused: 0,
        };
        conn.call(
            "initialize",
            &start.initialize(),
            Instant::now() + SETUP,
            None,
        )?;
        Ok(conn)
    }

    /// Sends a request and reads until its answer, refusing what the agent asks meanwhile and keeping what it says in `reply`.
    fn call(
        &mut self,
        method: &str,
        params: &Value,
        until: Instant,
        mut reply: Option<&mut String>,
    ) -> Result<Value, Fail> {
        self.next += 1;
        let id = self.next;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.link
            .send(request.to_string().as_bytes())
            .map_err(|_| Fail::gone("the agent's connection ended"))?;
        loop {
            if Instant::now() >= until {
                // The turn is told to stop, and the session ended: it may be in the middle of anything.
                let _ = self.link.send(
                    json!({"jsonrpc": "2.0", "method": "session/cancel",
                        "params": {"sessionId": params.get("sessionId").cloned().unwrap_or(Value::Null)}})
                    .to_string()
                    .as_bytes(),
                );
                return Err(Fail::broken(format!(
                    "{method} took longer than the time given"
                )));
            }
            let wait = until.saturating_duration_since(Instant::now()).min(POLL);
            match self.link.receive(Some(wait)) {
                Ok(Some(bytes)) => {
                    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
                        continue;
                    };
                    if message.get("method").is_some() {
                        self.incoming(&message, reply.as_deref_mut());
                    } else if message.get("id") == Some(&json!(id)) {
                        return match message.get("error") {
                            Some(error) => Err(Fail::refused(format!(
                                "{method} failed: {}",
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("an error")
                            ))),
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                    }
                }
                Ok(None) | Err(LinkError::Ended | LinkError::TooLong { .. }) => {
                    return Err(Fail::gone("the agent's connection ended"));
                }
                Err(LinkError::Timeout) => {}
            }
        }
    }

    /// What the agent sent on its own: text to keep, a permission to refuse, or a request of the client's it has none to give.
    fn incoming(&mut self, message: &Value, reply: Option<&mut String>) {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        match (method, id) {
            ("session/update", None) => {
                let update = &message["params"]["update"];
                if update.get("sessionUpdate").and_then(Value::as_str)
                    == Some("agent_message_chunk")
                    && update.pointer("/content/type") == Some(&json!("text"))
                    && let (Some(text), Some(reply)) = (
                        update.pointer("/content/text").and_then(Value::as_str),
                        reply,
                    )
                {
                    reply.push_str(text);
                }
            }
            ("session/request_permission", Some(id)) => {
                self.refused += 1;
                let rejecting = message["params"]["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|option| {
                        option
                            .get("kind")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind.starts_with("reject"))
                    })
                    .and_then(|option| option.get("optionId").and_then(Value::as_str));
                let outcome = rejecting.map_or_else(
                    || json!({"outcome": "cancelled"}),
                    |option| json!({"outcome": "selected", "optionId": option}),
                );
                let _ = self.link.send(
                    json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
                        .to_string()
                        .as_bytes(),
                );
            }
            (_, Some(id)) => {
                let _ = self.link.send(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601,
                        "message": "this host gives a session of the assist service no files, terminal or other request of its own"}})
                    .to_string()
                    .as_bytes(),
                );
            }
            _ => {}
        }
    }

    /// One request as a new conversation: nothing of an earlier one is in it.
    fn complete(&mut self, ask: &Ask<'_>, until: Instant) -> Result<Completion, Fail> {
        let opened = self
            .call(
                "session/new",
                &json!({"cwd": self.folder.to_string_lossy(), "mcpServers": []}),
                until,
                None,
            )
            .map_err(Fail::spoiled)?;
        let Some(session) = opened
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(Fail::broken("the agent opened no session"));
        };
        let modes = privilege::modes_of(&opened);
        let current = modes.current.as_deref();
        if let Some(least) = LEAST
            .iter()
            .find(|mode| modes.available.iter().any(|offered| offered == *mode))
            && current != Some(*least)
        {
            let (method, params) = if modes.set_mode {
                (
                    "session/set_mode",
                    json!({"sessionId": session, "modeId": least}),
                )
            } else if let Some(option) = &modes.option {
                (
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": option, "value": least}),
                )
            } else {
                ("", Value::Null)
            };
            if !method.is_empty() {
                self.call(method, &params, until, None)
                    .map_err(Fail::spoiled)?;
            }
        }
        for (category, wanted, what) in [
            ("model", ask.model, "model"),
            ("thought_level", ask.effort, "effort"),
        ] {
            let Some(wanted) = wanted else { continue };
            let Some((option, values)) = privilege::option_of(&opened, category) else {
                return Err(Fail::refused(format!(
                    "the agent lists no {what} to choose, so {what} {wanted:?} was not set"
                )));
            };
            if !values.iter().any(|value| value == wanted) {
                return Err(Fail::refused(format!(
                    "{what} {wanted:?} is not one the agent offers (it offers: {})",
                    values.join(", ")
                )));
            }
            // The agent listed it and would not take it: this request is refused, and the session is as it was.
            self.call(
                "session/set_config_option",
                &json!({"sessionId": session, "configId": option, "value": wanted}),
                until,
                None,
            )
            .map_err(Fail::refused_if_answered)?;
        }
        let mut text = String::new();
        let turn = self
            .call(
                "session/prompt",
                &json!({"sessionId": session, "prompt": [{"type": "text", "text": ask.text}]}),
                until,
                Some(&mut text),
            )
            .map_err(Fail::spoiled)?;
        let stop = turn
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn");
        if !matches!(stop, "end_turn" | "max_tokens") {
            return Err(Fail::broken(format!(
                "the agent ended its turn with {stop}"
            )));
        }
        if text.trim().is_empty() {
            return Err(Fail::broken("the agent answered nothing"));
        }
        Ok(Completion { text })
    }
}

struct Warm {
    key: String,
    conn: Conn,
    since: Instant,
}

#[derive(Default)]
struct State {
    idle: Vec<Warm>,
    /// Sessions that exist: the idle ones, the ones serving a request and the ones being started.
    live: usize,
    closed: bool,
}

/// The warm sessions of every harness backend.
pub(crate) struct Pool {
    idle_for: Duration,
    cap: usize,
    state: Mutex<State>,
    changed: Condvar,
    /// Numbers the private folders the sessions work in.
    folders: AtomicU64,
}

enum Taken {
    /// A session already running for the key.
    Warm(Conn),
    /// A place for one more: the caller starts it and says [`Pool::lost`] if it cannot.
    Room,
}

impl Pool {
    pub(crate) fn new(settings: &AssistPool) -> Arc<Self> {
        Arc::new(Self {
            idle_for: settings
                .idle_seconds
                .map_or(DEFAULT_IDLE, Duration::from_secs),
            cap: settings.max_sessions.unwrap_or(DEFAULT_SESSIONS).max(1),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            folders: AtomicU64::new(0),
        })
    }

    /// A pool with its own times, for a test.
    #[cfg(test)]
    pub(crate) fn with(idle_for: Duration, cap: usize) -> Arc<Self> {
        Arc::new(Self {
            idle_for,
            cap,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            folders: AtomicU64::new(0),
        })
    }

    /// A name for the private folder of the next session: the process, this pool and a count, so no two sessions share one.
    fn folder_name(&self) -> String {
        let number = self.folders.fetch_add(1, Ordering::SeqCst);
        format!(
            "{}-{:x}-{number}",
            std::process::id(),
            core::ptr::from_ref(self) as usize
        )
    }

    /// How long a session may be idle.
    pub(crate) fn idle_for(&self) -> Duration {
        self.idle_for
    }

    /// How many sessions exist.
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        lock(&self.state).live
    }

    fn take(&self, key: &str, until: Instant) -> Result<Taken, String> {
        let mut evicted = None;
        let mut state = lock(&self.state);
        let taken = loop {
            if state.closed {
                return Err("the host is stopping".to_owned());
            }
            if let Some(at) = state.idle.iter().rposition(|warm| warm.key == key) {
                break Taken::Warm(state.idle.swap_remove(at).conn);
            }
            if state.live < self.cap {
                state.live += 1;
                break Taken::Room;
            }
            // At the cap: the session that has been idle longest, of another harness, makes room.
            if let Some(at) = state
                .idle
                .iter()
                .enumerate()
                .min_by_key(|(_, warm)| warm.since)
                .map(|(at, _)| at)
            {
                evicted = Some(state.idle.remove(at));
                break Taken::Room;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "all {} sessions are busy with other requests",
                    self.cap
                ));
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        drop(state);
        // Ending an agent can take a moment, and nobody waits for it under the lock.
        drop(evicted);
        Ok(taken)
    }

    fn give(&self, key: &str, conn: Conn) {
        let mut state = lock(&self.state);
        if state.closed {
            state.live = state.live.saturating_sub(1);
            drop(state);
            drop(conn);
        } else {
            state.idle.push(Warm {
                key: key.to_owned(),
                conn,
                since: Instant::now(),
            });
        }
        self.changed.notify_all();
    }

    /// A session that was started or taken is gone: its place is free.
    fn lost(&self) {
        let mut state = lock(&self.state);
        state.live = state.live.saturating_sub(1);
        drop(state);
        self.changed.notify_all();
    }

    /// Ends the sessions nobody has asked for `idle_seconds`.
    pub(crate) fn reap(&self) {
        let mut state = lock(&self.state);
        let (keep, old): (Vec<Warm>, Vec<Warm>) = core::mem::take(&mut state.idle)
            .into_iter()
            .partition(|warm| warm.since.elapsed() < self.idle_for);
        state.idle = keep;
        state.live = state.live.saturating_sub(old.len());
        drop(state);
        drop(old);
        self.changed.notify_all();
    }

    /// Ends every session, and starts none.
    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        let idle = core::mem::take(&mut state.idle);
        state.live = state.live.saturating_sub(idle.len());
        drop(state);
        drop(idle);
        self.changed.notify_all();
    }
}

/// One `[assist_harness.NAME]`.
pub(crate) struct HarnessBackend {
    id: String,
    title: String,
    location: String,
    table: HarnessTable,
    opener: Opener,
    pool: Arc<Pool>,
}

impl core::fmt::Debug for HarnessBackend {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HarnessBackend")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl HarnessBackend {
    pub(crate) fn new(name: &str, table: &HarnessTable, opener: Opener, pool: Arc<Pool>) -> Self {
        let on = table
            .profile
            .as_deref()
            .map_or_else(String::new, |profile| format!(" on the profile {profile}"));
        let account = table
            .account
            .as_deref()
            .map_or_else(String::new, |account| format!(", account {account}"));
        Self {
            id: format!("harness:{name}"),
            title: table
                .title
                .clone()
                .unwrap_or_else(|| format!("{}{on}{account}", table.source)),
            location: format!(
                "Each request's text goes to {}, which this computer runs under its own login{account}, and so to that vendor's model under that vendor's terms. It is asked with no tool, and a request for one is refused.",
                table.source
            ),
            table: table.clone(),
            opener,
            pool,
        }
    }

    fn start(&self) -> Start {
        Start {
            source: self.table.source.clone(),
            account: self.table.account.clone(),
            profile: self.table.profile.clone(),
        }
    }

    /// Completes `ask` in a session of the harness, started if none is warm; a warm one that turns out dead is replaced once.
    fn run(
        &self,
        host: &dyn Host,
        ask: &Ask<'_>,
        timeout: Duration,
    ) -> Result<Completion, Failure> {
        let start = self.start();
        let key = start.key();
        let mut retried = false;
        let waiting = Instant::now() + timeout;
        loop {
            let (mut conn, reused) = match self.pool.take(&key, waiting).map_err(Failure::new)? {
                Taken::Warm(conn) => (conn, true),
                Taken::Room => match Conn::start(host, &start, &self.pool.folder_name()) {
                    Ok(conn) => (conn, false),
                    Err(fail) => {
                        self.pool.lost();
                        return Err(Failure::new(fail.why));
                    }
                },
            };
            // The time is for the turn: starting an agent the first time is not taken from it.
            let until = Instant::now() + timeout;
            match conn.complete(ask, until) {
                Ok(done) => {
                    self.pool.give(&key, conn);
                    return Ok(done);
                }
                Err(fail) if fail.keep => {
                    self.pool.give(&key, conn);
                    return Err(Failure::new(fail.why));
                }
                Err(fail) => {
                    drop(conn);
                    self.pool.lost();
                    if fail.lost && reused && !retried {
                        retried = true;
                        continue;
                    }
                    return Err(Failure::new(fail.why));
                }
            }
        }
    }
}

/// What a request is as a prompt: the instructions it carries and then the conversation. An agent's own system prompt cannot be
/// replaced over ACP, so the request's goes first in the user turn.
fn prompt_of(request: &ChatRequest) -> String {
    let mut text = String::new();
    if let Some(system) = request
        .system
        .as_deref()
        .filter(|system| !system.trim().is_empty())
    {
        text.push_str(system);
        text.push_str("\n\n");
    }
    if let [only] = request.messages.as_slice() {
        text.push_str(&only.content);
    } else {
        for message in &request.messages {
            text.push_str(&message.role);
            text.push_str(": ");
            text.push_str(&message.content);
            text.push('\n');
        }
    }
    text
}

impl Backend for HarnessBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn location(&self) -> &str {
        &self.location
    }

    fn kind(&self) -> &'static str {
        "harness"
    }

    fn serves(&self, operation: Operation) -> bool {
        operation == Operation::Complete
    }

    fn serve(
        &self,
        _network: &Arc<Network>,
        call: Call,
        timeout: Duration,
    ) -> Result<Answer, Failure> {
        let Call::Complete(request) = call else {
            return Err(Failure::new("a harness only completes a prompt"));
        };
        let host = self
            .opener
            .get()
            .ok_or_else(|| Failure::new("the host has not started its agents yet"))?;
        let model = Some(request.model.as_str())
            .filter(|model| !model.trim().is_empty())
            .or(self.table.model.as_deref());
        let ask = Ask {
            text: &prompt_of(&request),
            model,
            effort: self.table.effort.as_deref(),
        };
        let done = self.run(host.as_ref(), &ask, timeout)?;
        Ok(Answer::Complete(ChatReply::new(done.text)))
    }

    fn models(&self, _network: &Arc<Network>) -> Result<ModelListing, Failure> {
        let host = self
            .opener
            .get()
            .ok_or_else(|| Failure::new("the host has not started its agents yet"))?;
        let listed = host
            .choices(
                &self.table.source,
                &Principal::Owner,
                self.table.account.as_deref(),
            )
            .ok_or_else(|| Failure::new(format!("{} lists no models", self.table.source)))?;
        let models = listed
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| {
                let id = model.get("id").and_then(Value::as_str)?;
                Some(
                    ModelInfo::new(id)
                        .with_capabilities([capability::CHAT], CapabilitiesSource::Listed),
                )
            })
            .collect();
        Ok(ModelListing::of(models))
    }

    fn reap(&self) {
        self.pool.reap();
    }

    fn close(&self) {
        self.pool.close();
    }
}

#[cfg(test)]
mod tests;
