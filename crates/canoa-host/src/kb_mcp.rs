//! The `canoa-kb` MCP server: what lets an agent search, list and read the knowledge files of the overlays its session has
//! (`kb_search`, `kb_list`, `kb_read`). coqui-aos answers the tools and makes the indexes; this holds what a host adds: the
//! loopback server and its bearers, the configuration, the indexes' place and the `llm` search mode.
//!
//! It is offered the way `canoa-store` is. A client asks for it in `initialize` (`_meta["dev.boriken.canoa/kb"]` true), the host
//! adds one server to each session request over loopback HTTP behind a bearer minted for that session or, for an agent that
//! takes only stdio, through `PROGRAM mcp-bridge` with the bearer in its environment, and the bearer ends with the connection.
//! A session sees the overlays its connection applies, which its loadout chose, and nothing else. The tools only read.
//! `[host.kb] tools = false` turns it off, and the host needs a state directory to keep its indexes in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

mod embedder;
mod http;
mod library;
mod rank;
mod search;
mod tools;

use core::time::Duration;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use canoa::network::Network;
use coqui_aos::index::Settings;
use coqui_aos::{CallerFacts, KnowledgeOverlay, KnowledgeSettings};
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub(crate) use self::library::Source;
use self::library::{Builder, Library, Slot};
use self::rank::Ranker;
use crate::config::KbConfig;
use crate::delegate::BridgeCommand;

/// The `_meta` key of `initialize` that asks for the knowledge tools.
pub(crate) const META: &str = "dev.boriken.canoa/kb";
/// The name of the MCP server the host adds to a session.
pub(crate) const SERVER_NAME: &str = "canoa-kb";
/// The built-in skill that says when to use the tools, delivered with a session that has them.
pub(crate) const SKILL_NAME: &str = "canoa-kb";
const SKILL: &str = include_str!("kb_mcp/canoa-kb.md");
/// The most passages the model of the `llm` mode is shown, when `[host.kb]` does not say.
const DEFAULT_CANDIDATES: usize = 10;
/// How long the model of the `llm` mode has to order them, when `[host.kb]` does not say.
const DEFAULT_LLM_SECS: u64 = 30;
/// Requests the host remembers an answer for.
const MAX_PENDING: usize = 64;
/// Sessions of one connection that hold a bearer.
const MAX_SESSIONS: usize = 256;

