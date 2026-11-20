//! The engine over the Foundry Local service the person runs: chat completions sent to it over loopback HTTP, one at a time and
//! answered whole.
//!
//! The service, not this process, holds the model, so loading one is the person's choice (`foundry model load`) and nothing
//! here ever does it. Its streaming does not deliver tokens in the versions measured, so a request is never streamed to it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, locks and the clock are std's"
)]

use std::io::Read as _;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use canoa::network::Network;
use serde_json::Value;

use crate::{Cancel, Engine, EngineError, ModelInfo, is_cut_tool_call, running_endpoint};

/// What the engine serves and where.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DaemonConfig {
    /// The id of the model variant to serve (`qwen3-14b-generic-gpu:2`), as Foundry Local lists it.
    pub model: String,
    /// The service, as `http://127.0.0.1:PORT`; the running service's own when `None`.
    pub endpoint: Option<String>,
    /// What the model is offered to do. The service does not say what a model can (its listing says every Qwen calls tools, and
    /// only one of those did), so both are `true` unless a caller knows better, and a model that cannot is refused by the
    /// service in its own words.
    pub tools: bool,
    /// See [`DaemonConfig::tools`].
    pub images: bool,
    /// How many tokens the model holds at once, when the person knows.
    pub context: Option<u64>,
    /// How long one answer may take: a local model on a large prompt is slow, and the person stops one by ending the session.
    pub answer: Duration,
}

impl DaemonConfig {
    /// A model of `id` on the running service, offered tools and images.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            endpoint: None,
            tools: true,
            images: true,
            context: None,
            answer: DEFAULT_ANSWER,
        }
    }
}

/// How long one answer may take unless [`DaemonConfig::answer`] says otherwise.
pub const DEFAULT_ANSWER: Duration = Duration::from_secs(30 * 60);

/// An agent for one conversation with the service, and whether it opens its own sockets.
///
/// A request has an agent of its own so that cancelling it shuts its connection down and no one else's:
/// [`ureq::Agent::shutdown`] ends every connection of the agent it is called on, and it reaches only the sockets ureq opened. The
/// service is on this computer, so the connection goes straight to it unless the host's network says otherwise; one that goes
/// through a proxy is the network's agent, whose connections only it can close.
fn new_agent(base: &str, answer: Duration) -> Result<(ureq::Agent, bool), EngineError> {
    let settings = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(answer))
        .build();
    let network = Network::current();
    if network.goes_direct(base) {
        return Ok((ureq::Agent::new_with_config(settings), true));
    }
    network
        .agent(settings)
        .map(|agent| (agent, false))
        .map_err(|error| EngineError::Failed(error.to_string()))
}

/// A model run by the Foundry Local service.
#[derive(Debug)]
pub struct DaemonEngine {
    base: String,
    info: ModelInfo,
    /// How long one answer may take.
    answer: Duration,
    /// One request at a time: the service answers one, and a second on a model is not what it was measured with.
    one_at_a_time: Mutex<()>,
}

impl DaemonEngine {
    /// Finds the service and checks that it lists the model. It does not load the model.
    ///
    /// # Errors
    ///
    /// No service is running, it does not answer, or it does not know the model.
    pub fn open(config: &DaemonConfig, home: &Path) -> Result<Self, EngineError> {
        let base = config
            .endpoint
            .clone()
            .or_else(|| running_endpoint(home))
            .ok_or_else(|| {
                EngineError::Failed(
                    "no Foundry Local service is running: start it (`foundry server start`) and load a model".to_owned(),
                )
            })?
            .trim_end_matches('/')
            .to_owned();
        let (agent, _) = new_agent(&base, config.answer)?;
        let listed = agent
            .get(format!("{base}/v1/models"))
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call()
            .map_err(|error| {
                EngineError::Failed(format!(
                    "the Foundry Local service at {base} does not answer: {error}"
                ))
            })?;
        let status = listed.status().as_u16();
        let text = listed
            .into_body()
            .read_to_string()
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        if status != 200 {
            return Err(EngineError::Failed(format!(
                "the Foundry Local service at {base} answered {status} to its model list"
            )));
        }
        let known = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|list| list.get("data").and_then(Value::as_array).cloned())
            .is_some_and(|models| {
                models
                    .iter()
                    .any(|model| model.get("id").and_then(Value::as_str) == Some(&config.model))
            });
        if !known {
            return Err(EngineError::Failed(format!(
                "the Foundry Local service at {base} does not list a model called {}: use an id from `foundry model list`",
                config.model
            )));
        }
        let mut info = ModelInfo::new(config.model.clone())
            .tools(config.tools)
            .images(config.images);
        if let Some(context) = config.context {
            info = info.context(context);
        }
        Ok(Self {
            base,
            info,
            answer: config.answer,
            one_at_a_time: Mutex::new(()),
        })
    }
}

