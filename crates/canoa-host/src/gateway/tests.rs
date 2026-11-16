//! The gateway over real sockets, against a model that answers what it is told to.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa_foundry::{Engine, EngineError, ModelInfo};
use serde_json::{Value, json};

use super::{Running, Ticket};

const CLAUDE: &str = include_str!("../../../canoa-foundry/tests/fixtures/messages_first.json");
const CODEX: &str = include_str!("../../../canoa-foundry/tests/fixtures/codex_first.json");

/// A model that gives the answer it was built with, after the delay it was built with, and remembers what it was asked.
struct Stub {
    model: ModelInfo,
    loaded: bool,
    answer: Result<Value, EngineError>,
    delay: Duration,
    seen: Mutex<Vec<Value>>,
}

impl Stub {
    fn new(answer: Value) -> Self {
        Self {
            model: ModelInfo::new("local-model").tools(true).context(8192),
            loaded: true,
            answer: Ok(answer),
            delay: Duration::ZERO,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Engine for Stub {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        if self.loaded {
            Ok(self.model.clone())
        } else {
            Err(EngineError::NotLoaded("local-model".to_owned()))
        }
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.model.clone()]
    }

    fn chat(&self, request: &Value) -> Result<Value, EngineError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        std::thread::sleep(self.delay);
        self.answer.clone()
    }
}

fn text_answer(text: &str) -> Value {
    json!({"id": "chatcmpl-9", "object": "chat.completion", "model": "local-model",
        "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}})
}

fn call_answer(name: &str, arguments: &str) -> Value {
    json!({"id": "chatcmpl-9", "object": "chat.completion", "model": "local-model",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {"role": "assistant", "content": null,
            "tool_calls": [{"id": "call_7", "type": "function", "function": {"name": name, "arguments": arguments}}]}}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 9, "total_tokens": 29}})
}

struct Up {
    stub: Arc<Stub>,
    running: Arc<Running>,
    ticket: Ticket,
}

fn up(stub: Stub, keep_alive: Duration) -> Up {
    let stub = Arc::new(stub);
    let running =
        Running::start_with(Arc::clone(&stub) as Arc<dyn Engine>, keep_alive).expect("starts");
    let ticket = running.mint().expect("mints");
    Up {
        stub,
        running,
        ticket,
    }
}

/// What one request got: the status, the head's lines, and the body.
struct Got {
    status: u16,
    head: String,
    body: String,
}

impl Got {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("JSON: {}", self.body))
    }

    fn events(&self) -> Vec<String> {
        self.body
            .lines()
            .filter_map(|line| line.strip_prefix("event: "))
            .map(str::to_owned)
            .collect()
    }
}

fn fetch(
    running: &Running,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<&Value>,
) -> Got {
    let port = running.listener.addr().port();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("sets");
    let payload = body.map(Value::to_string);
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    if let Some(bearer) = bearer {
        request.push_str(&format!("Authorization: Bearer {bearer}\r\n"));
    }
    if let Some(payload) = &payload {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        ));
    }
    request.push_str("\r\n");
    request.push_str(payload.as_deref().unwrap_or(""));
    stream.write_all(request.as_bytes()).expect("writes");
    let mut all = String::new();
    let _ = stream.read_to_string(&mut all);
    let (head, body) = all.split_once("\r\n\r\n").unwrap_or((&all, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Got {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

fn post(up: &Up, path: &str, body: &Value) -> Got {
    fetch(
        &up.running,
        "POST",
        path,
        Some(up.ticket.bearer()),
        Some(body),
    )
}

fn fixture(text: &str, stream: bool) -> Value {
    let mut body = serde_json::from_str::<Value>(text).expect("a fixture")["body"].clone();
    body["stream"] = stream.into();
    body
}

#[test]
fn only_the_session_bearer_gets_in_and_it_stops_working_when_the_session_ends() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let ask = json!({"messages": [{"role": "user", "content": "hi"}]});
    let none = fetch(
        &up.running,
        "POST",
        "/v1/chat/completions",
        None,
        Some(&ask),
    );
    assert_eq!(none.status, 401);
    let wrong = fetch(
        &up.running,
        "POST",
        "/v1/chat/completions",
        Some("not-it"),
        Some(&ask),
    );
    assert_eq!(wrong.status, 401);
    assert_eq!(post(&up, "/v1/chat/completions", &ask).status, 200);
    let bearer = up.ticket.bearer().to_owned();
    let running = Arc::clone(&up.running);
    drop(up.ticket);
    let after = fetch(
        &running,
        "POST",
        "/v1/chat/completions",
        Some(&bearer),
        Some(&ask),
    );
    assert_eq!(after.status, 401, "{}", after.head);
}

#[test]
fn a_bearer_is_long_and_each_session_has_its_own() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let other = up.running.mint().expect("mints");
    assert_eq!(up.ticket.bearer().len(), 64);
    assert_ne!(up.ticket.bearer(), other.bearer());
    assert_eq!(up.ticket.address(), other.address());
    assert!(up.ticket.address().starts_with("http://127.0.0.1:"));
}

#[test]
fn what_is_not_served_is_not_found_and_the_wrong_method_says_which_is_right() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let bearer = Some(up.ticket.bearer());
    assert_eq!(
        fetch(&up.running, "GET", "/v1/nothing", bearer, None).status,
        404
    );
    let wrong = fetch(&up.running, "GET", "/v1/messages", bearer, None);
    assert_eq!(wrong.status, 405);
    assert!(wrong.head.contains("Allow: POST"), "{}", wrong.head);
    assert_eq!(
        fetch(&up.running, "POST", "/v1/models", bearer, Some(&json!({}))).status,
        405
    );
}

