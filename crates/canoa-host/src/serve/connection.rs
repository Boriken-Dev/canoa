//! Accept loops, key proof handshake, and routing proven connections to agents, harnesses, providers, and tools.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::sync::atomic::Ordering;
use std::ffi::OsString;
use std::thread;

use coqui_builder::builder::BuildError;
use coqui_link::auth::{self, Sealed};
use coqui_link::link::Prefilled;
use coqui_link::mux;
use coqui_peer_webcrypto::PeerId;
use coqui_transport::{Channel, Failure};
use coqui_transport_web::Limits;
use coqui_transport_web::websocket::{self, Frames};

use super::{
    Agent, Entered, FIRST_MESSAGE, MAX_WORKTREE_REQUESTS, PROOF_LIMIT, Process, Run, STOP_POLL,
    Served, Shared, UNPROVEN_PENDING,
};
use crate::accounts::{Gated, Held};
use crate::child;
use crate::client_tools::{self, ToolsLayer, ToolsPeek};
use crate::control;
use crate::delegate::{self, Delegating, Peek, Principal};
use crate::history::Recording;
use crate::inproc::{self, InProcess};
use crate::kb_mcp::{self, KbLayer};
use crate::kind;
use crate::mcp_list::{Given, Layer as McpListLayer};
use crate::pipe;
use crate::profile::hosted::{self, Connection, Hosted, Offer, Starting};
use crate::profile::{Session, Who, account_words, plain_launch_line};
use crate::provider::HostLine;
use crate::scope::{Scope, Scoped};
use crate::secrets::{self, Layer, Raw};
use crate::store::mcp::{self as store_mcp, StoreLayer, StorePeek};
use crate::transport;
use crate::worktree::Worktrees;

pub(super) fn url_of(bound: SocketAddr) -> String {
    format!("ws://{bound}")
}

pub(super) fn unproven_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_message = auth::MAX_PROOF_MESSAGE;
    limits.pending = UNPROVEN_PENDING;
    limits
}

/// A plain WebSocket listener of binary frames on `address`, for peers that prove themselves in-band.
///
/// Where the socket sits is the person's to choose, so a non-loopback address binds; the caller reports it through
/// [`websocket::Listener::plain_beyond_loopback`].
pub(crate) fn plain_listener(
    address: SocketAddr,
    limits: Limits,
    origins: Vec<String>,
) -> Result<websocket::Listener, Failure> {
    websocket::Listener::builder(address, websocket::ServerTls::Plain)
        .limits(limits)
        .frames(Frames::Binary)
        .origins(origins)
        .plain_beyond_loopback(true)
        .build()
        .map_err(|error| match error {
            BuildError::Failed(failure) => failure,
            // The address and the TLS choice are both given above; no other kind of error exists.
            _ => Failure::Unsupported,
        })
}

