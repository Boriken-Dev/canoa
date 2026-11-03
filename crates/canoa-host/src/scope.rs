//! What a client may send a stdio agent through this host. ACP lets `session/new` name a stdio MCP server, which the agent then
//! starts as a program on this machine with no question asked; `_meta` may carry an agent's private launch options. Neither
//! reaches the agent: the program the host was told to run is the only one a client gets started.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use coqui_web::{Channel, Failure};
use serde_json::{Map, Value, json};

use crate::config::plain_absolute;

/// JSON-RPC's code for a message that is not JSON.
const PARSE_ERROR: i64 = -32700;
/// JSON-RPC's code for JSON that is not one request, notification or response.
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC's code for parameters the receiver will not take.
const INVALID_PARAMS: i64 = -32602;
/// How long a refusal may wait on a peer that is not taking messages.
const REPLY_LIMIT: Duration = Duration::from_secs(30);
const REPLY_POLL: Duration = Duration::from_millis(10);

/// The rule for one entry's clients.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scope {
    roots: Vec<PathBuf>,
}

/// What becomes of one message from the client.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    /// Passes, as this serialization of what was parsed.
    Forward(Vec<u8>),
    /// Stops here, answered with this when it was a request.
    Refuse(Option<Vec<u8>>),
}

impl Scope {
    pub(crate) const fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// Parses `message` and decides. What passes is the host's own serialization, so a parser that read duplicate keys
    /// differently cannot see something else than was checked.
    pub(crate) fn check(&self, message: &[u8]) -> Verdict {
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return Verdict::Refuse(Some(error(&Value::Null, PARSE_ERROR, "not JSON")));
        };
        let Some(object) = value.as_object_mut() else {
            return Verdict::Refuse(Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "one JSON-RPC object per message",
            )));
        };
        strip_meta(object);
        let id = object.get("id").cloned();
        let is_request = object.contains_key("method") && id.is_some();
        if let Some(params) = object.get("params").and_then(Value::as_object)
            && let Err(why) = self.check_params(params)
        {
            return Verdict::Refuse(
                is_request.then(|| error(id.as_ref().unwrap_or(&Value::Null), INVALID_PARAMS, why)),
            );
        }
        match serde_json::to_vec(&value) {
            Ok(bytes) => Verdict::Forward(bytes),
            Err(_) => Verdict::Refuse(None),
        }
    }

    fn check_params(&self, params: &Map<String, Value>) -> Result<(), &'static str> {
        if let Some(servers) = params.get("mcpServers") {
            let servers = servers.as_array().ok_or("mcpServers is not a list")?;
            for server in servers {
                check_server(server)?;
            }
        }
        if self.roots.is_empty() {
            return Ok(());
        }
        if let Some(cwd) = params.get("cwd") {
            self.check_directory(cwd)?;
        }
        if let Some(more) = params.get("additionalDirectories") {
            for directory in more
                .as_array()
                .ok_or("additionalDirectories is not a list")?
            {
                self.check_directory(directory)?;
            }
        }
        Ok(())
    }

    fn check_directory(&self, directory: &Value) -> Result<(), &'static str> {
        let path = Path::new(directory.as_str().ok_or("a directory is not text")?);
        if plain_absolute(path) && self.roots.iter().any(|root| path.starts_with(root)) {
            Ok(())
        } else {
            Err("this host serves this agent only under its working roots")
        }
    }
}

/// An MCP server the agent would reach over the network, which starts nothing here. One with a command, or of any type
/// but these, is refused whole.
fn check_server(server: &Value) -> Result<(), &'static str> {
    const STDIO: &str = "a stdio MCP server would start a program on this machine, which this host does not do for a client";
    let server = server.as_object().ok_or("an MCP server is not an object")?;
    if server.contains_key("command") {
        return Err(STDIO);
    }
    match server.get("type").and_then(Value::as_str) {
        Some("http" | "sse") => {
            let url = server.get("url").and_then(Value::as_str).unwrap_or("");
            if url.len() > 8
                && url
                    .get(..8)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
            {
                Ok(())
            } else {
                Err("an http or sse MCP server must be https")
            }
        }
        Some("acp") => Ok(()),
        _ => Err(STDIO),
    }
}

/// Removes `_meta` at every depth: an agent may give it private meaning, such as options for how it starts a program.
fn strip_meta(object: &mut Map<String, Value>) {
    object.remove("_meta");
    for value in object.values_mut() {
        strip_value(value);
    }
}

fn strip_value(value: &mut Value) {
    match value {
        Value::Object(object) => strip_meta(object),
        Value::Array(items) => items.iter_mut().for_each(strip_value),
        _ => {}
    }
}

fn error(id: &Value, code: i64, message: &str) -> Vec<u8> {
    let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    serde_json::to_vec(&reply).unwrap_or_default()
}

/// A channel whose incoming messages pass [`Scope::check`]. A refused request is answered on the channel and never reaches
/// the agent.
pub(crate) struct Scoped<'a, C: Channel + ?Sized> {
    inner: &'a C,
    scope: &'a Scope,
}

impl<'a, C: Channel + ?Sized> Scoped<'a, C> {
    pub(crate) const fn new(inner: &'a C, scope: &'a Scope) -> Self {
        Self { inner, scope }
    }

    fn reply(&self, message: &[u8]) -> Result<(), Failure> {
        let until = Instant::now() + REPLY_LIMIT;
        loop {
            match self.inner.send(message) {
                Err(Failure::NoProgress) if Instant::now() < until => thread::sleep(REPLY_POLL),
                other => return other,
            }
        }
    }
}

impl<C: Channel + ?Sized> Channel for Scoped<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inner.send(message)
    }

    /// A refused message reads as a timeout, after which the channel is still usable, so a caller polling for its own
    /// reasons is never held by a peer that only sends what is refused.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(message) = self.inner.receive(timeout)? else {
            return Ok(None);
        };
        match self.scope.check(&message) {
            Verdict::Forward(bytes) => Ok(Some(bytes)),
            Verdict::Refuse(answer) => {
                if let Some(answer) = answer {
                    self.reply(&answer)?;
                }
                tracing::info!("a message was refused: {}", refusal_kind(&message));
                Err(Failure::Timeout)
            }
        }
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// The method of a refused message, if it named a short one, for the log. Nothing else of a client's is logged.
fn refusal_kind(message: &[u8]) -> String {
    serde_json::from_slice::<Value>(message)
        .ok()
        .and_then(|value| value.get("method")?.as_str().map(ToOwned::to_owned))
        .filter(|method| method.len() <= 64 && method.bytes().all(|byte| byte.is_ascii_graphic()))
        .unwrap_or_else(|| "a message".to_owned())
}

#[cfg(test)]
mod tests;