#[test]
fn a_chat_request_is_answered_and_the_model_is_asked_what_it_takes() {
    let up = up(Stub::new(text_answer("forty-two")), Duration::from_secs(10));
    let ask = json!({"model": "gpt-anything", "stream": false, "store": false,
        "messages": [{"role": "developer", "content": "be brief"}, {"role": "user", "content": "6 x 7?"}]});
    let got = post(&up, "/v1/chat/completions", &ask);
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.json()["choices"][0]["message"]["content"], "forty-two");
    let seen = up.stub.seen.lock().expect("lock");
    assert_eq!(seen[0]["model"], "local-model");
    assert_eq!(
        seen[0]["messages"][0],
        json!({"role": "system", "content": "be brief"})
    );
    assert!(seen[0].get("store").is_none());
}

#[test]
fn a_chat_stream_is_chunks_then_done_and_says_it_is_an_event_stream() {
    let up = up(
        Stub::new(call_answer("read", "{\"path\":\"a.txt\"}")),
        Duration::from_secs(10),
    );
    let ask = json!({"stream": true, "stream_options": {"include_usage": true},
        "messages": [{"role": "user", "content": "read a.txt"}],
        "tools": [{"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}}]});
    let got = post(&up, "/v1/chat/completions", &ask);
    assert_eq!(got.status, 200, "{}", got.body);
    assert!(got.head.contains("text/event-stream"), "{}", got.head);
    assert!(got.body.contains("\"role\":\"assistant\""));
    assert!(got.body.contains("\"name\":\"read\""));
    assert!(got.body.contains("\"finish_reason\":\"tool_calls\""));
    assert!(got.body.contains("\"usage\""));
    assert!(
        got.body.trim_end().ends_with("data: [DONE]"),
        "{}",
        got.body
    );
}

#[test]
fn what_claude_code_sends_runs_the_model_and_comes_back_as_a_tool_use_message() {
    let up = up(
        Stub::new(call_answer("Read", "{\"file_path\":\"secret.txt\"}")),
        Duration::from_secs(10),
    );
    let got = post(&up, "/v1/messages?beta=true", &fixture(CLAUDE, false));
    assert_eq!(got.status, 200, "{}", got.body);
    let message = got.json();
    assert_eq!(message["type"], "message");
    assert_eq!(message["stop_reason"], "tool_use");
    assert_eq!(message["content"][0]["type"], "tool_use");
    assert_eq!(message["content"][0]["name"], "Read");
    assert_eq!(
        message["content"][0]["input"],
        json!({"file_path": "secret.txt"})
    );
    assert!(
        got.head
            .to_ascii_lowercase()
            .contains("x-canoa-dropped: the tool deferredtoolplaceholder"),
        "{}",
        got.head
    );
    let seen = up.stub.seen.lock().expect("lock");
    assert_eq!(seen[0]["tools"][0]["function"]["name"], "Read");
}

#[test]
fn what_claude_code_asks_to_stream_is_replayed_as_a_messages_stream() {
    let up = up(
        Stub::new(call_answer("Read", "{\"file_path\":\"secret.txt\"}")),
        Duration::from_secs(10),
    );
    let got = post(&up, "/v1/messages?beta=true", &fixture(CLAUDE, true));
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        got.events(),
        [
            "message_start",
            "ping",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ],
        "{}",
        got.body
    );
    assert!(got.body.contains("\"stop_reason\":\"tool_use\""));
}

#[test]
fn what_codex_sends_comes_back_as_a_function_call_response_stream() {
    let up = up(
        Stub::new(call_answer("exec_command", "{\"cmd\":\"type secret.txt\"}")),
        Duration::from_secs(10),
    );
    let got = post(&up, "/v1/responses", &fixture(CODEX, true));
    assert_eq!(got.status, 200, "{}", got.body);
    let events = got.events();
    assert_eq!(events.first().map(String::as_str), Some("response.created"));
    assert_eq!(
        events.last().map(String::as_str),
        Some("response.completed")
    );
    assert!(
        events
            .iter()
            .any(|e| e == "response.function_call_arguments.done")
    );
    assert!(got.body.contains("\"name\":\"exec_command\""));
    let plain = post(&up, "/v1/responses", &fixture(CODEX, false));
    assert_eq!(plain.json()["output"][0]["type"], "function_call");
}

#[test]
fn a_model_that_is_not_loaded_is_a_refusal_in_the_apis_own_shape_not_a_failure_to_retry() {
    let mut stub = Stub::new(text_answer("hi"));
    stub.loaded = false;
    let up = up(stub, Duration::from_secs(10));
    let got = post(&up, "/v1/messages", &fixture(CLAUDE, false));
    assert_eq!(got.status, 400);
    assert_eq!(got.json()["type"], "error");
    assert!(
        got.json()["error"]["message"]
            .as_str()
            .expect("text")
            .contains("not loaded")
    );
    let chat = post(
        &up,
        "/v1/chat/completions",
        &json!({"messages": [{"role": "user", "content": "x"}]}),
    );
    assert_eq!(chat.status, 400);
    assert!(
        chat.json()["error"]["message"]
            .as_str()
            .expect("text")
            .contains("not loaded")
    );
    assert!(
        up.stub.seen.lock().expect("lock").is_empty(),
        "the model was never asked"
    );
}

#[test]
fn a_model_that_cannot_call_tools_is_told_what_it_cannot_do() {
    let mut stub = Stub::new(text_answer("hi"));
    stub.model = ModelInfo::new("local-model");
    let up = up(stub, Duration::from_secs(10));
    let got = post(&up, "/v1/messages", &fixture(CLAUDE, false));
    assert_eq!(got.status, 400);
    assert!(
        got.json()["error"]["message"]
            .as_str()
            .expect("text")
            .contains("cannot call tools")
    );
}

#[test]
fn an_engine_that_fails_is_a_bad_gateway_whole_and_an_error_event_in_a_stream() {
    let mut stub = Stub::new(Value::Null);
    stub.answer = Err(EngineError::Failed("the model crashed".to_owned()));
    let up = up(stub, Duration::from_secs(10));
    let ask = json!({"messages": [{"role": "user", "content": "hi"}]});
    let whole = post(&up, "/v1/messages", &ask);
    assert_eq!(whole.status, 502);
    assert!(whole.body.contains("the model crashed"));
    let streamed = post(
        &up,
        "/v1/messages",
        &json!({"messages": [{"role": "user", "content": "hi"}], "stream": true}),
    );
    assert_eq!(streamed.status, 200);
    assert_eq!(
        streamed.events(),
        ["message_start", "ping", "error"],
        "{}",
        streamed.body
    );
    let chat = post(
        &up,
        "/v1/chat/completions",
        &json!({"stream": true, "messages": [{"role": "user", "content": "hi"}]}),
    );
    assert!(chat.body.contains("the model crashed") && chat.body.contains("[DONE]"));
}

#[test]
fn what_the_service_refuses_about_a_request_is_told_as_a_refusal_and_a_model_unloaded_midway_as_not_loaded()
 {
    let mut stub = Stub::new(Value::Null);
    stub.answer = Err(EngineError::Refused(
        "the Foundry Local service refused it (500): structured tool calling requires markers"
            .to_owned(),
    ));
    let first = up(stub, Duration::from_secs(10));
    let ask = json!({"messages": [{"role": "user", "content": "hi"}]});
    let refused = post(&first, "/v1/messages", &ask);
    assert_eq!(refused.status, 400, "not a failure a client retries");
    assert!(refused.body.contains("structured tool calling requires"));
    let mut stub = Stub::new(Value::Null);
    stub.answer = Err(EngineError::NotLoaded("local-model".to_owned()));
    let second = up(stub, Duration::from_secs(10));
    let gone = post(&second, "/v1/chat/completions", &ask);
    assert_eq!(gone.status, 400);
    assert!(
        gone.body.contains("not loaded") && gone.body.contains("load it in Foundry Local"),
        "{}",
        gone.body
    );
}

#[test]
fn a_stream_says_it_is_still_there_while_a_slow_model_works() {
    let mut stub = Stub::new(text_answer("late"));
    stub.delay = Duration::from_millis(400);
    let up = up(stub, Duration::from_millis(40));
    let ask = json!({"stream": true, "messages": [{"role": "user", "content": "hi"}]});
    let got = post(&up, "/v1/messages", &ask);
    let events = got.events();
    assert_eq!(events[0], "message_start");
    assert!(
        events.iter().filter(|e| *e == "ping").count() >= 3,
        "{events:?}"
    );
    assert_eq!(events.last().map(String::as_str), Some("message_stop"));
    let responses = post(
        &up,
        "/v1/responses",
        &json!({"stream": true, "input": "hi"}),
    );
    assert!(
        responses.body.contains(": keep-alive"),
        "{}",
        responses.body
    );
    assert!(responses.body.contains("response.completed"));
}

#[test]
fn a_continued_response_and_a_body_that_is_not_json_are_refused_plainly() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let continued = post(
        &up,
        "/v1/responses",
        &json!({"input": "x", "previous_response_id": "resp_1"}),
    );
    assert_eq!(continued.status, 400);
    assert!(continued.body.contains("previous_response_id"));
    let port = up.running.listener.addr().port();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    let request = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\nContent-Length: 3\r\n\r\nnot",
        up.ticket.bearer()
    );
    stream.write_all(request.as_bytes()).expect("writes");
    let mut said = String::new();
    let _ = stream.read_to_string(&mut said);
    assert!(said.starts_with("HTTP/1.1 400"), "{said}");
}

#[test]
fn the_models_are_listed_with_what_each_can_do_and_counted_tokens_are_an_estimate() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let bearer = Some(up.ticket.bearer());
    let list = fetch(&up.running, "GET", "/v1/models", bearer, None);
    assert_eq!(list.status, 200);
    assert_eq!(list.json()["data"][0]["id"], "local-model");
    assert_eq!(list.json()["data"][0]["capabilities"]["tools"], true);
    let one = fetch(&up.running, "GET", "/v1/models/local-model", bearer, None);
    assert_eq!(one.json()["context_length"], 8192);
    assert_eq!(
        fetch(&up.running, "GET", "/v1/models/other", bearer, None).status,
        404
    );
    let counted = post(
        &up,
        "/v1/messages/count_tokens",
        &json!({"messages": [{"role": "user", "content": "hello there, local model"}]}),
    );
    assert!(counted.json()["input_tokens"].as_u64().expect("a number") > 0);
}

#[test]
fn a_page_cannot_reach_the_gateway() {
    let up = up(Stub::new(text_answer("hi")), Duration::from_secs(10));
    let port = up.running.listener.addr().port();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    let request = format!(
        "GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://example.org\r\nAuthorization: Bearer {}\r\n\r\n",
        up.ticket.bearer()
    );
    stream.write_all(request.as_bytes()).expect("writes");
    let mut said = String::new();
    let _ = stream.read_to_string(&mut said);
    assert!(said.starts_with("HTTP/1.1 403"), "{said}");
}