pub(super) fn accept_loop(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    listener: &websocket::Listener,
) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let pending = match listener.next(Some(STOP_POLL)) {
            Ok(pending) => pending,
            Err(Failure::Timeout) => continue,
            Err(failure) => {
                tracing::warn!(entry = served.name, "a connection failed: {failure}");
                continue;
            }
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || match pending.handshake() {
                Ok(channel) => {
                    connection(&shared, &served, &channel, entered, None, None);
                    channel.close();
                }
                Err(failure) => tracing::debug!("a handshake failed: {failure}"),
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
}

/// [`accept_loop`] for a transport provider: what it hands over is already a connection, so the proof runs on it at once.
pub(super) fn accept_transport_loop(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    listening: &transport::Listening,
) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let (from, link) = match listening.next(STOP_POLL) {
            transport::Next::Nobody => continue,
            transport::Next::Ended(status, reason) => {
                tracing::warn!(
                    entry = served.name,
                    status,
                    "its transport stopped listening: {reason}"
                );
                return;
            }
            transport::Next::Peer(from, link) => (from, link),
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        tracing::debug!(entry = served.name, from, "a connection on a transport");
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || {
                connection(&shared, &served, &link, entered, None, None);
                link.close();
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
    listening.close();
}

/// [`accept_loop`] for a pipe or Unix socket: there is no handshake to run before the proof.
pub(super) fn accept_pipe_loop(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    listener: &pipe::Listener,
) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let stream = match listener.accept_timeout(Some(STOP_POLL)) {
            Ok(stream) => stream,
            Err(pipe::Error::TimedOut) => continue,
            Err(error) => {
                tracing::warn!(entry = served.name, "a connection failed: {error}");
                thread::sleep(STOP_POLL);
                continue;
            }
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        if let Some(uid) = shared.portal_uid
            && !crate::portal_link::from_user(&stream, uid)
        {
            tracing::warn!(
                entry = served.name,
                "a connection from a user other than the portal's was refused"
            );
            continue;
        }
        match stream.peer() {
            Ok(peer) => tracing::debug!(entry = served.name, ?peer, "a connection on a pipe"),
            Err(error) => tracing::debug!(entry = served.name, "no peer identity: {error}"),
        }
        let channel = pipe::PipeChannel::new(stream, auth::MAX_PROOF_MESSAGE);
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || {
                connection(&shared, &served, &channel, entered, None, None);
                channel.close();
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
}

/// A connection's proof and what follows it. `admit`, when given, is asked once the key is known and may turn it away: the native
/// door uses it to bind a browser's key to the extension that paired it.
pub(super) fn connection<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    channel: &C,
    unproven: Entered,
    admit: Option<&dyn Fn(&PeerId) -> bool>,
    extension: Option<&dyn Fn(&PeerId) -> bool>,
) {
    let established = match auth::prove(
        &shared.method,
        channel,
        PROOF_LIMIT,
        shared.method.method().record_length(shared.max_line),
    ) {
        Ok(established) => established,
        Err(error @ coqui_link::Error::OtherMethod { .. }) => {
            return tracing::info!(entry = served.name, "a client was refused: {error}");
        }
        Err(error @ coqui_link::Error::Proof(_)) => {
            return tracing::info!(
                entry = served.name,
                "a key was refused: {error}; a key that proved itself and is not paired waits for `canoa accept`"
            );
        }
        Err(error) => {
            return tracing::info!(
                entry = served.name,
                "a connection ended before its proof: {error}"
            );
        }
    };
    drop(unproven);
    let peer = established.peer;
    if !shared.allows(&peer, served) {
        return tracing::info!(entry = served.name, %peer, "a paired key this entry does not list was turned away");
    }
    if let Some(admit) = admit
        && !admit(&peer)
    {
        return tracing::info!(entry = served.name, %peer, "a browser's key was turned away: it was paired by another extension");
    }
    let store = Arc::clone(&shared.store);
    let stopped = Arc::clone(&shared.stopped);
    let checked = peer;
    let sealed = Sealed::new(channel, established.session)
        .with_check(move || !stopped.load(Ordering::SeqCst) && store.is_accepted(&checked));
    let reach = if extension.is_some_and(|bound| bound(&peer)) {
        Reach::Extension
    } else {
        Reach::Remote
    };
    serve_proven(shared, served, &sealed, &peer, None, reach);
    sealed.close();
}

/// How a connection reached the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// A key that proved itself over a listener.
    Remote,
    /// A key the browser extension's PIN pairing bound to its origin, through the native door.
    Extension,
    /// The application in this process.
    InProcess,
}

impl Reach {
    pub(crate) const fn in_process(self) -> bool {
        matches!(self, Self::InProcess)
    }
}

/// Serves a connection whose peer is known, from its first message to its end; the caller closes `channel` after. `reach` is
/// [`Reach::InProcess`] only for the connections [`super::Local`] opens, whose peer is this process.
pub(super) fn serve_proven<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    sealed: &C,
    peer: &PeerId,
    context: Option<&delegate::ChildContext>,
    reach: Reach,
) {
    let Some(_running) = served.running.enter() else {
        tracing::info!(entry = served.name, %peer, "turned away: max_peers are running");
        return;
    };
    let mut first = sealed.receive(Some(FIRST_MESSAGE));
    if let Run::Agent { worktrees, .. } = &served.run {
        let mut answered = 0;
        while let Ok(Some(message)) = &first
            && Worktrees::is_request(message)
        {
            if answered == MAX_WORKTREE_REQUESTS {
                tracing::info!(entry = served.name, %peer, "turned away: too many worktree requests");
                return;
            }
            answered += 1;
            if let Some(reply) = worktrees.answer(message)
                && sealed.send(&reply).is_err()
            {
                return;
            }
            first = sealed.receive(Some(FIRST_MESSAGE));
        }
    }
    match first {
        Ok(Some(first)) if mux::starts_mux(&first) => {
            match shared.delegation.as_ref().filter(|_| served.delegable) {
                Some(delegation) => {
                    tracing::info!(entry = served.name, %peer, "a link of streams");
                    delegation.serve_link(sealed, first, &peer.to_string(), shared.max_line);
                }
                None => {
                    tracing::info!(entry = served.name, %peer, "a link of streams was refused: this entry serves plain ACP");
                }
            }
        }
        Ok(Some(first)) => {
            tracing::info!(entry = served.name, %peer, "proven");
            let copy = first.clone();
            give_agent(
                shared,
                served,
                &Prefilled::new(&super::limit::Naming::new(sealed, shared.max_line), first),
                (&copy, peer),
                (context, reach),
            );
        }
        Ok(None) | Err(_) => {}
    }
}

