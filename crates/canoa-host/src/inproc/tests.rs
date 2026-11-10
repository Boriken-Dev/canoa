use std::sync::mpsc::{Receiver, Sender};

use coqui_acp_api::Environment;
use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::{
    AccountCatalog, AccountRef, Channel as Provided, ExportPage, HarnessInfo, ImportReport,
    LoginOutcome, LoginPrompter, LoginRequest, RenewReport, RenewRequest, Requirement,
    SessionExport, SessionImport,
};
use serde_json::{Value, json};

use super::*;

/// A client's end of a connection, in memory: what it sends waits in `inbox`, what reaches it goes to `outbox`.
struct Memory {
    inbox: Mutex<Receiver<Option<Vec<u8>>>>,
    outbox: Mutex<Sender<Vec<u8>>>,
    refuse_sends: AtomicBool,
    /// Every send makes no progress, as a client that stopped reading.
    stall_sends: AtomicBool,
    /// The next receive panics, as anything on the thread that reads the client may.
    panic_receive: AtomicBool,
}

impl Channel for Memory {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if self.refuse_sends.load(Ordering::SeqCst) {
            return Err(Failure::PathLost);
        }
        if self.stall_sends.load(Ordering::SeqCst) {
            return Err(Failure::NoProgress);
        }
        let _ = self
            .outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(message.to_vec());
        Ok(())
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        assert!(
            !self.panic_receive.load(Ordering::SeqCst),
            "the thread that reads the client panics"
        );
        let inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        match inbox.recv_timeout(timeout.unwrap_or(Duration::from_secs(3600))) {
            Ok(message) => Ok(message),
            Err(RecvTimeoutError::Timeout) => Err(Failure::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn close(&self) {}
}

struct Client {
    send: Sender<Option<Vec<u8>>>,
    seen: Receiver<Vec<u8>>,
}

impl Client {
    fn say(&self, message: &Value) {
        self.send
            .send(Some(serde_json::to_vec(message).expect("JSON")))
            .expect("sent");
    }

    /// The next message the client was sent.
    fn next(&self) -> Value {
        let line = self
            .seen
            .recv_timeout(Duration::from_secs(20))
            .expect("a message in time");
        serde_json::from_slice(&line).expect("one JSON message")
    }
}

fn connection() -> (Memory, Client) {
    let (send, inbox) = mpsc::channel();
    let (outbox, seen) = mpsc::channel();
    (
        Memory {
            inbox: Mutex::new(inbox),
            outbox: Mutex::new(outbox),
            refuse_sends: AtomicBool::new(false),
            stall_sends: AtomicBool::new(false),
            panic_receive: AtomicBool::new(false),
        },
        Client { send, seen },
    )
}

/// What the stand-in provider saw of its connections.
#[derive(Default)]
struct Seen {
    /// The status each `close` carried, in order.
    closes: Mutex<Vec<u32>>,
    /// Requests it was started with that held a working root.
    roots: Mutex<Vec<Vec<String>>>,
}

/// A provider whose connection answers each message `echo` with `{"echo": ...}`, answers `end` by ending the connection with a
/// failure, `big` with a message over any limit, and closes the sink when the host closes.
struct Stand {
    seen: Arc<Seen>,
    /// It refuses to start a connection.
    refuses: bool,
}

struct StandChannel {
    sink: Mutex<Object<dyn ChannelSink>>,
    seen: Arc<Seen>,
}

impl Provided for StandChannel {
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        let message: Value = serde_json::from_slice(bytes)
            .map_err(|_| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "not JSON"))?;
        match message["method"].as_str() {
            Some("end") => {
                sink.closed(
                    u32::from(Status::GUATIAO_ERR_INTERNAL),
                    "the program exited with 3",
                );
                Ok(())
            }
            Some("big") => {
                // A sink that refuses a message ends the connection with its status and message.
                if let Err(refused) = sink.receive(&vec![b' '; 2 * 1024 * 1024]) {
                    sink.closed(u32::from(refused.status), refused.message());
                }
                Ok(())
            }
            _ => sink.receive(&serde_json::to_vec(&json!({"echo": message})).expect("JSON")),
        }
    }

