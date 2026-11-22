//! Lists the tools and resources of the MCP servers a session was given (`_canoa/mcp/list`).
//!
//! The `mcpServers` of each session request are read as they leave for the agent. An HTTP server, which the host's own servers
//! are as well, is asked directly. A stdio server the agent starts for itself is not started a second time: it is reported as
//! pending, since only the agent has it. A stdio server the host's profile gave the session is started once more, in the
//! session's folder and environment, to be asked and ended.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, sockets, pipes, and clocks are std's"
)]

use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::Instant;

use canoa::network::Network;
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value, json};

use crate::config::McpListConfig;
use crate::spawn::{Command, Stdio};

/// The session-channel RPC method that lists attached MCP servers' tools and resources.
pub(crate) const METHOD: &str = "_canoa/mcp/list";

const POLL: Duration = Duration::from_millis(50);
/// The items in a page of the host's answer when the request names no `limit`.
const DEFAULT_LIMIT: usize = 100;
const INVALID_PARAMS: i64 = -32602;
const MCP_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// What the host's profile gave the sessions of a connection: the names of its servers, and the environment the agent runs in.
#[derive(Clone)]
pub(crate) struct Given(Arc<GivenState>);

struct GivenState {
    names: Mutex<BTreeSet<String>>,
    environment: Vec<(String, String)>,
}

impl Given {
    /// The names of nothing yet, and the environment `connection` starts its agent with.
    pub(crate) fn for_connection(connection: Option<&crate::profile::hosted::Connection>) -> Self {
        Self::new(connection.map_or_else(Vec::new, |connection| {
            connection
                .env()
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().to_owned()))
                .collect()
        }))
    }

    fn new(environment: Vec<(String, String)>) -> Self {
        Self(Arc::new(GivenState {
            names: Mutex::new(BTreeSet::new()),
            environment,
        }))
    }

    /// Records the names of `servers`, the entries the profile added to a session request.
    pub(crate) fn add<'s>(&self, servers: impl IntoIterator<Item = &'s Value>) {
        lock(&self.0.names).extend(
            servers
                .into_iter()
                .filter_map(|server| server.get("name").and_then(Value::as_str))
                .map(str::to_owned),
        );
    }

    fn holds(&self, name: &str) -> bool {
        lock(&self.0.names).contains(name)
    }
}

/// One attached MCP server parsed from a session request's `mcpServers`.
#[derive(Clone, Debug)]
enum ServerSpec {
    Http {
        name: String,
        url: String,
        headers: Vec<(String, String)>,
    },
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        /// The session's folder, where the agent starts it.
        cwd: Option<String>,
        /// The host's profile gave it, so the host may start it; otherwise the agent starts it and only the agent has it.
        given: bool,
    },
    Unsupported {
        name: String,
        reason: String,
    },
}

impl ServerSpec {
    fn name(&self) -> &str {
        match self {
            Self::Http { name, .. } | Self::Stdio { name, .. } | Self::Unsupported { name, .. } => {
                name
            }
        }
    }
}

/// A full snapshot of tools, resources, and per-server statuses for one session, kept for deterministic `cursor` paging.
#[derive(Clone, Debug)]
struct Snapshot {
    generation: u64,
    server_filter: Option<String>,
    tools: Vec<Value>,
    resources: Vec<Value>,
    servers: Vec<Value>,
}

struct Pending {
    known_session: Option<String>,
    servers: Vec<ServerSpec>,
}

#[derive(Default)]
struct InventoryState {
    pending: BTreeMap<String, Pending>,
    sessions: BTreeMap<String, Vec<ServerSpec>>,
    snapshots: BTreeMap<String, Snapshot>,
    next_generation: u64,
}

/// Tracks attached MCP servers per session on one client connection and answers [`METHOD`] requests.
struct Inventory {
    settings: McpListConfig,
    given: Given,
    state: Mutex<InventoryState>,
    outbox: Mutex<VecDeque<Vec<u8>>>,
    closed: AtomicBool,
}

