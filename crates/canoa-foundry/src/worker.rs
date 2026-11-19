//! An engine in a process of its own: the lines the two ends speak, and the loop that serves them.
//!
//! A model run in the host's process takes the host with it when the native engine crashes (a prompt of a couple of thousand
//! tokens segfaulted one model measured). So the host starts a process that opens the engine, and talks to it over its standard
//! input and output, one JSON object a line:
//!
//! - the worker's first line is `{"ready": {"id", "tools", "images", "context"}}` once the engine is open (and, for an engine
//!   that loads a model, loaded), or `{"failed": "reason"}` and it ends;
//! - the host sends `{"id": N, "chat": REQUEST}` and the worker answers `{"id": N, "ok": RESPONSE}` or
//!   `{"id": N, "error": {"kind", "message"}}`; requests run side by side, each with its own cancel;
//! - the host sends `{"cancel": N}` to stop request N, and closes the worker's input to end it.
//!
//! What a native library prints to standard output lands among the lines, so a reader takes a line only if it is one of these
//! objects ([`hello`], [`answer`]) and skips the rest.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "pipes, locks and threads are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use std::io::{BufRead, Write};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use crate::{Cancel, Engine, EngineError, ModelInfo};

/// A model's description as the worker's first line carries it.
#[must_use]
pub fn model_json(model: &ModelInfo) -> Value {
    json!({"id": model.id, "tools": model.tools, "images": model.images, "context": model.context})
}

/// Reads what [`model_json`] wrote.
#[must_use]
pub fn model_from(value: &Value) -> Option<ModelInfo> {
    let mut model = ModelInfo::new(value.get("id")?.as_str()?)
        .tools(value.get("tools").and_then(Value::as_bool) == Some(true))
        .images(value.get("images").and_then(Value::as_bool) == Some(true));
    if let Some(context) = value.get("context").and_then(Value::as_u64) {
        model = model.context(context);
    }
    Some(model)
}

/// An error as a line carries it.
#[must_use]
pub fn error_json(error: &EngineError) -> Value {
    let (kind, message) = match error {
        EngineError::Cancelled => ("cancelled", error.to_string()),
        EngineError::CutToolCall(why) => ("cut_tool_call", why.clone()),
        EngineError::NotLoaded(model) => ("not_loaded", model.clone()),
        EngineError::Refused(why) => ("refused", why.clone()),
        EngineError::Failed(why) => ("failed", why.clone()),
    };
    json!({"kind": kind, "message": message})
}

/// Reads what [`error_json`] wrote; a kind this does not know is a failure with its words.
#[must_use]
pub fn error_from(value: &Value) -> EngineError {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the engine failed without saying why")
        .to_string();
    match value.get("kind").and_then(Value::as_str) {
        Some("cancelled") => EngineError::Cancelled,
        Some("cut_tool_call") => EngineError::CutToolCall(message),
        Some("not_loaded") => EngineError::NotLoaded(message),
        Some("refused") => EngineError::Refused(message),
        _ => EngineError::Failed(message),
    }
}

/// Opens an engine with `open` and serves `input`'s lines with it, writing the answers to `output`, until the input ends. It
/// returns when every request under way has ended; the engine is dropped then, which is what unloads its model.
///
/// # Errors
///
/// Writing to `output` failed, so the host is gone.
pub fn serve<E: Engine>(
    open: impl FnOnce() -> Result<E, EngineError>,
    input: impl BufRead,
    output: impl Write + Send,
) -> std::io::Result<()> {
    let output = Mutex::new(output);
    let say = |line: &Value| -> std::io::Result<()> {
        let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
        writeln!(out, "{line}")?;
        out.flush()
    };
    let engine = match open() {
        Ok(engine) => engine,
        Err(error) => return say(&json!({"failed": error.to_string()})),
    };
    let model = engine.model().unwrap_or_else(|_| ModelInfo::new("unknown"));
    say(&json!({"ready": model_json(&model)}))?;
    let running: Mutex<BTreeMap<u64, Cancel>> = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for line in input.lines() {
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(id) = message.get("cancel").and_then(Value::as_u64) {
                let cancel = running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&id)
                    .cloned();
                if let Some(cancel) = cancel {
                    cancel.cancel();
                }
                continue;
            }
            let (Some(id), Some(request)) = (
                message.get("id").and_then(Value::as_u64),
                message.get("chat"),
            ) else {
                continue;
            };
            let cancel = Cancel::new();
            running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id, cancel.clone());
            let (engine, running, say) = (&engine, &running, &say);
            let request = request.clone();
            scope.spawn(move || {
                let answer = match engine.chat(&request, &cancel) {
                    Ok(done) => json!({"id": id, "ok": done}),
                    Err(error) => json!({"id": id, "error": error_json(&error)}),
                };
                running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                // The host is gone when this cannot be written; its input ends next.
                let _ = say(&answer);
            });
        }
        // The host closed its end: what is still running is for no one.
        for cancel in running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            cancel.cancel();
        }
    });
    drop(engine);
    Ok(())
}

/// What a worker said that is not an answer: its first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hello {
    /// The engine is open, and this is the model it runs.
    Ready(ModelInfo),
    /// The engine could not be opened, for this reason.
    Failed(String),
}

/// Reads the worker's first line.
#[must_use]
pub fn hello(line: &str) -> Option<Hello> {
    let value: Value = serde_json::from_str(line).ok()?;
    if let Some(why) = value.get("failed").and_then(Value::as_str) {
        return Some(Hello::Failed(why.to_string()));
    }
    model_from(value.get("ready")?).map(Hello::Ready)
}