fn give_agent<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    (context, reach): (Option<&delegate::ChildContext>, Reach),
) {
    if let (Some(keep), Run::Agent { .. }, None) = (&served.keep, &served.run, context) {
        return super::sessions::serve(shared, served, keep, channel, (first, peer), reach);
    }
    give_agent_flow(shared, served, channel, (first, peer), (context, reach));
}

fn dispatch_non_agent<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    peer: &PeerId,
    in_process: bool,
) -> bool {
    match &served.run {
        Run::Provider {
            program,
            args,
            line,
        } => {
            give_provider(shared, served, channel, peer, (program, args), line);
            true
        }
        Run::Control { grants, everyone } => {
            let grants = if let Some(everyone) = everyone {
                everyone.clone()
            } else {
                let mut own: BTreeSet<control::Grant> = grants
                    .iter()
                    .find(|(key, _)| *key == peer)
                    .map(|(_, grants)| grants.clone())
                    .unwrap_or_default();
                if shared.is_admitted(peer) {
                    own.extend(shared.page_grants.iter().copied());
                }
                own
            };
            let caller = control::Caller::Peer {
                id: peer.to_string(),
                grants,
            };
            control::serve(&shared.control, &caller, channel);
            true
        }
        Run::Tools { grants } => {
            let grants = if in_process {
                control::Grant::all().into_iter().collect()
            } else {
                grants.clone()
            };
            shared
                .client_tools
                .provide(&peer.to_string(), &grants, channel);
            true
        }
        Run::Agent { .. } => false,
    }
}

fn attach_connection_tools(
    shared: &Shared,
    first: &[u8],
    connection: Option<Connection>,
    store: Option<&store_mcp::Link>,
    conn: Option<&Arc<delegate::Conn>>,
) -> (Option<Connection>, Option<Arc<kb_mcp::Link>>) {
    let connection = connection.map(|connection| {
        if store.is_some_and(store_mcp::Link::serves_store) && store_mcp::asked_in(first) {
            connection.with_skill(store_mcp::SKILL_NAME, store_mcp::skill_files())
        } else {
            connection
        }
    });
    let connection = connection.map(|connection| {
        if store.is_some_and(|link| link.offers(crate::mail::tools::SERVER_NAME)) {
            connection.with_skill(
                crate::mail::tools::SKILL_NAME,
                crate::mail::tools::skill_files(),
            )
        } else {
            connection
        }
    });
    let kb = connection.as_ref().and_then(|connection| {
        shared.kb_tools.for_connection(
            connection.kb_sources(),
            &connection.kb_caller(),
            connection.network(),
            kb_mcp::asked_in(first),
        )
    });
    let connection = connection.map(|connection| {
        if kb.is_some() {
            connection
                .with_kb_tools()
                .with_skill(kb_mcp::SKILL_NAME, kb_mcp::skill_files())
        } else {
            connection
        }
    });
    if let (Some(conn), Some(connection)) = (conn, &connection) {
        conn.suggest_with(connection.delegate_hints());
    }
    (connection, kb)
}