impl Inventory {
    fn new(settings: McpListConfig, given: Given) -> Arc<Self> {
        Arc::new(Self {
            settings,
            given,
            state: Mutex::new(InventoryState::default()),
            outbox: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut state = lock(&self.state);
        state.pending.clear();
        state.sessions.clear();
        state.snapshots.clear();
    }

    fn drain(&self) -> Vec<Vec<u8>> {
        lock(&self.outbox).drain(..).collect()
    }

    fn remember(&self, id: String, known_session: Option<String>, servers: Vec<ServerSpec>) {
        lock(&self.state).pending.insert(
            id,
            Pending {
                known_session,
                servers,
            },
        );
    }

    fn session_opened(&self, id: String, resolved_session: Option<String>) {
        let mut state = lock(&self.state);
        let Some(pending) = state.pending.remove(&id) else {
            return;
        };
        let Some(session_id) = resolved_session.or(pending.known_session) else {
            return;
        };
        state.sessions.insert(session_id.clone(), pending.servers);
        state.snapshots.remove(&session_id);
    }

    fn close_session(&self, session_id: &str) {
        let mut state = lock(&self.state);
        state.sessions.remove(session_id);
        state.snapshots.remove(session_id);
    }

    /// Inspects an agent-to-client message so `session/new`, `session/load`, `session/resume`, and `session/fork` responses
    /// bind captured `mcpServers` to the resulting `sessionId`.
    fn agent_said(&self, message: &[u8]) {
        let has_pending = !lock(&self.state).pending.is_empty();
        if !has_pending {
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
        if let Some(result) = value.get("result") {
            let resolved = result
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.session_opened(id, resolved);
        } else if value.get("error").is_some() {
            lock(&self.state).pending.remove(&id);
        }
    }

    /// Inspects a client-to-agent message after all server-injecting layers ran. Returns `None` when [`METHOD`] was intercepted.
    fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Option<Vec<u8>> {
        if !(contains(&message, b"session/") || contains(&message, METHOD.as_bytes())) {
            return Some(message);
        }
        let Ok(value) = serde_json::from_slice::<Value>(&message) else {
            return Some(message);
        };
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Some(message);
        };
        match method {
            m if m == METHOD => {
                let id = value.get("id").cloned();
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                self.spawn_list(id, params);
                None
            }
            "session/new" | "session/load" | "session/resume" | "session/fork" => {
                if let Some(id) = value.get("id").map(Value::to_string) {
                    let known = (method != "session/new")
                        .then(|| {
                            value
                                .pointer("/params/sessionId")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .flatten();
                    let servers = extract_servers(&value, &self.given);
                    self.remember(id, known, servers);
                }
                Some(message)
            }
            "session/close" => {
                if let Some(session_id) = value.pointer("/params/sessionId").and_then(Value::as_str)
                {
                    self.close_session(session_id);
                }
                Some(message)
            }
            _ => Some(message),
        }
    }

    fn spawn_list(self: &Arc<Self>, id: Option<Value>, params: Value) {
        let Some(id) = id else { return };
        let inventory = Arc::clone(self);
        let worker_id = id.clone();
        let work = move || {
            let answer = inventory.answer_list(&params);
            if inventory.is_closed() {
                return;
            }
            let reply = match answer {
                Ok(result) => json!({"jsonrpc": "2.0", "id": worker_id, "result": result}),
                Err(why) => {
                    json!({"jsonrpc": "2.0", "id": worker_id, "error": {"code": INVALID_PARAMS, "message": why}})
                }
            };
            if let Ok(bytes) = serde_json::to_vec(&reply) {
                lock(&inventory.outbox).push_back(bytes);
            }
        };
        if let Err(error) = thread::Builder::new()
            .name("canoa-mcp-list".to_owned())
            .spawn(work)
        {
            tracing::warn!("mcp_list: could not start worker thread: {error}");
            let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "could not list MCP servers"}});
            if let Ok(bytes) = serde_json::to_vec(&reply) {
                lock(&self.outbox).push_back(bytes);
            }
        }
    }

    fn answer_list(&self, params: &Value) -> Result<Value, String> {
        let params = params
            .as_object()
            .ok_or_else(|| "params must be an object".to_owned())?;
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "sessionId is required".to_owned())?;
        let limit = parse_limit(params.get("limit"))?;
        let server_filter = parse_optional_string(params.get("server"), "server")?;
        let cursor = parse_optional_string(params.get("cursor"), "cursor")?;

        if let Some(cursor) = cursor {
            let (generation, offset) = parse_cursor(&cursor)?;
            let state = lock(&self.state);
            let snapshot = state
                .snapshots
                .get(session_id)
                .filter(|snap| snap.generation == generation && snap.server_filter == server_filter)
                .ok_or_else(|| "invalid cursor".to_owned())?;
            return Ok(slice_snapshot(snapshot, offset, limit));
        }

        let specs = {
            let state = lock(&self.state);
            state
                .sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| "no such session".to_owned())?
        };
        let selected: Vec<ServerSpec> = specs
            .into_iter()
            .filter(|spec| {
                server_filter
                    .as_deref()
                    .is_none_or(|wanted| spec.name() == wanted)
            })
            .collect();