/// What a worker's line answers: the request's number and its outcome.
#[must_use]
pub fn answer(line: &str) -> Option<(u64, Result<Value, EngineError>)> {
    let value: Value = serde_json::from_str(line).ok()?;
    let id = value.get("id")?.as_u64()?;
    if let Some(error) = value.get("error") {
        return Some((id, Err(error_from(error))));
    }
    Some((id, Ok(value.get("ok")?.clone())))
}

/// The line that asks the worker for an answer to `request`, numbered `id`.
#[must_use]
pub fn chat_line(id: u64, request: &Value) -> String {
    json!({"id": id, "chat": request}).to_string()
}

/// The line that stops request `id`.
#[must_use]
pub fn cancel_line(id: u64) -> String {
    json!({"cancel": id}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use std::io::Cursor;

    /// A model that answers with what it was asked, and a request that says `wait` until it is cancelled.
    struct Echo;

    impl Engine for Echo {
        fn model(&self) -> Result<ModelInfo, EngineError> {
            Ok(ModelInfo::new("m").tools(true).context(4096))
        }

        fn models(&self) -> alloc::vec::Vec<ModelInfo> {
            alloc::vec![ModelInfo::new("m")]
        }

        fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError> {
            if request.get("wait").is_some() {
                let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
                while !cancel.is_cancelled() && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                return Err(EngineError::Cancelled);
            }
            if request.get("cut").is_some() {
                return Err(EngineError::CutToolCall("interrupted".to_owned()));
            }
            Ok(json!({"echo": request}))
        }
    }

    fn lines(out: &[u8]) -> alloc::vec::Vec<String> {
        String::from_utf8_lossy(out)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_worker_says_what_it_runs_answers_each_request_by_its_number_and_ends_with_its_input() {
        let mut sent = String::new();
        sent.push_str(&chat_line(7, &json!({"hello": 1})));
        sent.push('\n');
        sent.push_str(&chat_line(8, &json!({"cut": true})));
        sent.push('\n');
        let mut out = alloc::vec::Vec::new();
        serve(|| Ok(Echo), Cursor::new(sent), &mut out).expect("serves");
        let said = lines(&out);
        assert_eq!(
            hello(&said[0]),
            Some(Hello::Ready(ModelInfo::new("m").tools(true).context(4096)))
        );
        let mut answers: alloc::vec::Vec<_> = said[1..].iter().filter_map(|l| answer(l)).collect();
        answers.sort_by_key(|(id, _)| *id);
        assert_eq!(answers[0], (7, Ok(json!({"echo": {"hello": 1}}))));
        assert_eq!(
            answers[1],
            (8, Err(EngineError::CutToolCall("interrupted".to_owned())))
        );
    }

    #[test]
    fn a_cancel_line_stops_the_request_it_names_and_nothing_else() {
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        let reader = ChannelLines(receiver, alloc::vec::Vec::new());
        let out = Arc::new(Mutex::new(alloc::vec::Vec::<u8>::new()));
        let sink = Shared(Arc::clone(&out));
        let worker =
            std::thread::spawn(move || serve(|| Ok(Echo), std::io::BufReader::new(reader), sink));
        sender
            .send(chat_line(1, &json!({"wait": true})) + "\n")
            .expect("sent");
        sender
            .send(chat_line(2, &json!({"x": 2})) + "\n")
            .expect("sent");
        // The second answers while the first is held.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let text = String::from_utf8_lossy(&out.lock().expect("lock")).into_owned();
            if text.contains("\"echo\"") {
                assert!(!text.contains("cancelled"), "{text}");
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "the second never answered"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        sender.send(cancel_line(1) + "\n").expect("sent");
        drop(sender);
        worker.join().expect("ends").expect("serves");
        let said = lines(&out.lock().expect("lock"));
        let first = said
            .iter()
            .filter_map(|l| answer(l))
            .find(|(id, _)| *id == 1);
        assert_eq!(first, Some((1, Err(EngineError::Cancelled))));
    }

    #[test]
    fn an_engine_that_cannot_open_is_said_and_the_worker_ends() {
        let mut out = alloc::vec::Vec::new();
        serve(
            || Err::<Echo, _>(EngineError::Failed("no library".to_owned())),
            Cursor::new(String::new()),
            &mut out,
        )
        .expect("serves");
        assert_eq!(
            hello(&lines(&out)[0]),
            Some(Hello::Failed("no library".to_owned()))
        );
    }

    #[test]
    fn an_error_goes_across_as_the_kind_it_was() {
        for error in [
            EngineError::Cancelled,
            EngineError::CutToolCall("c".to_owned()),
            EngineError::NotLoaded("m".to_owned()),
            EngineError::Refused("r".to_owned()),
            EngineError::Failed("f".to_owned()),
        ] {
            let back = error_from(&error_json(&error));
            match (&error, &back) {
                (EngineError::Cancelled, EngineError::Cancelled) => {}
                _ => assert_eq!(back, error),
            }
        }
    }

    /// Lines arriving from another thread, as a pipe's input does.
    struct ChannelLines(std::sync::mpsc::Receiver<String>, alloc::vec::Vec<u8>);

    impl std::io::Read for ChannelLines {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.1.is_empty() {
                match self.0.recv() {
                    Ok(line) => self.1 = line.into_bytes(),
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.1.len());
            buf[..n].copy_from_slice(&self.1[..n]);
            self.1.drain(..n);
            Ok(n)
        }
    }

    struct Shared(Arc<Mutex<alloc::vec::Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
