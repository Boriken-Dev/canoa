//! `host/keys`, `host/accept`, `host/revoke` and `host/pair`, and `[host]` through `config/get`, `config/set` and `config/check`:
//! what the app's Hosting page does on this computer, for a page that reaches the host over the network. All of it is behind the
//! grant `hosting`: pairing a key lets that key in, and `[host]` names the programs the host runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and locks are std's"
)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use coqui_agentic_peer::auth::Identity;
use coqui_peer_webcrypto::TrustStore;
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::{Caller, REFUSED, bad, error_line, text_param};
use crate::pair_run::{Door, Fail, Run};
use crate::peers::{self, PeerState, PeersError};

/// How long `host/pair` waits for its PIN to be used when the request names no time, as `canoa pair` does.
const DEFAULT_PAIR_SECONDS: u64 = 300;

/// Pairings under way on this host at once: each holds a listener, a PIN and a thread, so a key cannot start them without end.
const MAX_PAIRINGS: usize = 4;

/// How often the link is read while a pairing runs.
const POLL: Duration = Duration::from_millis(50);

type Outcome = Result<Value, (i32, String)>;

/// What `host/pair` shows a PIN with: the host's own identity, which the other side pins, and the home the extension's door and
/// the origins keys are bound to live in.
#[derive(Clone)]
pub(crate) struct Pairing {
    pub(crate) identity: Arc<Identity>,
    pub(crate) home: PathBuf,
}

impl core::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pairing").finish_non_exhaustive()
    }
}

/// The pairings under way.
#[derive(Debug, Default)]
pub(crate) struct Running(AtomicUsize);

fn peer_failure(error: &PeersError) -> (i32, String) {
    match error {
        PeersError::NotAnId => bad("id is a peer id"),
        PeersError::NotWaiting | PeersError::AlreadyAccepted => (REFUSED, error.to_string()),
        _ => (REFUSED, error.to_string()),
    }
}

/// `host/keys`: every key the trust store holds, paired first.
pub(crate) fn keys(store: &TrustStore) -> Outcome {
    let listed = peers::list(store).map_err(|error| peer_failure(&error))?;
    let keys: Vec<Value> = listed
        .iter()
        .map(|peer| match peer.state {
            PeerState::Accepted => json!({"id": peer.id.to_string(), "state": "accepted"}),
            PeerState::Waiting { source } => json!({
                "id": peer.id.to_string(),
                "state": "waiting",
                "source": source.map(|source| source.to_string()),
            }),
        })
        .collect();
    Ok(json!({ "keys": keys }))
}

/// `host/accept` `{id}`: pairs a waiting key. Only a person's choice calls this: nothing is accepted by itself.
pub(crate) fn accept(store: &TrustStore, params: &Value) -> Outcome {
    let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
    let accepted = peers::accept(store, id).map_err(|error| peer_failure(&error))?;
    Ok(json!({"accepted": accepted.to_string()}))
}

/// `host/revoke` `{id}`: removes a key, paired or waiting; a link it holds ends within about a second.
pub(crate) fn revoke(store: &TrustStore, params: &Value) -> Outcome {
    let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
    let removed = peers::revoke(store, id).map_err(|error| peer_failure(&error))?;
    Ok(json!({ "removed": removed }))
}

/// What a `host/pair` request asks for.
struct Asked {
    door: Door,
    timeout: Duration,
}

fn asked(params: &Value) -> Result<Asked, (i32, String)> {
    let native = match params.get("native") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(native)) => *native,
        Some(_) => return Err(bad("native is true or false")),
    };
    let timeout = match params.get("timeout_seconds") {
        None | Some(Value::Null) => DEFAULT_PAIR_SECONDS,
        Some(seconds) => seconds
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .ok_or_else(|| bad("timeout_seconds is a whole number above 0"))?,
    };
    let door = if native {
        if text_param(params, "listen")?.is_some() || text_param(params, "origin")?.is_some() {
            return Err(bad("a native pairing takes no listen or origin"));
        }
        Door::Native
    } else {
        let listen: SocketAddr = match text_param(params, "listen")? {
            Some(listen) => listen
                .parse()
                .map_err(|_| bad("listen is an address and port, such as 127.0.0.1:0"))?,
            None => SocketAddr::from(([127, 0, 0, 1], 0)),
        };
        Door::Network {
            listen,
            origin: text_param(params, "origin")?.map(str::to_owned),
        }
    };
    Ok(Asked {
        door,
        timeout: Duration::from_secs(timeout),
    })
}

