//! Pairing by PIN, which `canoa pair` and the control channel's `host/pair` both run: a PIN is shown, one client that proves a key
//! and enters it is accepted in the trust store, and a wrong try ends it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and sockets are std's"
)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use coqui_agentic_peer::auth::{self, Identity};
use coqui_agentic_peer::pairing;
use coqui_peer_webcrypto::TrustStore;
use coqui_peer_webcrypto::pair::PairingDesk;
use coqui_transport::{Channel as _, Failure};
use coqui_transport_web::Limits;
use serde_json::{Value, json};

/// Connections pairing at once; past it a new one is dropped.
const MAX_PAIRING_RUNS: usize = 16;
/// How long one connection may take to pair.
const PAIRING_RUN: Duration = Duration::from_secs(60);

/// No browser sends this as its `Origin`, so the pairing listener takes only clients that send none: native programs.
const NO_BROWSER: &str = "-";

/// How long the loop waits for a connection before it looks at the clock and the cancel flag again.
const POLL: Duration = Duration::from_millis(100);

/// Where the PIN is dialled.
pub(crate) enum Door {
    /// A plain WebSocket listener on this address. `origin` is the one browser origin that may pair, and the one the key is bound to;
    /// without it a browser is refused before its handshake, so a page cannot spend the PIN.
    Network {
        listen: SocketAddr,
        origin: Option<String>,
    },
    /// The pipe or socket in the home that only the browser extension's native messaging host reaches, for this user.
    Native,
}

/// What a pairing is run with.
pub(crate) struct Run {
    /// The host's own identity: what the other side pins, and the method it pairs by.
    pub(crate) identity: Arc<Identity>,
    pub(crate) store: Arc<TrustStore>,
    /// The host's home, where an origin a key is bound to is recorded and the native door's name comes from.
    pub(crate) home: PathBuf,
    /// How long to wait for the PIN to be used.
    pub(crate) timeout: Duration,
    /// Raised to end the pairing with the PIN unused.
    pub(crate) cancelled: Arc<AtomicBool>,
}

/// Why a pairing did not end in a key.
pub(crate) enum Fail {
    /// The request was wrong.
    Usage(String),
    /// The pairing could not be done, or ended without a key.
    Run(String),
}

impl Fail {
    fn run(why: impl ToString) -> Self {
        Self::Run(why.to_string())
    }
}

/// Takes back the PIN when the run ends, however it ends: a connection still pairing runs on a thread of its own, which in an
/// application's process goes on after the command has ended.
struct ClearPin<'a>(&'a PairingDesk);

impl Drop for ClearPin<'_> {
    fn drop(&mut self) {
        self.0.clear_pin();
    }
}

/// One connection that reached the door, to be run to its end on a thread of its own.
type Job = Box<dyn FnOnce() -> Result<String, String> + Send>;

/// Waits a moment for a connection at the door: the job that pairs it, or none yet.
type Accept = Box<dyn Fn() -> Result<Option<Job>, String>>;

