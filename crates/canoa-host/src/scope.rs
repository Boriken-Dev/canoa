//! What a client may send a stdio agent through this host. ACP lets `session/new` name a stdio MCP server, which the agent then
//! starts as a program on this machine with no question asked; `_meta` may carry an agent's private launch options. Neither
//! reaches the agent: the program the host was told to run is the only one a client gets started.
//!
//! The one `_meta` that does is the usage extension's opt-in on `initialize`, rebuilt by the host: the client's wish for
//! notifications and the host's own account id, never an account the client names. The agent's usage reports leave with that
//! account too.

use alloc::borrow::{Cow, ToOwned};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use coqui_acp_api::{USAGE_CAPABILITY, USAGE_NOTIFICATION};
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value, json};

use crate::config::plain_absolute;
use crate::usage_splice::{Antigravity, Client, Inference, Splice};
use crate::worktree::Worktrees;

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
    /// The host's id for the account this entry's agent runs on, which it tells the agent and puts on every usage report.
    account: Option<String>,
    /// What the entry's failed turns have shown of its limits, for an agent that reports none; shared by its connections.
    usage: Arc<Inference>,
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
    /// The directories this scope confines a client to; none leaves it unconfined.
    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(crate) fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            account: None,
            usage: Arc::new(Inference::default()),
        }
    }

    /// This scope reading failed turns of an agent that reports no usage with `phrases`: `None` for coqui-acp's defaults, a
    /// list instead of them, an empty one not at all.
    #[must_use]
    pub(crate) fn limit_phrases(mut self, phrases: Option<Vec<String>>) -> Self {
        self.usage = Arc::new(Inference::new(phrases));
        self
    }

    /// This scope reading an Antigravity agent's quota too, as `command` runs it.
    #[must_use]
    pub(crate) fn antigravity(mut self, command: Antigravity) -> Self {
        self.usage = Arc::new(Inference::new(self.usage.phrases()).with_antigravity(command));
        self
    }

    /// One connection's share in the entry's inferred usage.
    pub(crate) fn splice(&self) -> Splice {
        Splice::new(Arc::clone(&self.usage), self.account.clone())
    }

    /// This scope with `account` as the id usage is reported under.
    #[must_use]
    pub(crate) fn account(mut self, account: String) -> Self {
        self.account = Some(account);
        self
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
        let notify = asks_usage_notices(object);
        strip_meta(object);
        self.tell_usage(object, notify);
        let id = object.get("id").cloned();
        let is_request = object.contains_key("method") && id.is_some();
        if Worktrees::is_request(message) {
            return Verdict::Refuse(is_request.then(|| {
                error(
                    id.as_ref().unwrap_or(&Value::Null),
                    INVALID_REQUEST,
                    "a worktree request comes first on a connection, before the agent starts",
                )
            }));
        }
        // ACP's parameters are always an object. A list would fill a request's fields in order, past every check below.
        let checked = match object.get("params") {
            None => Ok(()),
            Some(Value::Object(params)) => self.check_params(params),
            Some(_) => Err("params must be an object"),
        };
        if let Err(why) = checked {
            return Verdict::Refuse(
                is_request.then(|| error(id.as_ref().unwrap_or(&Value::Null), INVALID_PARAMS, why)),
            );
        }
        match serde_json::to_vec(&value) {
            Ok(bytes) => Verdict::Forward(bytes),
            Err(_) => Verdict::Refuse(None),
        }
    }

    /// Puts the usage extension's opt-in back on an `initialize` that `strip_meta` cleared: the client's own wish for
    /// notifications, and the account only the host names.
    fn tell_usage(&self, object: &mut Map<String, Value>, notify: bool) {
        if object.get("method").and_then(Value::as_str) != Some("initialize")
            || !(notify || self.account.is_some())
        {
            return;
        }
        let Some(Value::Object(params)) = object.get_mut("params") else {
            return;
        };
        let key = if params.get("protocolVersion").and_then(Value::as_u64) >= Some(2) {
            "capabilities"
        } else {
            "clientCapabilities"
        };
        let capabilities = params.entry(key).or_insert_with(|| json!({}));
        let Some(capabilities) = capabilities.as_object_mut() else {
            return;
        };
        let mut opt_in = Map::new();
        opt_in.insert("notify".to_owned(), Value::Bool(notify));
        if let Some(account) = &self.account {
            opt_in.insert("account".to_owned(), Value::String(account.clone()));
        }
        capabilities.insert("_meta".to_owned(), json!({ USAGE_CAPABILITY: opt_in }));
    }

    /// An agent's message to the client with the host's account on the usage report it carries, whatever account the agent
    /// named: the notification's report, and the answer to `_coqui_acp/usage`, which is the object that has a `supported`
    /// and a `buckets`. Anything else, and every message when this scope names no account, passes as it came.
    pub(crate) fn stamp<'a>(&self, message: &'a [u8]) -> Cow<'a, [u8]> {
        let Some(account) = &self.account else {
            return Cow::Borrowed(message);
        };
        // A cheap look first: most of an agent's messages are streamed text.
        if !contains(message, b"supported") && !contains(message, USAGE_NOTIFICATION.as_bytes()) {
            return Cow::Borrowed(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return Cow::Borrowed(message);
        };
        let report = if value.get("method").and_then(Value::as_str) == Some(USAGE_NOTIFICATION) {
            value.pointer_mut("/params/report")
        } else if value.get("method").is_none() {
            value.get_mut("result")
        } else {
            None
        };
        let Some(report) = report.and_then(Value::as_object_mut) else {
            return Cow::Borrowed(message);
        };
        if !(report.get("supported").is_some_and(Value::is_boolean)
            && report.get("buckets").is_some_and(Value::is_array))
        {
            return Cow::Borrowed(message);
        }
        report.insert("account".to_owned(), Value::String(account.clone()));
        serde_json::to_vec(&value).map_or(Cow::Borrowed(message), Cow::Owned)
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

/// Whether an `initialize` asks for usage notifications in its usage opt-in, which is in the `_meta` the host removes.
fn asks_usage_notices(object: &Map<String, Value>) -> bool {
    object.get("method").and_then(Value::as_str) == Some("initialize")
        && ["clientCapabilities", "capabilities"].iter().any(|key| {
            object
                .get("params")
                .and_then(|params| params.get(*key))
                .and_then(|capabilities| {
                    capabilities
                        .get("_meta")?
                        .get(USAGE_CAPABILITY)?
                        .get("notify")
                })
                == Some(&Value::Bool(true))
        })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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
    splice: Splice,
}

impl<'a, C: Channel + ?Sized> Scoped<'a, C> {
    /// This channel serves the agent running as process `pid`.
    #[must_use]
    pub(crate) fn agent(mut self, pid: u32) -> Self {
        self.splice = self.splice.agent(pid);
        self
    }

    pub(crate) fn new(inner: &'a C, scope: &'a Scope) -> Self {
        Self {
            inner,
            scope,
            splice: scope.splice(),
        }
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

thread_local! {
    /// Whether this thread is sending what the host itself says to its client.
    static HOST_OWN: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Runs `send`, which puts a message of the host's own on the client's stream: the one filter below lets it pass, which no
/// message of an agent's can ask for.
pub(crate) fn host_own<T>(send: impl FnOnce() -> T) -> T {
    let before = HOST_OWN.with(|own| own.replace(true));
    let done = send();
    HOST_OWN.with(|own| own.set(before));
    done
}

/// Whether an agent's `message` is a request or notification of the host's own, whose method starts with `_canoa/`: the host asks
/// its client those, so one an agent sends is never passed on, or the client could be made to show the host's questions for it.
pub(crate) fn is_host_only(message: &[u8]) -> bool {
    !HOST_OWN.with(core::cell::Cell::get)
        && contains(message, b"_canoa/")
        && serde_json::from_slice::<Value>(message).is_ok_and(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|method| method.starts_with("_canoa/"))
        })
}

impl<C: Channel + ?Sized> Channel for Scoped<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if is_host_only(message) {
            tracing::info!("an agent's `_canoa/` request was dropped");
            return Ok(());
        }
        self.splice
            .relay(message, |sent| self.inner.send(&self.scope.stamp(sent)))
    }

    /// A refused message reads as a timeout, after which the channel is still usable, so a caller polling for its own
    /// reasons is never held by a peer that only sends what is refused.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        self.splice
            .tick(|sent| self.inner.send(&self.scope.stamp(sent)));
        let Some(message) = self.inner.receive(timeout)? else {
            return Ok(None);
        };
        match self.scope.check(&message) {
            Verdict::Forward(bytes) => match self.splice.read_client(&bytes) {
                Client::Answer(answer) => {
                    self.reply(&answer)?;
                    Err(Failure::Timeout)
                }
                Client::Held => Err(Failure::Timeout),
                Client::Forward => Ok(Some(bytes)),
            },
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