struct ConnectionLinks<'a> {
    conn: Option<&'a Arc<delegate::Conn>>,
    link: Option<&'a Arc<secrets::Link>>,
    store: Option<&'a Arc<store_mcp::Link>>,
    kb: Option<&'a Arc<kb_mcp::Link>>,
    tools: Option<&'a Arc<client_tools::Link>>,
}

#[allow(clippy::too_many_arguments)]
fn launch_agent_program<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    peer: &PeerId,
    (program, args): (&OsString, &[OsString]),
    scope: &Scope,
    offer: Option<&Offer>,
    connection: Option<&Connection>,
    identity: &[(String, String)],
    links: ConnectionLinks<'_>,
) {
    let ConnectionLinks {
        conn,
        link,
        store,
        kb,
        tools,
    } = links;
    let launcher = offer.map_or(&shared.launcher, |offer| &offer.launcher);
    let mut launched = shared.program_args.clone();
    launched.extend([
        OsString::from("launch"),
        OsString::from("--"),
        program.clone(),
    ]);
    launched.extend(args.iter().cloned());
    let (key, agent) = match shared.start(launcher, &launched) {
        Ok(Some(started)) => started,
        Ok(None) => return,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the agent could not start: {error}");
        }
    };
    let end = match connection {
        Some(connection) => {
            let given = Given::for_connection(Some(connection));
            let hosted = Hosted::new(channel, scope, connection)
                .agent(agent.pid())
                .given(given.clone());
            let end = coqui_acp_bridge::relay::relay(
                &McpListLayer::new(&crate::awake::AwakeChannel::new(&ToolsLayer::new(
                    &KbLayer::new(
                        &StoreLayer::new(&Layer::new(&Delegating::new(&hosted, conn), link), store),
                        kb,
                    ),
                    tools,
                ), Arc::clone(&shared.keep_awake)), shared.mcp_list, given),
                &agent,
            );
            drop(hosted);
            end
        }
        None => {
            let scoped = Scoped::new(channel, scope)
                .agent(agent.pid())
                .launching(plain_launch_line(identity));
            coqui_acp_bridge::relay::relay(
                &McpListLayer::new(&crate::awake::AwakeChannel::new(&ToolsLayer::new(
                    &KbLayer::new(
                        &StoreLayer::new(&Layer::new(&Delegating::new(&scoped, conn), link), store),
                        kb,
                    ),
                    tools,
                ), Arc::clone(&shared.keep_awake)), shared.mcp_list, Given::for_connection(None)),
                &agent,
            )
        }
    };
    tracing::info!(entry = served.name, %peer, "left: {end:?}");
    shared.forget(key);
    let _ = agent.stop();
}