        let queried: Vec<ServerResult> = if selected.len() <= 1 {
            selected
                .iter()
                .map(|spec| query_server(spec, &self.settings, &self.given))
                .collect()
        } else {
            thread::scope(|scope| {
                let handles: Vec<_> = selected
                    .iter()
                    .map(|spec| scope.spawn(|| query_server(spec, &self.settings, &self.given)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            ServerResult::error(
                                "unknown",
                                "listing worker terminated unexpectedly".to_owned(),
                            )
                        })
                    })
                    .collect()
            })
        };

        let mut tools = Vec::new();
        let mut resources = Vec::new();
        let mut servers = Vec::with_capacity(queried.len());
        for outcome in queried {
            servers.push(outcome.summary);
            tools.extend(outcome.tools);
            resources.extend(outcome.resources);
        }

        let mut state = lock(&self.state);
        if !state.sessions.contains_key(session_id) {
            return Err("no such session".to_owned());
        }
        state.next_generation = state.next_generation.saturating_add(1);
        let snapshot = Snapshot {
            generation: state.next_generation,
            server_filter,
            tools,
            resources,
            servers,
        };
        let answer = slice_snapshot(&snapshot, 0, limit);
        state.snapshots.insert(session_id.to_owned(), snapshot);
        Ok(answer)
    }
}

fn parse_limit(value: Option<&Value>) -> Result<usize, String> {
    match value {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(Value::Number(num)) => {
            let raw = num
                .as_u64()
                .ok_or_else(|| "limit must be a positive integer".to_owned())?;
            if raw == 0 {
                return Err("limit must be at least 1".to_owned());
            }
            Ok(usize::try_from(raw).unwrap_or(usize::MAX))
        }
        Some(_) => Err("limit must be a positive integer".to_owned()),
    }
}

fn parse_optional_string(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(format!("{field} cannot be empty"));
            }
            Ok(Some(trimmed.to_owned()))
        }
        Some(_) => Err(format!("{field} must be text")),
    }
}

fn parse_cursor(cursor: &str) -> Result<(u64, usize), String> {
    let Some((gen_str, off_str)) = cursor.split_once(':') else {
        return Err("invalid cursor".to_owned());
    };
    let generation = gen_str
        .parse::<u64>()
        .map_err(|_| "invalid cursor".to_owned())?;
    let offset = off_str
        .parse::<usize>()
        .map_err(|_| "invalid cursor".to_owned())?;
    Ok((generation, offset))
}

fn slice_snapshot(snapshot: &Snapshot, offset: usize, limit: usize) -> Value {
    let tool_count = snapshot.tools.len();
    let res_count = snapshot.resources.len();
    let total = tool_count.saturating_add(res_count);
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);

    let t_start = start.min(tool_count);
    let t_end = end.min(tool_count);
    let r_start = start.saturating_sub(tool_count).min(res_count);
    let r_end = end.saturating_sub(tool_count).min(res_count);

    let mut out = json!({
        "tools": snapshot.tools[t_start..t_end],
        "resources": snapshot.resources[r_start..r_end],
        "servers": snapshot.servers,
    });
    if end < total {
        out["nextCursor"] = json!(format!("{}:{end}", snapshot.generation));
    }
    out
}

/// Parses `params.mcpServers` into connection specs. Never fails the session request; unrecognised servers become `Unsupported`.
fn extract_servers(request: &Value, given: &Given) -> Vec<ServerSpec> {
    let Some(list) = request
        .pointer("/params/mcpServers")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let cwd = request
        .pointer("/params/cwd")
        .and_then(Value::as_str)
        .map(str::to_owned);
    list.iter()
        .filter_map(|entry| parse_server_entry(entry, cwd.as_deref(), given))
        .collect()
}

