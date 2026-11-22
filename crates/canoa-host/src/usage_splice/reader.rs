//! An agent's own quota for the usage splice. What reads it is the agent's `usage_reader` provider: an agent may serve its quota
//! only once a session is open, so this module decides when: once a session has been open for the reader's settle time with no
//! turn running, or when its first turn ends (with retries, since what serves the figures comes up a moment later), after each
//! prompt, and when a client asks and the last reading is old. Nothing is read while the session is starting or a first turn
//! runs: the agent is busy with its own start then. With no session open it starts the entry's program of its own, opens an
//! empty session, reads and ends it.
//!
//! A reading never runs on the relay thread: it lists sockets and the process table, which take a while. A worker thread
//! updates the entry's [`Quota`] and leaves an answer in the connection's [`Link`]; the splice delivers it on the next
//! tick of the connection.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use canoa::usage_reader::{
    Failure as ReadFailure, ProcessMembers, ReaderInfo, Retry, UsageReader, UsageReading,
};
use coqui_acp_api::guatiao::Value as GuatiaoValue;
use coqui_acp_api::guatiao::library::kind::Instance;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::{Source, UsageReport, UsageSupport};
use coqui_acp_bridge::agent::Agent as BridgeAgent;
use coqui_acp_bridge::relay::relay;
use coqui_acp_harness::UsageTracker;
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::members::members;
use super::{Inference, answer_of, lock, unix_now};

/// How many messages of the private agent's are read before its answers to the two requests come.
const PRIVATE_MESSAGES: usize = 4096;

/// The host's [`ProcessMembers`]: the processes of an agent's tree, from this machine's process table.
#[derive(Debug, Clone, Copy)]
struct Table;

impl ProcessMembers for Table {
    fn members(&self, root: u32) -> Result<Vec<u32>, ProviderError> {
        Ok(members(root))
    }
}

/// A `usage_reader` provider, as it described itself: when to read and how its buckets are named. It is one a bundled agent's crate
/// offers, or one a library or source of canoa.cfg offers.
#[derive(Clone, Debug)]
pub(crate) struct Reader {
    info: Arc<ReaderInfo>,
    /// The registry of the library that offers it: none for a bundled agent's.
    registry: Option<Arc<Registry>>,
}

/// The description of the reader `id` among those `registry` offers.
fn described(registry: &Registry, id: &str) -> Option<ReaderInfo> {
    registry.offers::<dyn UsageReader>().find_map(|offer| {
        offer
            .default_instance()
            .and_then(|reader| reader.describe().ok())
            .filter(|info| info.id == id && info.problem().is_none())
    })
}

impl Reader {
    /// The bundled reader whose id is `id`, or `None` when no bundled agent offers one.
    pub(crate) fn named(id: &str) -> Option<Self> {
        let registry = crate::vendors::bundled_providers()?;
        described(registry, id).map(|info| Self {
            info: Arc::new(info),
            registry: None,
        })
    }

    /// The reader whose id is `id` that one of `registries` offers, which are those of the libraries and sources of canoa.cfg.
    pub(crate) fn offered_by(id: &str, registries: &[Arc<Registry>]) -> Option<Self> {
        registries.iter().find_map(|registry| {
            described(registry, id).map(|info| Self {
                info: Arc::new(info),
                registry: Some(Arc::clone(registry)),
            })
        })
    }

    /// The ids of the bundled readers.
    pub(crate) fn ids() -> Vec<String> {
        let Some(registry) = crate::vendors::bundled_providers() else {
            return Vec::new();
        };
        Self::ids_in(registry)
    }

    /// The ids of the readers `registry` offers.
    pub(crate) fn ids_in(registry: &Registry) -> Vec<String> {
        registry
            .offers::<dyn UsageReader>()
            .filter_map(|offer| offer.default_instance()?.describe().ok())
            .filter(|info| info.problem().is_none())
            .map(|info| info.id)
            .collect()
    }

    /// Description of how the reader polls and names its buckets.
    pub(crate) fn info(&self) -> &ReaderInfo {
        &self.info
    }