/// The key held in the variable `[host.kb] <setting>` names, for the server of `what`.
fn key_from(setting: &str, name: Option<&str>, what: &str) -> Option<String> {
    let name = name?;
    match std::env::var(name) {
        Ok(key) if !key.is_empty() => Some(key),
        _ => {
            tracing::warn!(
                "[host.kb] {setting} names {name}, which is not set: the {what} server is asked with no key"
            );
            None
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The files of the built-in skill, by path, for a session's skills.
pub(crate) fn skill_files() -> std::collections::BTreeMap<String, Vec<u8>> {
    [("SKILL.md".to_owned(), SKILL.as_bytes().to_vec())].into()
}

/// Whether an `initialize` asks for the knowledge tools.
pub(crate) fn asked_in(message: &[u8]) -> bool {
    if !contains(message, b"initialize") || !contains(message, META.as_bytes()) {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(message) else {
        return false;
    };
    value.get("method") == Some(&json!("initialize"))
        && value
            .pointer("/params/_meta")
            .and_then(|meta| meta.get(META))
            == Some(&json!(true))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Who the agent is to the templates of the knowledge it reads: what the connection's profile and loadout are, and the
/// session's own variables.
#[derive(Clone, Debug, Default)]
pub(crate) struct Caller {
    /// The loadout the connection started with.
    pub(crate) role: Option<String>,
    /// The profile it applies.
    pub(crate) profile: Option<String>,
    /// The session's own variables and their values; a sensitive one is never given to a template.
    pub(crate) env: Vec<(String, String)>,
}

impl Caller {
    fn facts(&self) -> CallerFacts {
        let mut facts = CallerFacts::new();
        if let Some(role) = &self.role {
            facts = facts.role(role);
        }
        if let Some(profile) = &self.profile {
            facts = facts.profile(profile);
        }
        for (name, value) in &self.env {
            facts = facts.env(name, value).allow(name);
        }
        facts
    }
}

/// What one bearer may do: read the knowledge of the overlays its connection applies.
pub(crate) struct Access {
    sources: Vec<Source>,
    /// The sources as the knowledge tools take them, in the same order.
    overlays: Vec<KnowledgeOverlay>,
    caller: CallerFacts,
    builder: Arc<Builder>,
    library: Arc<Library>,
    /// The network the connection's profile chose, which the requests made for its searches go through: its query's embedding and
    /// the model that orders passages. An index is made once for every connection that applies the overlay, so the documents'
    /// embeddings are asked through the process's network.
    network: Arc<Network>,
    calls: AtomicUsize,
}

impl Access {
    /// The index of `source`'s version, started the first time anyone asks.
    fn slot(&self, source: &Source) -> Arc<Slot> {
        self.library.slot(&self.builder, source)
    }
}

/// The bearers in force, by the digest of the token: the token itself is never kept.
#[derive(Default)]
pub(crate) struct Tokens(Mutex<HashMap<[u8; 32], Arc<Access>>>);

impl Tokens {
    fn add(&self, digest: [u8; 32], access: Arc<Access>) {
        lock(&self.0).insert(digest, access);
    }

    fn find(&self, digest: &[u8; 32]) -> Option<Arc<Access>> {
        lock(&self.0).get(digest).cloned()
    }

    fn remove(&self, digest: &[u8; 32]) {
        lock(&self.0).remove(digest);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.0).len()
    }
}

/// What the host holds for every connection: whether the tools are on, how indexes are made, and the server.
pub(crate) struct Hub {
    enabled: bool,
    builder: Option<Arc<Builder>>,
    library: Arc<Library>,
    /// What runs the stdio bridge, for an agent that takes no HTTP MCP server.
    bridge: BridgeCommand,
    tokens: Arc<Tokens>,
    server: Mutex<Option<http::Listener>>,
}

impl Hub {
    /// The hub for `config`. Indexes are kept under `state`; with none the tools are off, since there is nowhere to keep them.
    pub(crate) fn new(
        config: &KbConfig,
        state: Option<PathBuf>,
        bridge: BridgeCommand,
    ) -> Arc<Self> {
        let mut settings = Settings::default();
        // The knowledge of a served overlay is every file the tools serve, whatever its name.
        settings.include = vec!["**/*".to_owned()];
        if let Some(bytes) = config.chunk_bytes {
            settings.chunk_bytes = bytes;
        }
        if let Some(bytes) = config.overlap_bytes {
            settings.overlap_bytes = bytes;
        }
        settings.document_prefix = config.document_prefix.clone().unwrap_or_default();
        settings.query_prefix = config.query_prefix.clone().unwrap_or_default();
        let embedder = config
            .embeddings
            .as_deref()
            .zip(config.model.as_deref())
            .map(|(address, model)| {
                let mut embedder = embedder::NetworkEmbedder::new(address, model);
                if let Some(key) =
                    key_from("api_key_env", config.api_key_env.as_deref(), "embeddings")
                {
                    embedder = embedder.with_bearer(key);
                }
                embedder
            });
        let ranker = config
            .llm_endpoint
            .as_deref()
            .zip(config.llm_model.as_deref())
            .map(|(address, model)| {
                let mut ranker = Ranker::new(
                    address,
                    model,
                    config.llm_candidates.unwrap_or(DEFAULT_CANDIDATES),
                    Duration::from_secs(config.llm_timeout_secs.unwrap_or(DEFAULT_LLM_SECS)),
                );
                if let Some(key) =
                    key_from("llm_api_key_env", config.llm_api_key_env.as_deref(), "llm")
                {
                    ranker = ranker.with_bearer(key);
                }
                ranker
            });
        let builder = state.map(|state| {
            Arc::new(Builder {
                state,
                settings,
                embedder,
                knowledge: KnowledgeSettings::default(),
                ranker,
            })
        });
        Arc::new(Self {
            enabled: config.tools.unwrap_or(true),
            builder,
            library: Arc::new(Library::default()),
            bridge,
            tokens: Arc::new(Tokens::default()),
            server: Mutex::new(None),
        })
    }

    /// The server's address, started the first time it is wanted.
    fn url(self: &Arc<Self>) -> Option<String> {
        let mut server = lock(&self.server);
        if server.is_none() {
            *server = http::start(self).ok();
        }
        server
            .as_ref()
            .map(|listener| format!("http://{}/mcp", listener.addr()))
    }

    /// The knowledge tools of one connection that applies `sources`, or `None` when they are off, there is no state to keep
    /// indexes in, or no overlay has knowledge. `asked` is whether the client's `initialize` asked for them; `caller` is who
    /// its templates see. `network` is the one its profile chose for the requests made for it; the process's when `None`.
    pub(crate) fn for_connection(
        self: &Arc<Self>,
        sources: Vec<Source>,
        caller: &Caller,
        network: Option<Arc<Network>>,
        asked: bool,
    ) -> Option<Arc<Link>> {
        let builder = self.builder.as_ref().filter(|_| self.enabled && asked)?;
        if sources.is_empty() {
            return None;
        }
        Some(Arc::new(Link {
            hub: Arc::clone(self),
            access: Arc::new(Access {
                overlays: sources.iter().map(Source::overlay).collect(),
                sources,
                caller: caller.facts(),
                builder: Arc::clone(builder),
                library: Arc::clone(&self.library),
                network: network.unwrap_or_else(Network::current),
                calls: AtomicUsize::new(0),
            }),
            state: Mutex::new(State::default()),
            closed: AtomicBool::new(false),
        }))
    }
}

/// A request of the client's the host remembers until the agent answers it.
enum Pending {
    Initialize,
    Open { digest: [u8; 32] },
}

#[derive(Default)]
struct State {
    /// The agent takes MCP servers over HTTP.
    http: bool,
    /// The ACP version the agent answered `initialize` in when it takes stdio servers, else 0.
    stdio: u64,
    pending: HashMap<String, Pending>,
    digests: Vec<[u8; 32]>,
}

/// One client connection that is given the knowledge tools.
pub(crate) struct Link {
    hub: Arc<Hub>,
    access: Arc<Access>,
    state: Mutex<State>,
    closed: AtomicBool,
}

impl Link {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Ends every bearer of the connection.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut state = lock(&self.state);
        for digest in state.digests.drain(..) {
            self.hub.tokens.remove(&digest);
        }
    }

    fn remember(&self, id: String, pending: Pending) {
        let mut state = lock(&self.state);
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(id, pending);
        }
    }

    /// A message from the client after the scope has checked it. A session request gets the host's entry added.
    #[allow(
        clippy::collapsible_match,
        reason = "the request id moves into the call, which a guard cannot do"
    )]
    fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Vec<u8> {
        // JSON may escape any character, so a message with a backslash is read whatever its bytes say.
        if !(contains(&message, b"session/")
            || contains(&message, b"initialize")
            || message.contains(&b'\\'))
        {
            return message;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return message;
        };
        let (Some(method), Some(id)) = (
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned),
            value.get("id").map(Value::to_string),
        ) else {
            return message;
        };
        match method.as_str() {
            "initialize" => self.remember(id, Pending::Initialize),
            "session/new" | "session/load" | "session/resume" => {
                if self.open(&mut value, id) {
                    return serde_json::to_vec(&value).unwrap_or(message);
                }
            }
            _ => {}
        }
        message
    }

    /// Adds the host's MCP server, with a bearer of its own, to a session request. Whether the request was changed.
    fn open(self: &Arc<Self>, value: &mut Value, id: String) -> bool {
        let (http, stdio, count) = {
            let state = lock(&self.state);
            (state.http, state.stdio, state.digests.len())
        };
        if self.is_closed() || count >= MAX_SESSIONS || !(http || stdio > 0) {
            return false;
        }
        let Some(url) = self.hub.url() else {
            return false;
        };
        let mut secret = [0_u8; 32];
        if getrandom::fill(&mut secret).is_err() {
            return false;
        }
        let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let entry = if http {
            json!({
                "type": "http",
                "name": SERVER_NAME,
                "url": url,
                "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}],
            })
        } else {
            // The same stdio bridge the other host tools use, with the address and bearer in its environment. Version 1
            // writes a stdio server with no `type`; version 2 names it.
            let mut entry = json!({
                "name": SERVER_NAME,
                "command": self.hub.bridge.program,
                "args": self.hub.bridge.args,
                "env": [
                    {"name": crate::delegate::bridge::URL_VAR, "value": url},
                    {"name": crate::delegate::bridge::TOKEN_VAR, "value": token},
                ],
            });
            if stdio >= 2 {
                entry["type"] = json!("stdio");
            }
            entry
        };
        let Some(servers) = value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .map(|params| params.entry("mcpServers").or_insert_with(|| json!([])))
            .and_then(Value::as_array_mut)
        else {
            return false;
        };
        servers.retain(|server| server.get("name").and_then(Value::as_str) != Some(SERVER_NAME));
        servers.push(entry);
        self.hub.tokens.add(digest, Arc::clone(&self.access));
        lock(&self.state).digests.push(digest);
        self.remember(id, Pending::Open { digest });
        tracing::debug!("a session was given the knowledge tools");
        true
    }

    /// A message from the agent, before it goes to the client.
    fn agent_said(&self, message: &[u8]) {
        if lock(&self.state).pending.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        if value.get("method").is_some() {
            return;
        }
        let Some(id) = value.get("id").map(Value::to_string) else {
            return;
        };
        let pending = lock(&self.state).pending.remove(&id);
        let ok = value.get("result").filter(|_| value.get("error").is_none());
        match (pending, ok) {
            (Some(Pending::Initialize), Some(result)) => {
                let takes = |capability: Option<&Value>| {
                    capability.is_some_and(|said| said == &json!(true) || said.is_object())
                };
                let version = result
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let mut state = lock(&self.state);
                state.http = takes(
                    result
                        .pointer("/agentCapabilities/mcpCapabilities/http")
                        .or_else(|| result.pointer("/capabilities/session/mcp/http")),
                );
                // Version 1 has stdio in its baseline; version 2 says it.
                let stdio = version < 2 || takes(result.pointer("/capabilities/session/mcp/stdio"));
                state.stdio = if stdio { version } else { 0 };
            }
            // A session the agent did not open has no use for its bearer.
            (Some(Pending::Open { digest }), None) => {
                self.hub.tokens.remove(&digest);
                lock(&self.state).digests.retain(|held| *held != digest);
            }
            _ => {}
        }
    }
}

/// A channel that gives a session the knowledge server, sitting outside the scope and the profile where a session request has
/// been checked, and that ends the bearers with the connection.
pub(crate) struct KbLayer<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> KbLayer<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }
}

impl<C: Channel + ?Sized> Drop for KbLayer<'_, C> {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.close();
        }
    }
}

impl<C: Channel + ?Sized> Channel for KbLayer<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if let Some(link) = &self.link {
            link.agent_said(message);
        }
        self.inner.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        Ok(match (received, &self.link) {
            (Some(message), Some(link)) => Some(link.client_said(message)),
            (received, _) => received,
        })
    }

    fn close(&self) {
        if let Some(link) = &self.link {
            link.close();
        }
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

#[cfg(test)]
mod tests;