/// Shows a PIN (`shown` gets `{pin, peer, method, url | native}`) and waits for the key that uses it: its peer id.
///
/// # Errors
///
/// [`Fail::Usage`] for a bad origin; [`Fail::Run`] when the door cannot be opened, the PIN is used up or expires, nobody pairs in
/// time, or `run.cancelled` is raised.
pub(crate) fn run(
    door: Door,
    run: &Run,
    shown: &mut dyn FnMut(&Value) -> Result<(), String>,
) -> Result<String, Fail> {
    let identity = &run.identity;
    let store = &run.store;
    let desk = pairing::desk().map_err(Fail::run)?;
    let (mut announce, accept): (Value, Accept) = match door {
        Door::Network { listen, origin } => {
            if origin
                .as_deref()
                .is_some_and(|origin| !coqui_acp_bridge::valid_origin(origin))
            {
                return Err(Fail::Usage(
                    "an origin is scheme://host or scheme://host:port with no path, and never null"
                        .to_owned(),
                ));
            }
            // The PIN exchange runs inside the method's own session and takes one guess, so where the socket sits is the
            // person's to choose. A page could run its own PIN exchange and spend the one try, so a browser is refused before
            // the handshake unless its origin is named.
            let mut limits = Limits::default();
            limits.max_message = auth::MAX_PROOF_MESSAGE;
            limits.pending = 8;
            let origins = vec![origin.clone().unwrap_or_else(|| NO_BROWSER.to_owned())];
            let listener =
                crate::serve::plain_listener(listen, limits, origins).map_err(Fail::run)?;
            let bound = listener.local_addr().map_err(Fail::run)?;
            if listener.plain_beyond_loopback() {
                tracing::info!(
                    "non-default: pair --listen is {bound}, default loopback: any machine that reaches it may try the PIN"
                );
            }
            let (identity, store, desk, dir) = (
                Arc::clone(identity),
                Arc::clone(store),
                desk.clone(),
                run.home.clone(),
            );
            (
                json!({"url": format!("ws://{bound}")}),
                Box::new(move || {
                    let pending = match listener.next(Some(POLL)) {
                        Ok(pending) => pending,
                        Err(Failure::Timeout) => return Ok(None),
                        Err(failure) => return Err(failure.to_string()),
                    };
                    let (identity, store, desk, dir, origin) = (
                        Arc::clone(&identity),
                        Arc::clone(&store),
                        desk.clone(),
                        dir.clone(),
                        origin.clone(),
                    );
                    Ok(Some(Box::new(move || {
                        pending
                            .handshake()
                            .map_err(|failure| failure.to_string())
                            .and_then(|channel| {
                                let paired = pairing::respond(
                                    &channel, &identity, &store, &desk, None, PAIRING_RUN,
                                );
                                channel.close();
                                paired.map_err(|error| error.to_string())
                            })
                            .and_then(|paired| {
                                let peer = paired.peer.id();
                                // The one origin that may pair is the one the key is bound to, as a key paired through the
                                // door is.
                                match &origin {
                                    Some(origin) => crate::native::record_origin(
                                        &dir,
                                        &peer.to_string(),
                                        origin,
                                    )
                                    .map(|()| peer.to_string())
                                    .map_err(|error| {
                                        let _ = store.remove(&peer);
                                        format!(
                                            "the origin could not be recorded, so the key was not kept: {error}"
                                        )
                                    }),
                                    None => Ok(peer.to_string()),
                                }
                            })
                    }) as Job))
                }),
            )
        }
        Door::Native => {
            let name = crate::native::pair_door_name(&run.home).map_err(Fail::run)?;
            let listener = crate::pipe::Listener::bind(&name).map_err(Fail::run)?;
            let (identity, store, desk, dir) = (
                Arc::clone(identity),
                Arc::clone(store),
                desk.clone(),
                run.home.clone(),
            );
            (
                json!({"native": true}),
                Box::new(move || {
                    let stream = match listener.accept_timeout(Some(POLL)) {
                        Ok(stream) => stream,
                        Err(crate::pipe::Error::TimedOut) => return Ok(None),
                        Err(error) => return Err(error.to_string()),
                    };
                    let (identity, store, desk, dir) = (
                        Arc::clone(&identity),
                        Arc::clone(&store),
                        desk.clone(),
                        dir.clone(),
                    );
                    Ok(Some(Box::new(move || {
                        let channel =
                            crate::pipe::PipeChannel::new(stream, crate::native::MAX_PREAMBLE);
                        let result = crate::native::accept_pairing(&channel)
                            .ok_or_else(|| "a connection did not ask to pair".to_owned())
                            .and_then(|origin| {
                                pairing::respond(
                                    &channel, &identity, &store, &desk, None, PAIRING_RUN,
                                )
                                .map_err(|error| error.to_string())
                                .map(|paired| (paired.peer.id(), origin))
                            })
                            .and_then(|(peer, origin)| {
                                crate::native::record_origin(&dir, &peer.to_string(), &origin)
                                    .map(|()| peer.to_string())
                                    .map_err(|error| {
                                        // A key no extension is bound to is served to any: take it back out rather than leave it.
                                        let _ = store.remove(&peer);
                                        format!(
                                            "the extension could not be recorded, so its key was not kept: {error}"
                                        )
                                    })
                            });
                        channel.close();
                        result
                    }) as Job))
                }),
            )
        }
    };
    let shown_pin = desk.show_pin().map_err(Fail::run)?;
    let _clear = ClearPin(&desk);
    announce["pin"] = json!(shown_pin.pin.as_str());
    announce["peer"] = json!(identity.peer_id().to_string());
    announce["method"] = json!(identity.method().name());
    shown(&announce).map_err(Fail::run)?;
    let until = Instant::now() + run.timeout;
    let (report, reports) = mpsc::channel();
    let running = Arc::new(AtomicUsize::new(0));
    loop {
        if run.cancelled.load(Ordering::SeqCst) {
            return Err(Fail::run("pairing was cancelled"));
        }
        while let Ok(result) = reports.try_recv() {
            match result {
                Ok(peer) => return Ok(peer),
                Err(why) => {
                    tracing::info!("a pairing attempt failed: {why}");
                    if desk.shown_pin().is_none() {
                        return Err(Fail::run("the PIN is used up or expired: start again"));
                    }
                }
            }
        }
        if Instant::now() >= until {
            return Err(Fail::run("nobody paired in time"));
        }
        let Some(job) = accept().map_err(Fail::run)? else {
            continue;
        };
        // Each connection runs apart, so one that connects and says nothing costs neither the PIN nor the wait.
        if running.fetch_add(1, Ordering::SeqCst) >= MAX_PAIRING_RUNS {
            running.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (report, running) = (report.clone(), Arc::clone(&running));
        std::thread::spawn(move || {
            let result = job();
            running.fetch_sub(1, Ordering::SeqCst);
            let _ = report.send(result);
        });
    }
}