    /// A reading of one agent's figures.
    pub(crate) fn open(&self) -> Result<Object<dyn UsageReading>, String> {
        let registry = match &self.registry {
            Some(registry) => registry.as_ref(),
            None => crate::vendors::bundled_providers().ok_or("no reader is loaded")?,
        };
        for offer in registry.offers::<dyn UsageReader>() {
            let Some(reader) = offer.default_instance() else {
                continue;
            };
            if reader.describe().is_ok_and(|info| info.id == self.info.id) {
                return reader
                    .open(Table.into_object())
                    .map_err(|error| error.message().to_owned());
            }
        }
        Err("the reader is no longer offered".to_owned())
    }

    /// A reading of what the agent stores, from the reader configured with `config` (the entry's account settings); a reader
    /// that takes no configuration is opened as it is.
    pub(crate) fn open_configured(&self, config: &Value) -> Result<Configured, String> {
        let registry = match &self.registry {
            Some(registry) => registry.as_ref(),
            None => crate::vendors::bundled_providers().ok_or("no reader is loaded")?,
        };
        for offer in registry.offers::<dyn UsageReader>() {
            let Some(reader) = offer.default_instance() else {
                continue;
            };
            if !reader.describe().is_ok_and(|info| info.id == self.info.id) {
                continue;
            }
            if !offer.builds_instances() {
                return reader
                    .open(Table.into_object())
                    .map(|reading| Configured {
                        reading,
                        _instance: None,
                        _library: self.registry.clone(),
                    })
                    .map_err(|error| error.message().to_owned());
            }
            let instance = offer
                .instantiate(&GuatiaoValue::from(
                    crate::provider::map_of(&crate::config::Settings::new(
                        config.as_object().cloned().unwrap_or_default(),
                    ))
                    .map_err(|why| format!("the reader's settings: {why}"))?,
                ))
                .map_err(|error| format!("the reader's settings: {error}"))?;
            let reading = instance
                .open(Table.into_object())
                .map_err(|error| error.message().to_owned())?;
            return Ok(Configured {
                reading,
                _instance: Some(instance),
                _library: self.registry.clone(),
            });
        }
        Err("the reader is no longer offered".to_owned())
    }

    /// How long a reading counts as current: a request younger than this is answered from it, and no private agent is started
    /// more often.
    fn cache(&self) -> Duration {
        Duration::from_millis(self.info.cache_ms)
    }

    /// How long a session is open with no turn running before its quota is read: the agent's own start and first turn go first.
    pub(crate) fn settle(&self) -> Duration {
        Duration::from_millis(self.info.settle_ms)
    }

    fn id(&self) -> &str {
        &self.info.id
    }
}

/// A reading opened from a configured reader: the reading is let go first, then the instance it came from, then the library.
pub(crate) struct Configured {
    pub(crate) reading: Object<dyn UsageReading>,
    _instance: Option<Instance<dyn UsageReader>>,
    _library: Option<Arc<Registry>>,
}

/// One agent's reading, opened from its reader and asked one thread at a time. A library's reading is let go before the library.
struct Opened {
    reading: Mutex<Object<dyn UsageReading>>,
    /// Keeps the library the reading came from loaded; declared after the reading, which is let go first.
    _library: Option<Arc<Registry>>,
}

impl Opened {
    fn new(reader: &Reader) -> Result<Self, String> {
        reader.open().map(|reading| Self {
            reading: Mutex::new(reading),
            _library: reader.registry.clone(),
        })
    }

    /// What the agent said when it opened a session.
    fn opened(&self, result: &str) {
        if let Err(error) = lock(&self.reading).opened(result) {
            tracing::debug!("usage reader: {}", error.message());
        }
    }

    /// The figures of the agent whose process is `root`: `Ok` once read, or why not.
    fn read(
        &self,
        root: u32,
        tree: bool,
    ) -> Result<Vec<coqui_acp_api::usage::Bucket>, ReadFailure> {
        match lock(&self.reading).read(root, tree) {
            Ok(reading) => match reading.failure {
                None => Ok(reading.buckets),
                Some(failure) => Err(failure),
            },
            Err(error) => Err(ReadFailure::new(error.message(), Retry::Never)),
        }
    }
}