/// Serves one connection with an agent of its own, until either ends.
pub(super) fn give_agent_flow<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    (context, reach): (Option<&delegate::ChildContext>, Reach),
) {
    if dispatch_non_agent(shared, served, channel, peer, reach.in_process()) {
        return;
    }
    let Run::Agent {
        agent,
        scope,
        offer,
        refresh,
        ..
    } = &served.run
    else {
        return;
    };
    let any_account = shared
        .store_grants(peer, context.is_some())
        .is_some_and(|grants| grants.contains(&control::Grant::Accounts));
    let settled;
    let (first, pool) = match agent {
        Agent::Harness(harness) => match harness.settle(first, any_account) {
            Ok(done) => {
                settled = done;
                (settled.first.as_slice(), settled.pool.as_deref())
            }
            Err(why) => return turn_away(channel, first, (&served.name, peer), why),
        },
        Agent::Program { .. } => (first, None),
    };
    let child_scope;
    let scope = match context.and_then(|context| context.parent_session.clone()) {
        Some(parent) => {
            child_scope = scope.child_of(Some(parent));
            &child_scope
        }
        None => scope,
    };
    let conn = shared
        .delegation
        .as_ref()
        .filter(|_| served.delegable)
        .map(|delegation| {
            delegation.connection(match context {
                Some(context) => delegate::ConnInfo {
                    principal: context.principal.clone(),
                    entry: served.name.clone(),
                    roots: context.roots.clone(),
                    depth: context.depth,
                    child: true,
                },
                None => delegate::ConnInfo {
                    principal: if *peer == shared.method.peer_id() {
                        Principal::Owner
                    } else {
                        Principal::Key(*peer)
                    },
                    entry: served.name.clone(),
                    roots: scope.roots().to_vec(),
                    depth: 0,
                    child: false,
                },
            })
        });
    let link = shared
        .secrets
        .for_connection(peer, reach.in_process(), context.is_some());
    let store = shared
        .store_tools
        .for_connection(shared.store_grants(peer, context.is_some()).as_ref());
    let peeked = Peek::new(channel, conn.as_ref());
    let raw = Raw::new(&peeked, link.as_ref());
    let store_peek = StorePeek::new(&raw, store.as_ref());
    let tools = shared.client_tools.for_connection(
        context.is_some(),
        client_tools::Who {
            agent: served.name.clone(),
            via: if reach.in_process() {
                client_tools::Via::App
            } else {
                client_tools::Via::Paired(peer.to_string())
            },
            extension: reach == Reach::Extension,
        },
    );
    let tools_peek = ToolsPeek::new(&store_peek, tools.as_ref());
    let channel = &tools_peek;
    let fresh = refresh
        .as_ref()
        .map_or_else(|| offer.clone(), |refresh| refresh.offer());
    let offer = fresh.as_ref();
    let account = match &agent {
        Agent::Harness(harness) => harness.account_facts(first, any_account),
        Agent::Program { .. } => None,
    };
    let provider = match &agent {
        Agent::Harness(harness) => Some(harness.id()),
        Agent::Program { .. } => None,
    };
    let harness_id = hosted::harness_identity(provider, &served.name);
    let makers = match &agent {
        Agent::Harness(harness) => Arc::clone(harness.makers()),
        Agent::Program { .. } => Arc::default(),
    };
    let (account_kind, account_provider) = account
        .as_ref()
        .map_or((None, None), |(_, kind, provides)| {
            account_words(kind, provides, harness_id, &makers)
        });
    let version = hosted::harness_version(provider);
    let who = Who {
        harness: harness_id,
        version: version.as_deref(),
        account: account.as_ref().map(|(label, ..)| label.as_str()),
        account_kind,
        account_provider,
        role: context.is_some().then_some("delegate"),
        ..Who::default()
    };
    let connection = match offer {
        Some(offer) => match prepare(
            served,
            channel,
            first,
            peer,
            offer,
            *peer == shared.method.peer_id(),
            Starting {
                who,
                host: hosted::host_variables(provider),
                account: match &agent {
                    Agent::Harness(harness) => harness.account_environment(first, any_account),
                    Agent::Program { .. } => Vec::new(),
                },
            },
        ) {
            Some(connection) => Some(connection),
            None => return,
        },
        None => None,
    };
    let identity: Vec<(String, String)> = if offer.is_none() {
        Session::new(who, Vec::new(), &|name| std::env::var(name).ok())
            .identity()
            .to_vec()
    } else {
        Vec::new()
    };
    let (connection, kb) =
        attach_connection_tools(shared, first, connection, store.as_deref(), conn.as_ref());
    match agent {
        Agent::Program { program, args } => launch_agent_program(
            shared,
            served,
            channel,
            peer,
            (program, args),
            scope,
            offer.map(Arc::as_ref),
            connection.as_ref(),
            &identity,
            ConnectionLinks {
                conn: conn.as_ref(),
                link: link.as_ref(),
                store: store.as_ref(),
                kb: kb.as_ref(),
                tools: tools.as_ref(),
            },
        ),
        Agent::Harness(options) => give_harness(
            shared,
            served,
            channel,
            (peer, first, any_account, pool),
            options,
            scope,
            (
                connection.as_ref(),
                conn.as_ref(),
                link.as_ref(),
                store.as_ref(),
                kb.as_ref(),
                tools.as_ref(),
                &identity,
            ),
        ),
    }
}