    fn close(&self, status: u32, _reason: &str) {
        self.seen
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(status);
        self.sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed(status, "");
    }
}

impl AcpHarness for Stand {
    fn describe(&self) -> Result<HarnessInfo, ProviderError> {
        Ok(HarnessInfo::new("Stand", vec![1]))
    }

    fn accounts(&self) -> Result<AccountCatalog, ProviderError> {
        Ok(AccountCatalog::new())
    }

    fn start(
        &self,
        request: StartRequest,
        spawner: Object<dyn ProcessSpawner>,
        sink: Object<dyn ChannelSink>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn coqui_acp_api::Channel>, ProviderError> {
        drop((spawner, keeper));
        if self.refuses {
            return Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"));
        }
        self.seen
            .roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.working_roots.clone());
        Ok(StandChannel {
            sink: Mutex::new(sink),
            seen: Arc::clone(&self.seen),
        }
        .into_object())
    }

    fn refusal(
        &self,
        _requirements: Vec<Requirement>,
        _environment: Secret<Environment>,
        _account: Option<AccountRef>,
    ) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    fn program(&self) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    fn import_session(
        &self,
        _session: SessionImport,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<ImportReport, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }

    fn export_session(
        &self,
        _export: SessionExport,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<ExportPage, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }

    fn login(
        &self,
        _request: LoginRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }

    fn renew(
        &self,
        _request: RenewRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }
}

fn stand() -> (Arc<Stand>, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    (
        Arc::new(Stand {
            seen: Arc::clone(&seen),
            refuses: false,
        }),
        seen,
    )
}

fn served(
    channel: Arc<Memory>,
    harness: Arc<Stand>,
    control: Arc<InProcess>,
) -> thread::JoinHandle<End> {
    thread::spawn(move || {
        let request = StartRequest::builder().working_root("/root").build();
        serve_kind(&*channel, &*harness, request, 1024 * 1024, &control)
    })
}

fn closes(seen: &Seen) -> Vec<u32> {
    seen.closes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// How `running` ended, when it ended within `wait` without panicking.
fn ends_within(running: thread::JoinHandle<End>, wait: Duration) -> Option<End> {
    let (done, ended) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(running.join());
    });
    ended.recv_timeout(wait).ok().and_then(Result::ok)
}

#[test]
fn a_message_reaches_the_provider_and_its_answer_the_client_and_the_clients_close_closes_the_connection()
 {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let running = served(Arc::new(channel), harness, InProcess::new());
    client.say(&json!({"method": "echo", "id": 1}));
    assert_eq!(client.next()["echo"]["id"], 1);
    client.send.send(None).expect("closed");
    assert_eq!(running.join().expect("served"), End::PeerClosed);
    assert_eq!(closes(&seen), [0], "the host closed it in order");
    assert_eq!(
        seen.roots.lock().expect("lock").as_slice(),
        [vec!["/root".to_owned()]],
        "the request reached the provider"
    );
}

#[test]
fn a_message_holding_a_line_break_reaches_the_provider_whole() {
    let (channel, client) = connection();
    let (harness, _seen) = stand();
    let running = served(Arc::new(channel), harness, InProcess::new());
    client
        .send
        .send(Some(b"{\"method\": \"echo\",\n\"id\": 7}".to_vec()))
        .expect("sent");
    assert_eq!(client.next()["echo"]["id"], 7);
    client.send.send(None).expect("closed");
    assert_eq!(running.join().expect("served"), End::PeerClosed);
}

