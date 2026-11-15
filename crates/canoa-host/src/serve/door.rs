//! The door: one pipe or Unix socket per home, owner-only, that the browser's native messaging host relays to. A connection says
//! in its first message which entry it opens, and from there it is the ordinary link: the same proof, trust store and scope as
//! on an entry's address. The door admits nobody by itself.

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::thread;

use coqui_agentic_peer::auth;
use coqui_transport::Channel as _;
use serde_json::{Value, json};

use super::{Entered, Served, Shared, connection};
use crate::native::{self, MAX_PREAMBLE};
use crate::pipe;

/// How long a connection has to say what it opens.
const PREAMBLE_WAIT: Duration = Duration::from_secs(5);
/// How often the accept loop looks to see whether the host is stopping.
const STOP_POLL: Duration = Duration::from_millis(100);

/// What the door serves.
pub(super) struct Door {
    listener: pipe::Listener,
    /// What `@hello` answers: a claim, with no proof, to a process of this user.
    hello: Value,
    served: Vec<Arc<Served>>,
    control: Arc<Served>,
    home: PathBuf,
}

impl Door {
    pub(super) fn new(
        listener: pipe::Listener,
        hello: Value,
        served: Vec<Arc<Served>>,
        control: Arc<Served>,
        home: PathBuf,
    ) -> Self {
        Self {
            listener,
            hello,
            served,
            control,
            home,
        }
    }

    /// Takes connections until the host stops.
    pub(super) fn run(&self, shared: &Arc<Shared>) {
        while !shared.stopped.load(Ordering::SeqCst) {
            let stream = match self.listener.accept_timeout(Some(STOP_POLL)) {
                Ok(stream) => stream,
                Err(pipe::Error::TimedOut) => continue,
                Err(error) => {
                    tracing::warn!("a connection to the native door failed: {error}");
                    thread::sleep(STOP_POLL);
                    continue;
                }
            };
            let Some(entered) = shared.unproven.enter() else {
                tracing::warn!(
                    "a connection to the native door was dropped: too many are unproven"
                );
                continue;
            };
            let channel = pipe::PipeChannel::new(stream, MAX_PREAMBLE);
            let shared = Arc::clone(shared);
            let hello = self.hello.clone();
            let served = self.served.clone();
            let control = Arc::clone(&self.control);
            let home = self.home.clone();
            let spawned = thread::Builder::new()
                .name("canoa-door".to_owned())
                .spawn(move || {
                    meet(
                        &shared,
                        (&hello, &served, &control),
                        &home,
                        &channel,
                        entered,
                    );
                    channel.close();
                });
            if let Err(error) = spawned {
                tracing::warn!("a connection to the native door was dropped: {error}");
            }
        }
    }
}

fn reply(channel: &pipe::PipeChannel, value: &Value) -> bool {
    channel.send(value.to_string().as_bytes()).is_ok()
}

fn meet(
    shared: &Shared,
    (hello, served, control): (&Value, &[Arc<Served>], &Arc<Served>),
    home: &Path,
    channel: &pipe::PipeChannel,
    entered: Entered,
) {
    let Ok(Some(first)) = channel.receive(Some(PREAMBLE_WAIT)) else {
        return;
    };
    let Some(wanted) = native::read_preamble(&first) else {
        reply(channel, &json!({ "error": "not a native door request" }));
        return;
    };
    let target = match wanted.open.as_str() {
        native::HELLO => {
            reply(channel, hello);
            return;
        }
        crate::control::NAME => control,
        name => match served.iter().find(|served| served.name == name) {
            Some(served) => served,
            None => {
                reply(channel, &json!({ "error": "no such entry" }));
                return;
            }
        },
    };
    if !reply(channel, &json!({ "opened": true })) {
        return;
    }
    channel.set_max_message(auth::MAX_PROOF_MESSAGE);
    tracing::debug!(
        entry = target.name,
        origin = wanted.origin,
        "a browser reached the door"
    );
    // A key bound to the extension that paired it is not served to another origin.
    let origin = wanted.origin;
    let admit = |peer: &coqui_peer_webcrypto::PeerId| {
        native::origin_allows(home, &peer.to_string(), &origin)
    };
    connection(shared, target, channel, entered, Some(&admit));
}