fn parse_server_entry(entry: &Value, cwd: Option<&str>, given: &Given) -> Option<ServerSpec> {
    let obj = entry.as_object()?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("mcp")
        .to_owned();
    let kind = obj.get("type").and_then(Value::as_str);
    match kind {
        Some("http") => Some(ServerSpec::Http {
            name: name.clone(),
            url: obj
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            headers: parse_pairs(obj.get("headers")),
        }),
        None | Some("stdio") if obj.get("command").is_some() => {
            let env = parse_pairs(obj.get("env"));
            // Host-run internal servers (`canoa-store`, `canoa-kb`, `canoa-delegate`, `canoa-*`) inject their loopback
            // HTTP URL + bearer into `bridge` env when an agent only speaks stdio: query them over HTTP directly.
            if let Some((url, token)) = bridge_http_from_env(&env) {
                return Some(ServerSpec::Http {
                    name,
                    url,
                    headers: vec![("Authorization".to_owned(), format!("Bearer {token}"))],
                });
            }
            let command = obj
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let args = obj
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Some(ServerSpec::Stdio {
                given: given.holds(&name),
                name,
                command,
                args,
                env,
                cwd: cwd.map(str::to_owned),
            })
        }
        Some(other) => Some(ServerSpec::Unsupported {
            name,
            reason: format!("{other} MCP servers cannot be listed by the host"),
        }),
        None => Some(ServerSpec::Unsupported {
            name,
            reason: "MCP server has no supported transport".to_owned(),
        }),
    }
}

fn bridge_http_from_env(env: &[(String, String)]) -> Option<(String, String)> {
    let url = env
        .iter()
        .find(|(k, _)| k == crate::delegate::bridge::URL_VAR)
        .map(|(_, v)| v.clone())?;
    let token = env
        .iter()
        .find(|(k, _)| k == crate::delegate::bridge::TOKEN_VAR)
        .map(|(_, v)| v.clone())?;
    Some((url, token))
}

fn parse_pairs(value: Option<&Value>) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    match value {
        Some(Value::Array(list)) => {
            for item in list {
                if let (Some(k), Some(v)) = (
                    item.get("name").and_then(Value::as_str),
                    item.get("value").and_then(Value::as_str),
                ) {
                    pairs.push((k.to_owned(), v.to_owned()));
                }
            }
        }
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if let Some(v) = v.as_str() {
                    pairs.push((k.clone(), v.to_owned()));
                }
            }
        }
        _ => {}
    }
    pairs
}

struct ServerResult {
    summary: Value,
    tools: Vec<Value>,
    resources: Vec<Value>,
}

impl ServerResult {
    fn ok(name: &str, tools: Vec<Value>, resources: Vec<Value>) -> Self {
        Self {
            summary: json!({
                "name": name,
                "status": "ok",
                "tools": tools.len(),
                "resources": resources.len(),
            }),
            tools,
            resources,
        }
    }

    fn pending(name: &str) -> Self {
        Self {
            summary: json!({
                "name": name,
                "status": "pending",
                "note": "the agent starts this server: its tools are listed when the agent reports them",
                "tools": 0,
                "resources": 0,
            }),
            tools: Vec::new(),
            resources: Vec::new(),
        }
    }

    fn error(name: &str, reason: String) -> Self {
        Self {
            summary: json!({
                "name": name,
                "status": "error",
                "error": reason,
                "tools": 0,
                "resources": 0,
            }),
            tools: Vec::new(),
            resources: Vec::new(),
        }
    }
}

trait McpRpc {
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String>;
    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, RpcError>, String>;
}

struct RpcError {
    code: i64,
    message: String,
}