impl core::fmt::Debug for Opened {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Opened")
    }
}

/// How to run an entry's agent, from its configuration, and who reads its quota.
#[derive(Clone, Debug)]
pub(crate) struct Launch {
    /// What reads the agent's quota.
    pub(crate) reader: Reader,
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

/// An entry's agent's usage: the buckets every connection read, shared by them.
#[derive(Debug)]
pub(crate) struct Quota {
    command: Launch,
    tracker: Mutex<UsageTracker>,
    read_at: Mutex<Option<Instant>>,
    private: Mutex<Private>,
    /// How long an open session with no turn waits before its first reading.
    settle: Duration,
    /// How many connections of this entry are opening a session.
    starting: AtomicUsize,
}

impl Quota {
    pub(crate) fn new(command: Launch) -> Self {
        let settle = command.reader.settle();
        let mut support = UsageSupport::none();
        support.sources = alloc::vec![Source::Reported, Source::Inferred];
        support.needs_session = true;
        Self {
            command,
            tracker: Mutex::new(UsageTracker::new(support, None)),
            read_at: Mutex::new(None),
            private: Mutex::new(Private::default()),
            settle,
            starting: AtomicUsize::new(0),
        }
    }

    /// As [`Quota::new`], with the wait before the first reading of an idle session set.
    #[cfg(test)]
    pub(crate) fn with_settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
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
        self.tracker()
            .replace(&self.command.reader.info.bucket_prefix, buckets, unix_now());
        *lock(&self.read_at) = Some(Instant::now());
    }

    /// Whether a reading is younger than the reader's cache time.
    pub(crate) fn fresh(&self) -> bool {
        lock(&self.read_at).is_some_and(|at| at.elapsed() < self.command.reader.cache())
    }

    /// Reads with a private agent on a thread of its own and runs `done` after, unless that is not allowed now: no
    /// directory to work in or no way to start it through the host, an entry that gives its agent an environment, or one
    /// started less than the reader's cache time ago, or while a session of this entry is starting (the figures as they stand answer
    /// then). A request that comes while one is reading waits for it. `false` says nothing was started and `done` will
    /// not run.
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
        if self.starting.load(Ordering::SeqCst) > 0 {
            return false;
        }
        {
            let mut private = lock(&self.private);
            if private.busy {
                private.waiting.push(Box::new(done));
                return true;
            }
            if private
                .last
                .is_some_and(|at| at.elapsed() < self.command.reader.cache())
            {
                return false;
            }
            private.busy = true;
            private.last = Some(Instant::now());
        }
        let quota = Arc::clone(self);
        let started = thread::Builder::new()
            .name("canoa-usage-private".to_owned())
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
                    tracing::debug!(
                        "usage reader {}, private agent: {why}",
                        quota.command.reader.id()
                    );
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
        let end = Instant::now() + Duration::from_millis(self.command.reader.info.private_limit_ms);
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
        let reading = Opened::new(&self.command.reader)?;
        reading.opened(&opened.to_string());
        // The agent was started for this, below nothing of the host's: its whole tree is looked through from the first try.
        let mut why = "the agent shows nothing to read".to_owned();
        for delay in &self.command.reader.info.private_retries_ms {
            thread::sleep(
                Duration::from_millis(*delay).min(end.saturating_duration_since(Instant::now())),
            );
            if Instant::now() >= end {
                break;
            }
            match reading.read(pid, true) {
                Ok(buckets) => {
                    self.apply(buckets);
                    return Ok(());
                }
                Err(failure) if failure.retry == Retry::NotYet => why = failure.why,
                Err(failure) => return Err(failure.why),
            }
        }
        Err(why)
    }
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
    reading: Opened,
    busy: AtomicBool,
    /// Usage requests waiting for the reading under way, answered after it.
    waiters: Mutex<Vec<Value>>,
    /// Whether this connection is counted in its entry's sessions that are starting.
    starting: AtomicBool,
    /// How many turns of this connection are running.
    running: AtomicUsize,
    /// Whether a turn has ended since the session opened.
    turned: AtomicBool,
}