/// The profile the client chose in its first message, ready for its agent; `None` once it was refused, answered and logged.
fn prepare<C: Channel>(
    served: &Served,
    channel: &C,
    first: &[u8],
    peer: &PeerId,
    offer: &Offer,
    in_process: bool,
    starting: Starting<'_>,
) -> Option<Connection> {
    let refuse = |answer: Option<Vec<u8>>, log: &str| {
        if let Some(answer) = answer {
            let _ = channel.send(&answer);
        }
        tracing::info!(entry = served.name, %peer, "{log}");
    };
    let index = match hosted::choose(offer, first) {
        Ok(index) => index,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let only = match hosted::choose_overlays(first) {
        Ok(only) => only,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let loadout = match hosted::choose_loadout(offer, index, first) {
        Ok(loadout) => loadout,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let preset = match hosted::choose_preset(offer, index, first) {
        Ok(preset) => preset,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let client = match hosted::choose_client(first) {
        Ok(client) => client,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let lookup = |name: &str| std::env::var(name).ok();
    match Connection::prepare_with(
        offer,
        index,
        &served.name,
        &lookup,
        hosted::Choices {
            only: only.as_deref(),
            loadout: loadout.as_deref(),
            preset: preset.as_deref(),
            client: client.as_deref(),
        },
        starting,
    ) {
        Ok(connection) => Some(connection.asking(in_process)),
        Err(why) => {
            let id = serde_json::from_slice::<serde_json::Value>(first)
                .ok()
                .and_then(|message| message.get("id").cloned());
            let answer = id.map(|id| {
                serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32603, "message": "the host's profile for this entry cannot be applied",
                }}))
                .unwrap_or_default()
            });
            refuse(
                answer,
                &format!("the profile could not be applied, so no agent started: {why}"),
            );
            None
        }
    }
}

/// What a connection's sessions are given beside the agent: its profile, and the delegation, secret and store tools.
type Joined<'a> = (
    Option<&'a Connection>,
    Option<&'a Arc<delegate::Conn>>,
    Option<&'a Arc<secrets::Link>>,
    Option<&'a Arc<store_mcp::Link>>,
    Option<&'a Arc<kb_mcp::Link>>,
    Option<&'a Arc<client_tools::Link>>,
    &'a [(String, String)],
);

/// Answers the `initialize` in `first` with `-32602` and `why`, and says in the log that the connection was turned away.
fn turn_away<C: Channel>(channel: &C, first: &[u8], (entry, peer): (&str, &PeerId), why: String) {
    let id = serde_json::from_slice::<serde_json::Value>(first)
        .ok()
        .and_then(|message| message.get("id").cloned());
    if let Some(id) = id {
        let answer = serde_json::json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": crate::control::BAD_PARAMS, "message": why}});
        let _ = channel.send(answer.to_string().as_bytes());
    }
    tracing::info!(entry, %peer, "turned away: {why}");
}