fn query_server(spec: &ServerSpec, settings: &McpListConfig, given: &Given) -> ServerResult {
    match spec {
        ServerSpec::Unsupported { name, reason } => ServerResult::error(name, reason.clone()),
        ServerSpec::Http { name, url, headers } => {
            let mut transport = match HttpMcp::new(url, headers, settings) {
                Ok(transport) => transport,
                Err(why) => return ServerResult::error(name, why),
            };
            collect_mcp(name, &mut transport, settings)
        }
        ServerSpec::Stdio {
            name, given: false, ..
        } => ServerResult::pending(name),
        ServerSpec::Stdio {
            name,
            command,
            args,
            env,
            cwd,
            given: true,
        } => {
            let mut environment = given.0.environment.clone();
            environment.extend(env.iter().cloned());
            let mut transport =
                match StdioMcp::start(command, args, &environment, cwd.as_deref(), settings) {
                    Ok(transport) => transport,
                    Err(why) => return ServerResult::error(name, why),
                };
            let result = collect_mcp(name, &mut transport, settings);
            transport.stop();
            result
        }
    }
}

fn collect_mcp<T: McpRpc>(
    server_name: &str,
    rpc: &mut T,
    settings: &McpListConfig,
) -> ServerResult {
    let init_params = json!({
        "protocolVersion": MCP_VERSIONS[0],
        "capabilities": {},
        "clientInfo": {"name": "canoa-host", "version": env!("CARGO_PKG_VERSION")},
    });
    let init = match rpc.request(1, "initialize", init_params) {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            return ServerResult::error(
                server_name,
                format!("initialize failed ({}): {}", err.code, err.message),
            );
        }
        Err(why) => return ServerResult::error(server_name, why),
    };
    let _ = rpc.notify("notifications/initialized", json!({}));

    let caps = init.get("capabilities").and_then(Value::as_object);
    let has_tools_cap = caps.is_none_or(|c| c.contains_key("tools") || c.is_empty());
    let has_resources_cap = caps.is_some_and(|c| c.contains_key("resources"));

    let mut next_id = 2_u64;
    let tools = if has_tools_cap {
        match page_list(
            rpc,
            &mut next_id,
            "tools/list",
            "tools",
            (server_name, settings.max_items_per_server),
            format_tool,
        ) {
            Ok(list) => list,
            Err(why) => return ServerResult::error(server_name, why),
        }
    } else {
        Vec::new()
    };

    let resources = if has_resources_cap {
        match page_list(
            rpc,
            &mut next_id,
            "resources/list",
            "resources",
            (server_name, settings.max_items_per_server),
            format_resource,
        ) {
            Ok(list) => list,
            Err(why) => return ServerResult::error(server_name, why),
        }
    } else {
        Vec::new()
    };

    ServerResult::ok(server_name, tools, resources)
}

fn page_list<T: McpRpc>(
    rpc: &mut T,
    next_id: &mut u64,
    method: &str,
    field: &str,
    (server_name, max_items): (&str, usize),
    normalize: impl Fn(&str, &Value) -> Option<Value>,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let id = *next_id;
        *next_id = next_id.saturating_add(1);
        let params = match &cursor {
            Some(c) => json!({"cursor": c}),
            None => json!({}),
        };
        let result = match rpc.request(id, method, params)? {
            Ok(result) => result,
            Err(err) if err.code == -32601 => return Ok(Vec::new()),
            Err(err) => return Err(format!("{method} failed ({}): {}", err.code, err.message)),
        };
        let page = result
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{method} answer lacked {field} list"))?;
        for raw in page {
            if items.len() >= max_items {
                return Ok(items);
            }
            if let Some(entry) = normalize(server_name, raw) {
                items.push(entry);
            }
        }
        let next = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if next.is_none() || next == cursor {
            break;
        }
        cursor = next;
    }
    Ok(items)
}

fn format_tool(server: &str, raw: &Value) -> Option<Value> {
    let obj = raw.as_object()?;
    let name = obj.get("name").and_then(Value::as_str)?;
    let mut out = Map::new();
    out.insert("server".to_owned(), json!(server));
    out.insert("name".to_owned(), json!(name));
    if let Some(title) = obj.get("title").and_then(Value::as_str) {
        out.insert("title".to_owned(), json!(title));
    }
    if let Some(description) = obj.get("description").and_then(Value::as_str) {
        out.insert("description".to_owned(), json!(description));
    }
    if let Some(schema) = obj.get("inputSchema").filter(|s| s.is_object()) {
        out.insert("inputSchema".to_owned(), schema.clone());
    }
    Some(Value::Object(out))
}