/// What the service said was wrong, from the error body it sent.
fn reason(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.trim().chars().take(300).collect())
}

impl DaemonEngine {
    /// One chat request, ended when `cancel` is cancelled. An agent of ours has its connection shut down by the cancel, which ends
    /// the call; through a proxy the call goes on in a thread of its own that nobody waits for, since only the network's agent can
    /// close it.
    fn post_until_cancelled(
        &self,
        agent: &ureq::Agent,
        own: bool,
        request: &Value,
        cancel: &Cancel,
    ) -> Result<(u16, String), EngineError> {
        if own {
            return Self::post(&self.base, agent, request);
        }
        let (sent, answer) = std::sync::mpsc::channel();
        let (base, agent, request) = (self.base.clone(), agent.clone(), request.clone());
        std::thread::spawn(move || {
            let _ = sent.send(Self::post(&base, &agent, &request));
        });
        loop {
            match answer.recv_timeout(Duration::from_millis(100)) {
                Ok(done) => return done,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if cancel.is_cancelled() => {
                    return Err(EngineError::Cancelled);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(EngineError::Failed(
                        "the request to the service ended without an answer".to_owned(),
                    ));
                }
            }
        }
    }

    /// One chat request: the status and the body of what the service answered.
    fn post(
        base: &str,
        agent: &ureq::Agent,
        request: &Value,
    ) -> Result<(u16, String), EngineError> {
        let response = agent
            .post(format!("{base}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .send(request.to_string())
            .map_err(|error| {
                EngineError::Failed(format!("the Foundry Local service did not answer: {error}"))
            })?;
        let status = response.status().as_u16();
        let mut body = String::new();
        response
            .into_body()
            .into_reader()
            .take(256 * 1024 * 1024)
            .read_to_string(&mut body)
            .map_err(|error| EngineError::Failed(format!("the answer was cut off: {error}")))?;
        Ok((status, body))
    }
}

/// What a window leaves for the answer, from the service's refusal: "requires 67001 total tokens (3001 input + 64000 output),
/// which exceeds the model's maximum context length of 40960 tokens".
fn room_left(reason: &str) -> Option<u64> {
    let number_before = |text: &str, word: &str| -> Option<u64> {
        let end = text.find(word)?;
        text[..end]
            .split(|c: char| !c.is_ascii_digit())
            .rfind(|s| !s.is_empty())?
            .parse()
            .ok()
    };
    let input = number_before(reason, " input")?;
    let window = reason
        .split("context length of ")
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse::<u64>()
        .ok()?;
    // A few tokens of margin, since the service counts the template's own.
    let left = window.checked_sub(input)?.checked_sub(64)?;
    (left > 0).then_some(left)
}

impl Engine for DaemonEngine {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        Ok(self.info.clone())
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.info.clone()]
    }

    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError> {
        let mut request = request.clone();
        if let Some(object) = request.as_object_mut() {
            object.insert("model".into(), self.info.id.clone().into());
            object.insert("stream".into(), false.into());
        }
        let _one = crate::engine::lock_unless_cancelled(&self.one_at_a_time, cancel)?;
        // Cancelling closes the connection to the service, which is what tells it the answer is not wanted: its model is then free
        // for the next request. The agent is this request's own, so no other request is touched.
        let (agent, own) = new_agent(&self.base, self.answer)?;
        if own {
            let victim = agent.clone();
            cancel.on_cancel(move || victim.shutdown());
        }
        let gone = |error: EngineError| {
            if cancel.is_cancelled() {
                EngineError::Cancelled
            } else {
                error
            }
        };
        let (mut status, mut body) = self
            .post_until_cancelled(&agent, own, &request, cancel)
            .map_err(gone)?;
        // The service refuses a request whose prompt and allowed answer together exceed the model's window, and agents ask for
        // far more than that (Claude Code asks for 64000): the answer is allowed what the window leaves, and the request is
        // made again once.
        if status == 400
            && let Some(left) = room_left(&reason(&body))
            && let Some(object) = request.as_object_mut()
        {
            object.insert("max_tokens".into(), left.into());
            (status, body) = self
                .post_until_cancelled(&agent, own, &request, cancel)
                .map_err(gone)?;
        }
        if (200..300).contains(&status) {
            return serde_json::from_str(&body)
                .map_err(|_| EngineError::Failed("the service's answer is not JSON".to_owned()));
        }
        let why = reason(&body);
        if why.to_ascii_lowercase().contains("not loaded") {
            return Err(EngineError::NotLoaded(self.info.id.clone()));
        }
        // A tool call cut by the answer's token allowance is an answer that stopped for length, not a refusal of the request.
        if is_cut_tool_call(&why) {
            return Err(EngineError::CutToolCall(why));
        }
        // What the service refuses about this request is said back as the service said it: retrying it changes nothing.
        Err(EngineError::Refused(format!(
            "the Foundry Local service refused it ({status}): {why}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// A service that answers each request with the next of `replies` (a status and a body), and keeps what it was sent.
    fn serve(replies: Vec<(u16, String)>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("binds");
        let address = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let (sent, seen) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let text = read_request(&mut stream);
                let _ = sent.send(text);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (address, seen)
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut all = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).unwrap_or(0);
            all.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&all).into_owned();
            if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if rest.len() >= length {
                    return text;
                }
            }
            if read == 0 {
                return text;
            }
        }
    }