/// Serves one connection with an `acp_harness` provider in this process, which [`super::Stopper::stop_agents`] ends like any agent.
fn give_harness<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (peer, first, any_account, pool): (&PeerId, &[u8], bool, Option<&str>),
    harness: &kind::Harness,
    scope: &Scope,
    (connection, conn, link, store, kb, tools, identity): Joined<'_>,
) {
    let turn_away = |why: String| turn_away(channel, first, (&served.name, peer), why);
    let lifetime = match harness.lifetime_of(first) {
        Ok(lifetime) => lifetime,
        Err(why) => return turn_away(why),
    };
    let account = match harness.choose(
        first,
        &format!("a connection to {}", served.name),
        any_account,
    ) {
        Ok(account) => account,
        Err(why) => return turn_away(why),
    };
    let control = InProcess::new();
    let key = match shared.record(|| Ok((Process::Translator(Arc::clone(&control)), ()))) {
        Ok(Some((key, ()))) => key,
        Ok(None) | Err(_) => return,
    };
    let started = harness.connection_as(
        connection.map(|connection| {
            let environment = connection
                .env()
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().to_owned()))
                .collect();
            (environment, connection.skills())
        }),
        identity,
        account.as_ref(),
        lifetime,
    );
    let gated = Gated::new(
        channel,
        account.as_ref().map(|held| {
            let mut report = held.report();
            if let (Some(pool), Some(map)) = (pool, report.as_object_mut()) {
                map.insert("pool".to_owned(), serde_json::Value::from(pool));
            }
            report
        }),
        account.as_ref().is_some_and(Held::rotates),
    );
    let accounted;
    let scope = match &account {
        Some(held) => {
            accounted = scope.clone().account(held.usage_id());
            &accounted
        }
        None => scope,
    };
    // One program per client connection, which carries every session of that connection: a second line for the same peer
    // and entry means a client connected again, never that a prompt restarted the program.
    tracing::info!(entry = %served.name, harness = harness.id(), %peer, "starting the harness program: a client connected");
    let to_provider = (started.request, started.spawner);
    let given = Given::for_connection(connection);
    let end = match connection {
        Some(connection) => inproc::serve_kind(
            &McpListLayer::new(&crate::awake::AwakeChannel::new(&ToolsLayer::new(
                &KbLayer::new(
                    &StoreLayer::new(
                        &Layer::new(
                            &Delegating::new(
                                &Recording::new(
                                    &Hosted::in_process(&gated, scope, connection).given(given.clone()),
                                    &shared.history,
                                    &served.name,
                                    harness.id(),
                                ),
                                conn,
                            ),
                            link,
                        ),
                        store,
                    ),
                    kb,
                ),
                tools,
            ), Arc::clone(&shared.keep_awake)), shared.mcp_list, given),
            &*started.harness,
            to_provider,
            started.watch,
            shared.max_line,
            &control,
        ),
        None => inproc::serve_kind(
            &McpListLayer::new(&crate::awake::AwakeChannel::new(&ToolsLayer::new(
                &KbLayer::new(
                    &StoreLayer::new(
                        &Layer::new(
                            &Delegating::new(
                                &Recording::new(
                                    &Scoped::new(&gated, scope),
                                    &shared.history,
                                    &served.name,
                                    harness.id(),
                                ),
                                conn,
                            ),
                            link,
                        ),
                        store,
                    ),
                    kb,
                ),
                tools,
            ), Arc::clone(&shared.keep_awake)), shared.mcp_list, given),
            &*started.harness,
            to_provider,
            started.watch,
            shared.max_line,
            &control,
        ),
    };
    drop(account);
    tracing::info!(entry = served.name, %peer, "left: {end:?}");
    shared.forget(key);
}

/// Serves one connection with a provider process of its own, which [`super::Stopper::stop_agents`] ends like any agent.
fn give_provider<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    peer: &PeerId,
    (program, args): (&OsString, &[OsString]),
    line: &HostLine,
) {
    let mut prepared = match child::prepare(program, args) {
        Ok(prepared) => prepared,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the provider could not start: {error}");
        }
    };
    let started = shared.record(|| {
        let (running, streams) = prepared.start()?;
        let running = Arc::new(running);
        Ok((Process::Provider(Arc::clone(&running)), (running, streams)))
    });
    let (key, (running, streams)) = match started {
        Ok(Some(started)) => started,
        Ok(None) => return,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the provider could not start: {error}");
        }
    };
    let end = child::serve(
        &served.name,
        prepared,
        &running,
        streams,
        line,
        shared.max_line,
        &crate::scope::Screening::new(channel),
    );
    tracing::info!(entry = served.name, %peer, "left: {end}");
    shared.forget(key);
    running.stop();
}