#[test]
fn stopping_it_closes_the_connection_and_returns_once_the_provider_has_said_it_is_closed() {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let control = InProcess::new();
    let running = served(Arc::new(channel), harness, Arc::clone(&control));
    client.say(&json!({"method": "echo", "id": 1}));
    client.next();
    control.stop();
    assert_eq!(closes(&seen), [0]);
    assert_eq!(running.join().expect("served"), End::Stopped);
    drop(client);
}

#[test]
fn a_client_that_takes_nothing_more_ends_the_connection_and_closes_the_provider() {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let channel = Arc::new(channel);
    let running = served(Arc::clone(&channel), harness, InProcess::new());
    client.say(&json!({"method": "echo", "id": 1}));
    client.next();
    channel.refuse_sends.store(true, Ordering::SeqCst);
    client.say(&json!({"method": "echo", "id": 2}));
    assert_eq!(running.join().expect("served"), End::Failed);
    assert_eq!(closes(&seen).len(), 1, "the provider was closed once");
    assert_ne!(closes(&seen), [0], "and told the client was gone");
}

#[test]
fn stopping_it_while_the_client_takes_nothing_returns_at_once() {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let channel = Arc::new(channel);
    let control = InProcess::new();
    let running = served(Arc::clone(&channel), harness, Arc::clone(&control));
    client.say(&json!({"method": "echo", "id": 1}));
    client.next();
    channel.stall_sends.store(true, Ordering::SeqCst);
    client.say(&json!({"method": "echo", "id": 2}));
    thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    control.stop();
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "stop waited {:?} on a client that takes nothing",
        asked.elapsed()
    );
    assert_eq!(closes(&seen).len(), 1);
    assert_eq!(
        ends_within(running, Duration::from_secs(5)),
        Some(End::Stopped)
    );
}

#[test]
fn a_provider_that_ends_the_connection_with_a_failure_ends_the_serving() {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let running = served(Arc::new(channel), harness, InProcess::new());
    client.say(&json!({"method": "end"}));
    assert_eq!(
        ends_within(running, Duration::from_secs(10)),
        Some(End::AgentFailed)
    );
    assert!(closes(&seen).is_empty(), "the host closed nothing");
    drop(client);
}

#[test]
fn a_message_of_the_providers_over_the_limit_ends_the_connection() {
    let (channel, client) = connection();
    let (harness, _seen) = stand();
    let running = served(Arc::new(channel), harness, InProcess::new());
    client.say(&json!({"method": "big"}));
    assert_eq!(
        ends_within(running, Duration::from_secs(10)),
        Some(End::Protocol)
    );
    drop(client);
}

#[test]
fn a_provider_that_refuses_to_start_ends_the_serving() {
    let (channel, client) = connection();
    let seen = Arc::new(Seen::default());
    let harness = Arc::new(Stand {
        seen,
        refuses: true,
    });
    let control = InProcess::new();
    let running = served(Arc::new(channel), harness, Arc::clone(&control));
    assert_eq!(running.join().expect("served"), End::AgentFailed);
    // Stopping waits for nothing: the serving finished.
    let asked = Instant::now();
    control.stop();
    assert!(asked.elapsed() < Duration::from_secs(1));
    drop(client);
}

#[test]
fn a_panic_on_the_thread_that_serves_ends_the_connection_and_stopping_it_waits_for_nothing() {
    let (channel, client) = connection();
    let (harness, seen) = stand();
    let channel = Arc::new(channel);
    let control = InProcess::new();
    let running = served(Arc::clone(&channel), harness, Arc::clone(&control));
    client.say(&json!({"method": "echo", "id": 1}));
    client.next();
    channel.panic_receive.store(true, Ordering::SeqCst);
    let ended = ends_within(running, Duration::from_secs(10));
    assert!(
        matches!(ended, Some(End::Failed | End::AgentFailed)),
        "the connection ended as {ended:?}"
    );
    assert_eq!(closes(&seen).len(), 1, "the provider was closed");
    let asked = Instant::now();
    control.stop();
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
}