    fn open(address: &str) -> Result<DaemonEngine, EngineError> {
        let mut config = DaemonConfig::new("local-model");
        config.endpoint = Some(address.to_owned());
        DaemonEngine::open(&config, Path::new("."))
    }

    #[test]
    fn a_service_that_lists_the_model_is_opened_and_one_that_does_not_is_refused_by_name() {
        let listing = r#"{"data":[{"id":"local-model","object":"model"}]}"#.to_owned();
        let (address, _) = serve(vec![(200, listing)]);
        let engine = open(&address).expect("opens");
        assert!(engine.model().expect("a model").tools);
        let (address, _) = serve(vec![(200, r#"{"data":[{"id":"other"}]}"#.to_owned())]);
        let refused = open(&address).expect_err("not listed");
        assert!(
            refused
                .to_string()
                .contains("does not list a model called local-model"),
            "{refused}"
        );
        let (address, _) = serve(vec![(500, "boom".to_owned())]);
        assert!(open(&address).is_err());
        assert!(open("http://127.0.0.1:9").is_err(), "nothing listens there");
    }

    #[test]
    fn a_chat_goes_to_the_service_as_a_whole_request_for_its_model_and_comes_back_as_it_answered() {
        let answer =
            r#"{"id":"c1","choices":[{"message":{"role":"assistant","content":"hi"}}]}"#.to_owned();
        let (address, seen) = serve(vec![
            (200, r#"{"data":[{"id":"local-model"}]}"#.to_owned()),
            (200, answer),
        ]);
        let engine = open(&address).expect("opens");
        let _ = seen.recv().expect("the listing");
        let request = serde_json::json!({"model": "gpt-x", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
        let said = engine.chat(&request, &Cancel::new()).expect("answers");
        assert_eq!(said["choices"][0]["message"]["content"], "hi");
        let sent = seen.recv().expect("the chat");
        assert!(sent.starts_with("POST /v1/chat/completions"), "{sent}");
        let body: Value =
            serde_json::from_str(sent.split_once("\r\n\r\n").expect("body").1).expect("JSON");
        assert_eq!(body["model"], "local-model");
        assert_eq!(body["stream"], false, "never streamed to the service");
    }

    #[test]
    fn a_model_that_is_not_loaded_and_a_refused_request_are_told_apart_from_a_service_that_is_gone()
    {
        let (address, _) = serve(vec![
            (200, r#"{"data":[{"id":"local-model"}]}"#.to_owned()),
            (400, r#"{"error":{"message":"Model not loaded: local-model"}}"#.to_owned()),
            (500, r#"{"error":{"message":"structured tool calling requires model-published markers"}}"#.to_owned()),
        ]);
        let engine = open(&address).expect("opens");
        let ask = serde_json::json!({"messages": []});
        assert_eq!(
            engine.chat(&ask, &Cancel::new()),
            Err(EngineError::NotLoaded("local-model".to_owned()))
        );
        let refused = engine.chat(&ask, &Cancel::new()).expect_err("refused");
        assert!(matches!(refused, EngineError::Refused(_)), "{refused:?}");
        assert!(
            refused
                .to_string()
                .contains("structured tool calling requires")
        );
        // The service has no more answers: its listener is gone.
        let gone = engine.chat(&ask, &Cancel::new()).expect_err("gone");
        assert!(matches!(gone, EngineError::Failed(_)), "{gone:?}");
    }

    #[test]
    fn a_window_that_the_prompt_and_the_answer_overflow_is_asked_again_with_what_it_leaves() {
        let refusal = r#"{"error":{"message":"Invalid request: x.cc:112 requires 67001 total tokens (3001 input + 64000 output), which exceeds the model's maximum context length of 40960 tokens"}}"#;
        assert_eq!(room_left(&reason(refusal)), Some(40960 - 3001 - 64));
        assert_eq!(room_left("some other refusal"), None);
        let (address, seen) = serve(vec![
            (200, r#"{"data":[{"id":"local-model"}]}"#.to_owned()),
            (400, refusal.to_owned()),
            (
                200,
                r#"{"choices":[{"message":{"content":"ok"}}]}"#.to_owned(),
            ),
        ]);
        let engine = open(&address).expect("opens");
        let _ = seen.recv().expect("the listing");
        let said = engine
            .chat(
                &serde_json::json!({"max_tokens": 64000, "messages": []}),
                &Cancel::new(),
            )
            .expect("answers on the second try");
        assert_eq!(said["choices"][0]["message"]["content"], "ok");
        let _first = seen.recv().expect("the first");
        let second = seen.recv().expect("the second");
        let (_, sent) = second.split_once("\r\n\r\n").expect("body");
        let body: Value = serde_json::from_str(sent).expect("JSON");
        assert_eq!(body["max_tokens"], 40960 - 3001 - 64);
    }

    #[test]
    fn a_tool_call_the_services_answer_ran_out_of_tokens_on_is_told_apart_from_a_refusal() {
        let cut = r#"{"error":{"message":"Inference failed: chat_session.cc:1005 ThrowInvalidToolCallWithoutRecovery Model emitted an invalid tool call: Tool-call output was interrupted before the batch could be committed"}}"#;
        let (address, _) = serve(vec![
            (200, r#"{"data":[{"id":"local-model"}]}"#.to_owned()),
            (500, cut.to_owned()),
        ]);
        let engine = open(&address).expect("opens");
        let cut = engine
            .chat(&serde_json::json!({"messages": []}), &Cancel::new())
            .expect_err("cut");
        assert!(matches!(cut, EngineError::CutToolCall(_)), "{cut:?}");
    }

    /// A service that takes a request and then says nothing, and tells when the connection it holds is closed.
    fn hang() -> (String, std::sync::mpsc::Receiver<&'static str>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("binds");
        let address = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let (said, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let listing = r#"{"data":[{"id":"local-model"}]}"#;
            let (mut stream, _) = listener.accept().expect("the listing");
            let _ = read_request(&mut stream);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{listing}",
                    listing.len()
                )
                .as_bytes(),
            );
            drop(stream);
            let (mut stream, _) = listener.accept().expect("the chat");
            let _ = read_request(&mut stream);
            let _ = said.send("asked");
            // Nothing is ever answered: a read that ends is the client closing its side.
            let mut byte = [0_u8; 1];
            let _ = stream.read(&mut byte);
            let _ = said.send("closed");
        });
        (address, heard)
    }

    #[test]
    fn a_request_that_is_cancelled_closes_its_connection_to_the_service_and_ends() {
        let (address, heard) = hang();
        let engine = std::sync::Arc::new(open(&address).expect("opens"));
        let cancel = Cancel::new();
        let asking = {
            let (engine, cancel) = (std::sync::Arc::clone(&engine), cancel.clone());
            std::thread::spawn(move || engine.chat(&serde_json::json!({"messages": []}), &cancel))
        };
        assert_eq!(heard.recv_timeout(Duration::from_secs(10)), Ok("asked"));
        cancel.cancel();
        let ended = asking.join().expect("the call ends");
        assert_eq!(ended, Err(EngineError::Cancelled));
        assert_eq!(
            heard.recv_timeout(Duration::from_secs(10)),
            Ok("closed"),
            "the service saw the connection close"
        );
    }

    #[test]
    fn a_request_cancelled_while_it_waits_for_the_model_never_reaches_the_service() {
        let (address, _) = serve(vec![(200, r#"{"data":[{"id":"local-model"}]}"#.to_owned())]);
        let engine = open(&address).expect("opens");
        let _busy = engine.one_at_a_time.lock().expect("the model is taken");
        let cancel = Cancel::new();
        cancel.cancel();
        assert_eq!(
            engine.chat(&serde_json::json!({"messages": []}), &cancel),
            Err(EngineError::Cancelled)
        );
    }

    #[test]
    fn a_reason_is_read_from_the_services_error_shapes_or_from_its_text() {
        assert_eq!(reason(r#"{"error":{"message":"a"}}"#), "a");
        assert_eq!(reason(r#"{"message":"b"}"#), "b");
        assert_eq!(reason(r#"{"error":"c"}"#), "c");
        assert_eq!(reason("plain words"), "plain words");
    }
}
