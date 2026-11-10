//! Antigravity's quota for the usage splice. `agy acp` serves its own language server on loopback once a session is open, and
//! `coqui_acp_harness::antigravity` reads the account's remaining fraction and reset time for each model from it. This module
//! decides when: after a session opens (with retries, since the port comes up a moment later), after each prompt, and when a
//! client asks and the last reading is old. With no session open it starts an `agy acp` of its own, opens an empty session,
//! reads and ends it.
//!
//! A reading never runs on the relay thread: it lists sockets and the process table, which take a while. A worker thread
//! updates the entry's [`Quota`] and leaves an answer in the connection's [`Link`]; the splice delivers it on the next
//! tick of the connection.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_acp_api::{Source, UsageReport, UsageSupport};
use coqui_acp_bridge::agent::Agent as BridgeAgent;
use coqui_acp_bridge::relay::relay;
use coqui_acp_harness::UsageTracker;
use coqui_acp_harness::antigravity::{AcpModel, AgyQuota, BUCKET_PREFIX, ReadError, ReadLimits};
use coqui_web_channel::{Channel, Failure};
use serde_json::{Value, json};

use super::members::members;
use super::{Inference, answer_of, lock, unix_now};

/// How long a reading counts as current: a request younger than this is answered from it, and no private agent is started
/// more often.
pub(crate) const CACHE: Duration = Duration::from_secs(15);
/// How long after a session opens each further try waits, while the agent has no port yet.
const RETRIES: [Duration; 4] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
/// How long a private agent may take from start to end.
const PRIVATE_LIMIT: Duration = Duration::from_secs(40);
/// How long a private agent waits between tries for its port.
const PRIVATE_RETRIES: [Duration; 5] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];
/// How many messages of the private agent's are read before its answers to the two requests come.
const PRIVATE_MESSAGES: usize = 4096;

/// How to run an entry's agy, from its configuration.
#[derive(Clone, Debug)]
pub(crate) struct Antigravity {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    /// The host's state directory; a private agent needs it for its empty working directory.
    pub(crate) state_dir: Option<PathBuf>,
    /// Whether a private agent may be started: not for an entry whose profiles give the agent an environment of its own,
    /// since the account it reads would then be the host's, not the connection's.
    pub(crate) private: bool,
    /// Whether each connection reads a quota of its own: for an entry with profiles, whose connections may run as
    /// different accounts, so no connection is answered from another's reading.
    pub(crate) per_connection: bool,
    /// How a private agent is started and recorded with the host, so stopping the host ends it and none starts while it
    /// stops; none, and no private agent is started.
    pub(crate) starter: Option<Starter>,
}

/// A process the host started for a reading, and what to call once it has ended.
pub(crate) type Started = (Arc<BridgeAgent>, Box<dyn FnOnce() + Send>);

/// Starts an agent through the host, which records it: `Ok(None)` once the host is stopping.
#[derive(Clone)]
pub(crate) struct Starter(pub(crate) Arc<StartFn>);

/// What a [`Starter`] calls.
pub(crate) type StartFn =
    dyn Fn(&OsString, &[OsString]) -> std::io::Result<Option<Started>> + Send + Sync;

impl core::fmt::Debug for Starter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Starter")
    }
}

/// Runs its closure when dropped, also while a panic unwinds.
struct Finally<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Finally<F> {
    fn drop(&mut self) {
        if let Some(finish) = self.0.take() {
            finish();
        }
    }
}

#[derive(Default)]
struct Private {
    busy: bool,
    last: Option<Instant>,
    /// Requests that came while a private agent was reading, answered when it ends.
    waiting: Vec<Box<dyn FnOnce() + Send>>,
}

impl core::fmt::Debug for Private {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Private")
            .field("busy", &self.busy)
            .field("waiting", &self.waiting.len())
            .finish_non_exhaustive()
    }
}

/// An entry's agy usage: the buckets every connection read, shared by them.
#[derive(Debug)]
pub(crate) struct Quota {
    command: Antigravity,
    tracker: Mutex<UsageTracker>,
    read_at: Mutex<Option<Instant>>,
    private: Mutex<Private>,
}

impl Quota {
    pub(crate) fn new(command: Antigravity) -> Self {
        let mut support = UsageSupport::none();
        support.sources = alloc::vec![Source::Reported, Source::Inferred];
        support.needs_session = true;
        Self {
            command,
            tracker: Mutex::new(UsageTracker::new(support, None)),
            read_at: Mutex::new(None),
            private: Mutex::new(Private::default()),
        }
    }

