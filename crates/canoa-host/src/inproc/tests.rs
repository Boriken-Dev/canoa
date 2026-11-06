use std::sync::mpsc::{Receiver, Sender};

use coqui_acp_harness::event::{Stop, TurnEnd};
use coqui_acp_harness::{
    AgentInfo, ConfigOption, Conversation, Event, EventSink, HarnessError, OpenRequest, Takes,
};
use serde_json::{Value, json};

use super::*;

/// A client's end of a connection, in memory: what it sends waits in `inbox`, what reaches it goes to `outbox`.
struct Memory {
    inbox: Mutex<Receiver<Option<Vec<u8>>>>,
    outbox: Mutex<Sender<Vec<u8>>>,
    refuse_sends: AtomicBool,
}

impl Channel for Memory {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if self.refuse_sends.load(Ordering::SeqCst) {
            return Err(Failure::PathLost);
        }
        let _ = self
            .outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(message.to_vec());
        Ok(())
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
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

    /// Messages until one with this id, which it answers.
    fn answer(&self, id: u64) -> (Value, Vec<Value>) {
        let mut before = Vec::new();
        loop {
            let line = self
                .seen
                .recv_timeout(Duration::from_secs(20))
                .expect("an answer in time");
            let message: Value = serde_json::from_slice(&line).expect("one JSON message per line");
            if message.get("id") == Some(&json!(id)) && message.get("method").is_none() {
                return (message, before);
            }
            before.push(message);
        }
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
        },
        Client { send, seen },
    )
}

/// A translator whose sessions say `hello` to each prompt, and count how many of them were closed.
#[derive(Default)]
struct Hello {
    closed: Arc<Mutex<u32>>,
}

struct HelloSession {
    events: EventSink,
    closed: Arc<Mutex<u32>>,
}

impl Translator for Hello {
    fn info(&self) -> AgentInfo {
        AgentInfo::new("hello", "Hello", "0")
    }

    fn takes(&self) -> Takes {
        Takes::new()
    }

    fn open(
        &self,
        _request: OpenRequest,
        events: EventSink,
    ) -> Result<Box<dyn Conversation>, HarnessError> {
        Ok(Box::new(HelloSession {
            events,
            closed: Arc::clone(&self.closed),
        }))
    }
}

impl Conversation for HelloSession {
    fn session_id(&self) -> String {
        "s1".to_owned()
    }

    fn options(&self) -> Vec<ConfigOption> {
        Vec::new()
    }

    fn prompt(&self, _content: Vec<coqui_acp_harness::event::Content>) -> Result<(), HarnessError> {
        self.events.send(Event::MessageChunk {
            message: "m1".to_owned(),
            text: "hello".to_owned(),
        });
        self.events
            .send(Event::TurnEnd(TurnEnd::stopped(Stop::EndTurn)));
        Ok(())
    }

    fn interrupt(&self) -> Result<(), HarnessError> {
        Ok(())
    }

    fn set_option(&self, _id: &str, _value: &str) -> Result<Vec<ConfigOption>, HarnessError> {
        Ok(Vec::new())
    }

    fn close(&self) {
        *self.closed.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }
}

fn cwd() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

fn open_session(client: &Client) {
    client.say(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}));
    let (initialized, _) = client.answer(1);
    assert_eq!(
        initialized["result"]["agentInfo"]["name"], "hello",
        "{initialized}"
    );
    client.say(&json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": cwd(), "mcpServers": []}}));
    let (opened, _) = client.answer(2);
    assert_eq!(opened["result"]["sessionId"], "s1", "{opened}");
}

fn served(channel: Memory, agent: Arc<Hello>, control: Arc<InProcess>) -> thread::JoinHandle<End> {
    thread::spawn(move || serve(&channel, agent, Settings::new(), 1024 * 1024, &control))
}

#[test]
fn a_turn_reaches_the_client_and_its_close_closes_the_agents_sessions() {
    let (channel, client) = connection();
    let agent = Arc::new(Hello::default());
    let closed = Arc::clone(&agent.closed);
    let running = served(channel, agent, InProcess::new());
    open_session(&client);
    client.say(
        &json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt",
        "params": {"sessionId": "s1", "prompt": [{"type": "text", "text": "hi"}]}}),
    );
    let (answered, updates) = client.answer(3);
    assert_eq!(answered["result"]["stopReason"], "end_turn", "{answered}");
    assert!(
        updates
            .iter()
            .any(|update| update["params"]["update"]["content"]["text"] == "hello"),
        "{updates:?}"
    );
    client.send.send(None).expect("closed");
    assert_eq!(running.join().expect("served"), End::PeerClosed);
    assert_eq!(*closed.lock().unwrap_or_else(PoisonError::into_inner), 1);
}

#[test]
fn stopping_it_ends_the_connection_and_returns_once_its_sessions_are_closed() {
    let (channel, client) = connection();
    let agent = Arc::new(Hello::default());
    let closed = Arc::clone(&agent.closed);
    let control = InProcess::new();
    let running = served(channel, agent, Arc::clone(&control));
    open_session(&client);
    control.stop();
    assert_eq!(*closed.lock().unwrap_or_else(PoisonError::into_inner), 1);
    assert_eq!(running.join().expect("served"), End::Stopped);
    drop(client);
}

#[test]
fn a_message_holding_a_line_break_ends_the_connection() {
    let (channel, client) = connection();
    let running = served(channel, Arc::new(Hello::default()), InProcess::new());
    client
        .send
        .send(Some(b"{\"jsonrpc\": \"2.0\",\n\"id\": 1}".to_vec()))
        .expect("sent");
    assert_eq!(running.join().expect("served"), End::Protocol);
}

#[test]
fn a_client_that_takes_nothing_more_ends_the_connection_and_the_agents_sessions() {
    let (channel, client) = connection();
    let agent = Arc::new(Hello::default());
    let closed = Arc::clone(&agent.closed);
    let channel = Arc::new(channel);
    let shared = Arc::clone(&channel);
    let control = InProcess::new();
    let running = {
        let control = Arc::clone(&control);
        thread::spawn(move || serve(&*shared, agent, Settings::new(), 1024 * 1024, &control))
    };
    open_session(&client);
    channel.refuse_sends.store(true, Ordering::SeqCst);
    client.say(
        &json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt",
        "params": {"sessionId": "s1", "prompt": [{"type": "text", "text": "hi"}]}}),
    );
    assert_eq!(running.join().expect("served"), End::Failed);
    assert_eq!(*closed.lock().unwrap_or_else(PoisonError::into_inner), 1);
}