fn format_resource(server: &str, raw: &Value) -> Option<Value> {
    let obj = raw.as_object()?;
    let uri = obj.get("uri").and_then(Value::as_str)?;
    let mut out = Map::new();
    out.insert("server".to_owned(), json!(server));
    out.insert("uri".to_owned(), json!(uri));
    let name = obj.get("name").and_then(Value::as_str).unwrap_or(uri);
    out.insert("name".to_owned(), json!(name));
    if let Some(title) = obj.get("title").and_then(Value::as_str) {
        out.insert("title".to_owned(), json!(title));
    }
    if let Some(description) = obj.get("description").and_then(Value::as_str) {
        out.insert("description".to_owned(), json!(description));
    }
    if let Some(mime) = obj.get("mimeType").and_then(Value::as_str) {
        out.insert("mimeType".to_owned(), json!(mime));
    }
    Some(Value::Object(out))
}

struct HttpMcp {
    agent: ureq::Agent,
    deadline: Instant,
    max_reply: u64,
    url: String,
    headers: Vec<(String, String)>,
    session_header: Option<String>,
}

impl HttpMcp {
    fn new(
        url: &str,
        headers: &[(String, String)],
        settings: &McpListConfig,
    ) -> Result<Self, String> {
        let query = Duration::from_secs(settings.query_seconds);
        let parsed = url::Url::parse(url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
            .ok_or_else(|| "MCP URL is not an http or https address".to_owned())?;
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(query))
            .build();
        let agent = Network::current()
            .agent(config)
            .map_err(|error| format!("network unavailable: {error}"))?;
        Ok(Self {
            agent,
            deadline: Instant::now() + query,
            max_reply: u64::try_from(settings.max_reply_bytes).unwrap_or(u64::MAX),
            url: parsed.to_string(),
            headers: headers.to_vec(),
            session_header: None,
        })
    }

    fn send_json(&mut self, body: &Value) -> Result<(u16, String, Vec<u8>), String> {
        if self.deadline <= Instant::now() {
            return Err("server did not answer in time".to_owned());
        }
        let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
        let mut req = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(session_id) = &self.session_header {
            req = req.header("Mcp-Session-Id", session_id);
        }
        for (name, value) in &self.headers {
            if name.eq_ignore_ascii_case("content-type")
                || name.eq_ignore_ascii_case("accept")
                || name.eq_ignore_ascii_case("host")
                || name.eq_ignore_ascii_case("content-length")
            {
                continue;
            }
            req = req.header(name.as_str(), value.as_str());
        }
        let mut response = req
            .send(&bytes[..])
            .map_err(|error| format!("HTTP MCP request failed: {error}"))?;
        let status = response.status().as_u16();
        if let Some(sid) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            self.session_header = Some(sid.to_owned());
        }
        let ctype = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut out = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(self.max_reply.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|error| format!("HTTP MCP response did not finish arriving: {error}"))?;
        if out.len() as u64 > self.max_reply {
            return Err("HTTP MCP response is over [host.mcp_list] max_reply_bytes".to_owned());
        }
        Ok((status, ctype, out))
    }
}

impl McpRpc for HttpMcp {
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let _ = self.send_json(&msg)?;
        Ok(())
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, RpcError>, String> {
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let (status, ctype, raw) = self.send_json(&body)?;
        if !(200..300).contains(&status) {
            return Err(format!("server answered HTTP {status}"));
        }
        let parsed = if ctype.contains("text/event-stream") {
            parse_sse_response(&raw, id)?
        } else {
            serde_json::from_slice::<Value>(&raw)
                .map_err(|_| "server answer is not JSON".to_owned())?
        };
        extract_rpc_outcome(parsed)
    }
}

fn parse_sse_response(raw: &[u8], expected_id: u64) -> Result<Value, String> {
    let text = core::str::from_utf8(raw).map_err(|_| "SSE answer is not UTF-8".to_owned())?;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(payload) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(payload)
            && value.get("method").is_none()
            && value.get("id").and_then(Value::as_u64) == Some(expected_id)
        {
            return Ok(value);
        }
    }
    Err("SSE answer lacked JSON-RPC response".to_owned())
}

fn extract_rpc_outcome(value: Value) -> Result<Result<Value, RpcError>, String> {
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown MCP error")
            .to_owned();
        return Ok(Err(RpcError { code, message }));
    }
    match value.get("result") {
        Some(result) => Ok(Ok(result.clone())),
        None => Err("MCP answer lacked result or error".to_owned()),
    }
}