impl Drop for Reading {
    fn drop(&mut self) {
        self.started();
    }
}

impl Reading {
    pub(crate) fn new(
        inference: Arc<Inference>,
        quota: Arc<Quota>,
        link: Arc<Link>,
        account: Option<String>,
        root: u32,
    ) -> Option<Self> {
        let reading = match Opened::new(&quota.command.reader) {
            Ok(reading) => reading,
            Err(why) => {
                tracing::debug!("usage reader {}: {why}", quota.command.reader.id());
                return None;
            }
        };
        Some(Self {
            inference,
            quota,
            link,
            account,
            root,
            reading,
            busy: AtomicBool::new(false),
            waiters: Mutex::new(Vec::new()),
            starting: AtomicBool::new(false),
            running: AtomicUsize::new(0),
            turned: AtomicBool::new(false),
        })
    }

    /// A session of this connection is being opened: the entry reads nothing privately until it is.
    pub(crate) fn starting(&self) {
        if !self.starting.swap(true, Ordering::SeqCst) {
            self.quota.starting.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The session's opening ended, well or not.
    pub(crate) fn started(&self) {
        if self.starting.swap(false, Ordering::SeqCst) {
            self.quota.starting.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// A turn of this connection began.
    pub(crate) fn turn_began(&self) {
        self.running.fetch_add(1, Ordering::SeqCst);
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
            .name("canoa-usage-reader".to_owned())
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
    fn read_now(&self) -> Result<(), ReadFailure> {
        self.quota.apply(self.reading.read(self.root, false)?);
        Ok(())
    }

    fn read_quietly(&self) {
        if let Err(failure) = self.read_now() {
            tracing::debug!(
                "usage reader {}: {}",
                self.quota.command.reader.id(),
                failure.why
            );
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

    /// A session opened and the agent answered with `result` (JSON text): once its first turn has ended, or it has been idle for the
    /// reader's settle time, reads, and again after each of the reader's retry times while the agent shows nothing to read or
    /// does not answer in time.
    pub(crate) fn opened(self: &Arc<Self>, result: Option<&str>) {
        self.started();
        if let Some(result) = result {
            self.reading.opened(result);
        }
        self.spawn(|me| {
            if !me.settle() {
                return;
            }
            let delays: Vec<Duration> = [Duration::ZERO]
                .into_iter()
                .chain(
                    me.quota
                        .command
                        .reader
                        .info
                        .retries_ms
                        .iter()
                        .map(|ms| Duration::from_millis(*ms)),
                )
                .collect();
            for (index, wait) in delays.iter().enumerate() {
                if !me.pause(*wait) {
                    return;
                }
                let read = me.read_now();
                me.flush_waiters();
                match read {
                    Ok(()) => return,
                    Err(failure)
                        if matches!(failure.retry, Retry::NotYet | Retry::NoAnswer)
                            && index + 1 < delays.len() => {}
                    Err(failure) => {
                        tracing::debug!(
                            "usage reader {}: {}",
                            me.quota.command.reader.id(),
                            failure.why
                        );
                        return;
                    }
                }
            }
        });
    }

    /// Waits until a turn has ended or the session has been open for the settle time with none running, or a client asks
    /// for figures; `false` when the connection ended meanwhile.
    fn settle(&self) -> bool {
        let begun = Instant::now();
        loop {
            if self.link.closed() {
                return false;
            }
            if self.turned.load(Ordering::SeqCst)
                || !lock(&self.waiters).is_empty()
                || (begun.elapsed() >= self.quota.settle
                    && self.running.load(Ordering::SeqCst) == 0)
            {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
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
        // A compare-exchange loop rather than `fetch_update`, which newer compilers deprecate and the minimum one lacks
        // the replacement for.
        let mut held = self.running.load(Ordering::SeqCst);
        while let Some(less) = held.checked_sub(1) {
            match self
                .running
                .compare_exchange(held, less, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(now) => held = now,
            }
        }
        self.turned.store(true, Ordering::SeqCst);
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