    fn tracker(&self) -> MutexGuard<'_, UsageTracker> {
        self.tracker.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The buckets read so far, as of `now`.
    pub(crate) fn report(&self, now: i64) -> UsageReport {
        self.tracker().snapshot(None, now)
    }

    /// How many times the buckets changed.
    pub(crate) fn revision(&self) -> u64 {
        self.tracker().snapshot(None, 0).revision
    }

    /// Takes a complete reading.
    pub(crate) fn apply(&self, buckets: Vec<coqui_acp_api::usage::Bucket>) {
        self.tracker().replace(BUCKET_PREFIX, buckets, unix_now());
        *lock(&self.read_at) = Some(Instant::now());
    }

    /// Whether a reading is younger than [`CACHE`].
    pub(crate) fn fresh(&self) -> bool {
        lock(&self.read_at).is_some_and(|at| at.elapsed() < CACHE)
    }

    /// Reads with a private agent on a thread of its own and runs `done` after, unless that is not allowed now: no
    /// directory to work in or no way to start it through the host, an entry that gives its agent an environment, or one
    /// started less than [`CACHE`] ago. A request that comes while one is reading waits for it. `false` says nothing was
    /// started and `done` will not run.
    pub(crate) fn read_privately(self: &Arc<Self>, done: impl FnOnce() + Send + 'static) -> bool {
        let Some(directory) = self
            .command
            .state_dir
            .as_ref()
            .filter(|_| self.command.private && self.command.starter.is_some())
            .map(|state| state.join("usage-read"))
        else {
            return false;
        };
        {
            let mut private = lock(&self.private);
            if private.busy {
                private.waiting.push(Box::new(done));
                return true;
            }
            if private.last.is_some_and(|at| at.elapsed() < CACHE) {
                return false;
            }
            private.busy = true;
            private.last = Some(Instant::now());
        }
        let quota = Arc::clone(self);
        let started = thread::Builder::new()
            .name("canoa-agy-private".to_owned())
            .spawn(move || {
                // Whatever ends the reading, a panic included, frees the next one and answers who waited.
                let _finish = Finally(Some(|| {
                    let waiting = {
                        let mut private = lock(&quota.private);
                        private.busy = false;
                        core::mem::take(&mut private.waiting)
                    };
                    done();
                    for answer in waiting {
                        answer();
                    }
                }));
                if let Err(why) = quota.private_reading(&directory) {
                    tracing::debug!("antigravity usage, private agent: {why}");
                }
            });
        if started.is_err() {
            let waiting = {
                let mut private = lock(&self.private);
                private.busy = false;
                core::mem::take(&mut private.waiting)
            };
            for answer in waiting {
                answer();
            }
            return false;
        }
        true
    }

    /// Starts the entry's agent, opens an empty session in `directory`, reads, and ends the agent.
    fn private_reading(&self, directory: &Path) -> Result<(), String> {
        std::fs::create_dir_all(directory).map_err(|error| error.kind().to_string())?;
        let cwd = std::path::absolute(directory)
            .map_err(|error| error.kind().to_string())?
            .to_string_lossy()
            .into_owned();
        let starter = self.command.starter.as_ref().ok_or("no host to start it")?;
        let (agent, forget) = (starter.0)(&self.command.program, &self.command.args)
            .map_err(|error| format!("could not start: {}", error.kind()))?
            .ok_or("the host is stopping")?;
        let _ended = Finally(Some({
            let agent = Arc::clone(&agent);
            move || {
                let _ = agent.stop();
                forget();
            }
        }));
        let (to_agent, inbound) = mpsc::channel::<Vec<u8>>();
        let (outbound, from_agent) = mpsc::channel::<Vec<u8>>();
        let driver = Driver {
            inbound: Mutex::new(inbound),
            outbound: Mutex::new(Some(outbound)),
        };
        let end = Instant::now() + PRIVATE_LIMIT;
        let pid = agent.pid();
        thread::scope(|scope| {
            scope.spawn(|| {
                relay(&driver, &agent);
                // The agent ended or the link did: nothing more will come, which the conversation must hear.
                driver.hang_up();
            });
            let outcome = self.converse(&to_agent, &from_agent, &cwd, pid, end);
            // The link ends, and with it the relay.
            drop(to_agent);
            outcome
        })
    }