struct StdioMcp {
    child: crate::spawn::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: mpsc::Receiver<Result<String, String>>,
    stderr_rx: mpsc::Receiver<String>,
    deadline: Instant,
}

impl StdioMcp {
    fn start(
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: Option<&str>,
        settings: &McpListConfig,
    ) -> Result<Self, String> {
        let max_line = settings.max_reply_bytes;
        let mut cmd = Command::new(command);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        cmd.args(args)
            .stdin(Stdio::Piped)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("could not start server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "could not open server stdin".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "could not open server stdout".to_owned())?;
        let stderr = child.stderr.take();

        let (line_tx, lines) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("canoa-mcp-stdout".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut buf = Vec::new();
                    let limit = u64::try_from(max_line)
                        .unwrap_or(u64::MAX)
                        .saturating_add(1);
                    match (&mut reader).take(limit).read_until(b'\n', &mut buf) {
                        Ok(0) => break,
                        Ok(_) if buf.len() > max_line => {
                            let _ = line_tx.send(Err(
                                "server output line is over [host.mcp_list] max_reply_bytes"
                                    .to_owned(),
                            ));
                            break;
                        }
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf).trim().to_owned();
                            if !line.is_empty() && line_tx.send(Ok(line)).is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            let _ = line_tx.send(Err(format!("read error: {err}")));
                            break;
                        }
                    }
                }
            });

        let (err_tx, stderr_rx) = mpsc::channel();
        if let Some(mut stderr) = stderr {
            let _ = thread::Builder::new()
                .name("canoa-mcp-stderr".to_owned())
                .spawn(move || {
                    let mut buf = Vec::new();
                    let _ = (&mut stderr).take(1024).read_to_end(&mut buf);
                    let text = String::from_utf8_lossy(&buf).trim().to_owned();
                    let _ = err_tx.send(text);
                });
        } else {
            let _ = err_tx.send(String::new());
        }

        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            stderr_rx,
            deadline: Instant::now() + Duration::from_secs(settings.query_seconds),
        })
    }

    fn stderr_hint(&self) -> String {
        self.stderr_rx
            .recv_timeout(Duration::from_millis(100))
            .ok()
            .filter(|s| !s.is_empty())
            .map_or_else(String::new, |s| format!(": {s}"))
    }

    fn stop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for StdioMcp {
    fn drop(&mut self) {
        self.stop();
    }
}

impl McpRpc for StdioMcp {
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err("server stdin closed".to_owned());
        };
        let mut line = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("could not write to server: {error}"))
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, RpcError>, String> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err("server stdin closed".to_owned());
        };
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("could not write to server: {error}{}", self.stderr_hint()))?;

        loop {
            let left = self.deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("server did not answer {method} in time"));
            }
            match self.lines.recv_timeout(left) {
                Ok(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if value.get("method").is_none()
                        && value.get("id").and_then(Value::as_u64) == Some(id)
                    {
                        return extract_rpc_outcome(value);
                    }
                }
                Ok(Err(why)) => return Err(why),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!("server did not answer {method} in time"));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "server exited before answering {method}{}",
                        self.stderr_hint()
                    ));
                }
            }
        }
    }
}

/// Outermost session channel wrapper that records attached MCP servers and answers [`METHOD`].
pub(crate) struct Layer<'a, C: Channel + ?Sized> {
    inner: &'a C,
    inventory: Arc<Inventory>,
}

impl<'a, C: Channel + ?Sized> Layer<'a, C> {
    pub(crate) fn new(inner: &'a C, settings: McpListConfig, given: Given) -> Self {
        Self {
            inner,
            inventory: Inventory::new(settings, given),
        }
    }

    fn flush(&self) -> Result<(), Failure> {
        for message in self.inventory.drain() {
            crate::scope::host_own(|| self.inner.send(&message))?;
        }
        Ok(())
    }
}

impl<C: Channel + ?Sized> Drop for Layer<'_, C> {
    fn drop(&mut self) {
        self.inventory.close();
    }
}