/// A pairing's progress as the notification a link carries: the PIN and where to dial, naming the request it belongs to.
fn notification(request: &Value, shown: &Value) -> Vec<u8> {
    let mut params = shown.clone();
    params["id"] = request.get("id").cloned().unwrap_or(Value::Null);
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "host/progress", "params": params}))
        .unwrap_or_default()
}

enum Out {
    Shown(Value),
    Done(Result<String, Fail>),
}

/// `host/pair` `{native?, listen?, origin?, timeout_seconds?}` on `channel`: shows a PIN, announces it as a `host/progress`
/// notification (`pin`, `peer`, `method`, and `url`, or `native`), and answers `{paired}` with the key that used it. The link
/// closing ends the pairing with the PIN unused. Other requests on the link are answered as they come by `others`.
pub(crate) fn pair<C: Channel>(
    caller: &Caller,
    setup: Option<&Pairing>,
    store: Option<&Arc<TrustStore>>,
    running: &Running,
    line: &[u8],
    channel: &C,
    others: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
) -> Vec<u8> {
    let request: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let started = (|| {
        super::allow(caller, super::Grant::Hosting)?;
        let (Some(setup), Some(store)) = (setup, store) else {
            return Err((REFUSED, "this host cannot pair by PIN".to_owned()));
        };
        let asked = asked(&params)?;
        if running.0.fetch_add(1, Ordering::SeqCst) >= MAX_PAIRINGS {
            running.0.fetch_sub(1, Ordering::SeqCst);
            return Err((
                REFUSED,
                format!(
                    "{MAX_PAIRINGS} pairings are under way: each holds a PIN and a listener, so end one first"
                ),
            ));
        }
        Ok((setup.clone(), Arc::clone(store), asked))
    })();
    let (setup, store, asked) = match started {
        Ok(started) => started,
        Err((code, message)) => return error_line(&id, code, &message),
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let (out, events) = mpsc::channel();
    let worker = {
        let (cancelled, out) = (Arc::clone(&cancelled), out.clone());
        std::thread::spawn(move || {
            let run = Run {
                identity: setup.identity,
                store,
                home: setup.home,
                timeout: asked.timeout,
                cancelled,
            };
            let shown = out.clone();
            let done = crate::pair_run::run(asked.door, &run, &mut |value| {
                shown
                    .send(Out::Shown(value.clone()))
                    .map_err(|_| "the link ended".to_owned())
            });
            let _ = out.send(Out::Done(done));
        })
    };
    let answer = loop {
        match events.recv_timeout(POLL) {
            Ok(Out::Shown(value)) => {
                if channel.send(&notification(&request, &value)).is_err() {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(Out::Done(Ok(peer))) => {
                break serde_json::to_vec(
                    &json!({"jsonrpc": "2.0", "id": id, "result": {"paired": peer}}),
                )
                .unwrap_or_default();
            }
            Ok(Out::Done(Err(Fail::Usage(why)))) => break error_line(&id, super::BAD_PARAMS, &why),
            Ok(Out::Done(Err(Fail::Run(why)))) => break error_line(&id, REFUSED, &why),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break error_line(&id, REFUSED, "the pairing ended without an answer");
            }
        }
        match channel.receive(Some(Duration::from_millis(1))) {
            Ok(Some(next)) => {
                if let Some(reply) = others(&next)
                    && channel.send(&reply).is_err()
                {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Err(Failure::Timeout) => {}
            Ok(None) | Err(_) => cancelled.store(true, Ordering::SeqCst),
        }
    };
    let _ = worker.join();
    running.0.fetch_sub(1, Ordering::SeqCst);
    answer
}