    fn converse(
        &self,
        to_agent: &mpsc::Sender<Vec<u8>>,
        from_agent: &mpsc::Receiver<Vec<u8>>,
        cwd: &str,
        pid: u32,
        end: Instant,
    ) -> Result<(), String> {
        let ask = |id: u64, method: &str, params: Value| -> Result<Value, String> {
            let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            to_agent
                .send(serde_json::to_vec(&request).map_err(|_| "unwritable".to_owned())?)
                .map_err(|_| "the agent ended".to_owned())?;
            for _ in 0..PRIVATE_MESSAGES {
                let wait = end.saturating_duration_since(Instant::now());
                let line = from_agent.recv_timeout(wait).map_err(|error| match error {
                    RecvTimeoutError::Timeout => format!("{method} took too long"),
                    RecvTimeoutError::Disconnected => "the agent ended".to_owned(),
                })?;
                let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if message.get("id") == Some(&json!(id)) && message.get("method").is_none() {
                    return if message.get("error").is_some() {
                        Err(format!("{method} was refused"))
                    } else {
                        Ok(message["result"].clone())
                    };
                }
            }
            Err(format!("{method} was not answered"))
        };
        ask(
            1,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )?;
        let opened = ask(2, "session/new", json!({"cwd": cwd, "mcpServers": []}))?;
        let models = models_of(&opened);
        let mut reader = AgyQuota::new(ReadLimits::default());
        for delay in PRIVATE_RETRIES {
            thread::sleep(delay.min(end.saturating_duration_since(Instant::now())));
            if Instant::now() >= end {
                break;
            }
            match reader.read(&members(pid), &models) {
                Ok(buckets) => {
                    self.apply(buckets);
                    return Ok(());
                }
                Err(ReadError::NoPort) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(ReadError::NoPort.to_string())
    }
}

/// The models a `session/new` answer lists: `models.availableModels[].{modelId, name}`.
pub(crate) fn models_of(result: &Value) -> Vec<AcpModel> {
    result["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model.get("modelId")?.as_str()?;
            let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
            Some(AcpModel::new(id, name))
        })
        .collect()
}

/// A [`Channel`] whose peer is the host itself: what the relay reads from the agent's output and what it writes to its input.
struct Driver {
    inbound: Mutex<mpsc::Receiver<Vec<u8>>>,
    outbound: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
}

impl Driver {
    /// Lets go of the way to the conversation, which then hears that nothing more comes.
    fn hang_up(&self) {
        self.outbound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

impl Channel for Driver {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.outbound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .ok_or(Failure::PathLost)?
            .send(message.to_vec())
            .map_err(|_| Failure::PathLost)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let inbound = self.inbound.lock().unwrap_or_else(PoisonError::into_inner);
        match inbound.recv_timeout(timeout.unwrap_or(Duration::from_secs(3600))) {
            Ok(message) => Ok(Some(message)),
            Err(RecvTimeoutError::Timeout) => Err(Failure::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn close(&self) {}
}

/// What the splice of one connection shares with the threads that read for it.
#[derive(Debug, Default)]
pub(crate) struct Link {
    outbox: Mutex<Vec<Vec<u8>>>,
    closed: AtomicBool,
}

impl Link {
    fn push(&self, message: Vec<u8>) {
        lock(&self.outbox).push(message);
    }

    /// What is waiting to go to the client.
    pub(crate) fn take(&self) -> Vec<Vec<u8>> {
        core::mem::take(&mut *lock(&self.outbox))
    }

    /// Puts back, ahead of anything newer, what could not go.
    pub(crate) fn restore(&self, mut unsent: Vec<Vec<u8>>) {
        let mut outbox = lock(&self.outbox);
        unsent.append(&mut outbox);
        *outbox = unsent;
    }

    /// Ends the threads that read for the connection.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Queues the answer to the usage request `id`: the entry's report as it stands now.
    pub(crate) fn answer(
        &self,
        inference: &Inference,
        quota: &Quota,
        id: &Value,
        account: Option<&str>,
    ) {
        let report = inference.report_with(unix_now(), account, Some(quota));
        if let Some(bytes) = answer_of(&report, id) {
            self.push(bytes);
        }
    }
}

/// The connection's own agent, read through the process it started.
#[derive(Debug)]
pub(crate) struct Reading {
    inference: Arc<Inference>,
    quota: Arc<Quota>,
    link: Arc<Link>,
    account: Option<String>,
    root: u32,
    reader: Mutex<AgyQuota>,
    models: Mutex<Vec<AcpModel>>,
    /// Whether the agent's port was not its own process's, so the whole tree is looked through.
    wide: AtomicBool,
    busy: AtomicBool,
    /// Usage requests waiting for the reading under way, answered after it.
    waiters: Mutex<Vec<Value>>,
}

impl Reading {
    pub(crate) fn new(
        inference: Arc<Inference>,
        quota: Arc<Quota>,
        link: Arc<Link>,
        account: Option<String>,
        root: u32,
    ) -> Self {
        Self {
            inference,
            quota,
            link,
            account,
            root,
            reader: Mutex::new(AgyQuota::new(ReadLimits::default())),
            models: Mutex::new(Vec::new()),
            wide: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            waiters: Mutex::new(Vec::new()),
        }
    }

    /// Runs `job` on a thread of its own unless a reading of this connection is under way.
    pub(crate) fn spawn(self: &Arc<Self>, job: impl FnOnce(&Self) + Send + 'static) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let me = Arc::clone(self);
        let started = thread::Builder::new()
            .name("canoa-agy-usage".to_owned())
            .spawn(move || {
                // However the job ends, a panic included, the next one may start and a request that waited is answered
                // with what there is.
                let again = Arc::clone(&me);
                let _release = Finally(Some(move || {
                    again.busy.store(false, Ordering::SeqCst);
                    again.flush_waiters();
                }));
                job(&me);
            });
        if started.is_err() {
            self.busy.store(false, Ordering::SeqCst);
            self.flush_waiters();
            return false;
        }
        true
    }

    /// One reading of the agent's own processes.
    fn read_now(&self) -> Result<(), ReadError> {
        let models = lock(&self.models).clone();
        let mut reader = lock(&self.reader);
        let wide = self.wide.load(Ordering::SeqCst);
        let mut read = reader.read(
            &if wide {
                members(self.root)
            } else {
                alloc::vec![self.root]
            },
            &models,
        );
        if !wide && matches!(read, Err(ReadError::NoPort | ReadError::NotOwned)) {
            let owners = members(self.root);
            if owners.len() > 1 {
                self.wide.store(true, Ordering::SeqCst);
                read = reader.read(&owners, &models);
            }
        }
        self.quota.apply(read?);
        Ok(())
    }

    fn read_quietly(&self) {
        if let Err(why) = self.read_now() {
            tracing::debug!("antigravity usage: {why}");
        }
        self.flush_waiters();
    }

    /// Answers the usage requests waiting, with the report as it stands.
    fn flush_waiters(&self) {
        let waiting = core::mem::take(&mut *lock(&self.waiters));
        for id in waiting {
            self.link
                .answer(&self.inference, &self.quota, &id, self.account.as_deref());
        }
    }

    /// A session opened: reads, again after 2, 4, 8 and 16 seconds while the agent has no port.
    pub(crate) fn opened(self: &Arc<Self>, models: Vec<AcpModel>) {
        if !models.is_empty() {
            *lock(&self.models) = models;
        }
        self.spawn(|me| {
            let delays: Vec<Duration> = [Duration::ZERO].into_iter().chain(RETRIES).collect();
            for (index, wait) in delays.iter().enumerate() {
                if !me.pause(*wait) {
                    return;
                }
                let read = me.read_now();
                me.flush_waiters();
                match read {
                    Ok(()) => return,
                    Err(ReadError::NoPort) if index + 1 < delays.len() => {}
                    Err(why) => {
                        tracing::debug!("antigravity usage: {why}");
                        return;
                    }
                }
            }
        });
    }

    /// Waits `wait`; `false` when the connection ended meanwhile.
    fn pause(&self, wait: Duration) -> bool {
        let end = Instant::now() + wait;
        while Instant::now() < end {
            if self.link.closed() {
                return false;
            }
            // A client asking for figures does not wait out the retries.
            if !lock(&self.waiters).is_empty() {
                break;
            }
            thread::sleep(
                Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())),
            );
        }
        !self.link.closed()
    }

    /// A prompt was answered, which may have spent some of the quota.
    pub(crate) fn prompted(self: &Arc<Self>) {
        self.spawn(Self::read_quietly);
    }

    /// Answers the usage request `id` after a reading: the one under way, or a new one.
    pub(crate) fn ask(self: &Arc<Self>, id: Value) {
        self.wait_for(id);
        self.spawn(Self::read_quietly);
    }

    /// Holds the usage request `id` until the reading under way ends.
    pub(crate) fn wait_for(&self, id: Value) {
        lock(&self.waiters).push(id);
    }
}