impl<C: Channel + ?Sized> Channel for Layer<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inventory.agent_said(message);
        self.inner.send(message)?;
        self.flush()
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let until = timeout.map(|timeout| Instant::now() + timeout);
        loop {
            self.flush()?;
            let slice = match until {
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(Failure::Timeout);
                    }
                    left.min(POLL)
                }
                None => POLL,
            };
            match self.inner.receive(Some(slice)) {
                Ok(Some(message)) => {
                    if let Some(forward) = self.inventory.client_said(message) {
                        return Ok(Some(forward));
                    }
                }
                Ok(None) => return Ok(None),
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(failure),
            }
        }
    }

    fn close(&self) {
        self.inventory.close();
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_servers_converts_bridge_stdio_to_http_and_marks_unsupported() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {
                "mcpServers": [
                    {
                        "type": "stdio",
                        "name": "canoa-store",
                        "command": "canoa",
                        "args": ["mcp-bridge"],
                        "env": [
                            {"name": "CANOA_DELEGATE_URL", "value": "http://127.0.0.1:1234/mcp"},
                            {"name": "CANOA_DELEGATE_TOKEN", "value": "secret-tok"}
                        ]
                    },
                    {"type": "stdio", "name": "probe", "command": "/bin/probe", "args": ["--x"]},
                    {"type": "sse", "name": "events", "url": "https://example.test/sse"}
                ]
            }
        });
        let given = Given::new(Vec::new());
        given.add(&[json!({"name": "probe"})]);
        let parsed = extract_servers(&req, &given);
        assert_eq!(parsed.len(), 3);
        assert!(matches!(
            &parsed[0],
            ServerSpec::Http { name, url, .. } if name == "canoa-store" && url == "http://127.0.0.1:1234/mcp"
        ));
        assert!(matches!(
            &parsed[1],
            ServerSpec::Stdio { name, command, given: true, .. } if name == "probe" && command == "/bin/probe"
        ));
        assert!(matches!(
            &parsed[2],
            ServerSpec::Unsupported { name, .. } if name == "events"
        ));
    }

    #[test]
    fn a_stdio_server_the_agent_starts_is_pending_and_not_started() {
        let spec = ServerSpec::Stdio {
            name: "theirs".to_owned(),
            command: "/no/such/program".to_owned(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            given: false,
        };
        let outcome = query_server(&spec, &McpListConfig::default(), &Given::new(Vec::new()));
        assert_eq!(outcome.summary["status"], "pending");
        assert!(outcome.summary.get("error").is_none());
    }

    #[test]
    fn a_page_is_the_limit_asked_for_with_no_ceiling() {
        assert_eq!(parse_limit(None), Ok(DEFAULT_LIMIT));
        assert_eq!(parse_limit(Some(&json!(100_000))), Ok(100_000));
        assert!(parse_limit(Some(&json!(0))).is_err());
    }

    #[test]
    fn slice_snapshot_pages_across_tools_then_resources_and_stops_cleanly() {
        let snap = Snapshot {
            generation: 7,
            server_filter: None,
            tools: vec![
                json!({"server": "a", "name": "t1"}),
                json!({"server": "a", "name": "t2"}),
            ],
            resources: vec![
                json!({"server": "a", "uri": "file:///1", "name": "r1"}),
                json!({"server": "a", "uri": "file:///2", "name": "r2"}),
            ],
            servers: vec![json!({"name": "a", "status": "ok", "tools": 2, "resources": 2})],
        };
        let p1 = slice_snapshot(&snap, 0, 3);
        assert_eq!(
            p1["tools"],
            json!([{"server": "a", "name": "t1"}, {"server": "a", "name": "t2"}])
        );
        assert_eq!(
            p1["resources"],
            json!([{"server": "a", "uri": "file:///1", "name": "r1"}])
        );
        assert_eq!(p1["nextCursor"], json!("7:3"));

        let p2 = slice_snapshot(&snap, 3, 3);
        assert_eq!(p2["tools"], json!([]));
        assert_eq!(
            p2["resources"],
            json!([{"server": "a", "uri": "file:///2", "name": "r2"}])
        );
        assert!(p2.get("nextCursor").is_none());
    }

    #[test]
    fn parse_sse_response_picks_matching_json_rpc_id() {
        let sse = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n\n";
        let parsed = parse_sse_response(sse, 2).expect("sse parsed");
        assert_eq!(parsed["id"], 2);
        assert_eq!(parsed["result"], json!({"tools": []}));
    }
}
