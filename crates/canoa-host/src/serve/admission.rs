//! Page admission, in-process [`Local`] handles, and delegation [`Host`] implementation.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::Ordering;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread;

use coqui_link::auth::{Proof, Trust};
use coqui_peer_webcrypto::{PeerId, TrustStore};

use super::connection::{Reach, serve_proven};
use super::{Agent, Run, Served, Shared};
use crate::client_tools;
use crate::config::Config;
use crate::control;
use crate::delegate::{self, Principal};
use crate::local::{self, Link};

/// What a page this host's own web server admitted holds besides what its key was given: what `[host.native] control` gives a
/// browser key, and the `services` grant when a session service has something to give its sessions (the mail service, once an
/// account is named and its tools are on).
pub(super) fn page_grants(
    config: &Config,
    services: &crate::services::Services,
) -> BTreeSet<control::Grant> {
    let mut grants = config.native.control.clone();
    if services.offered_any() {
        grants.insert(control::Grant::Services);
    }
    grants
}

/// Why a page's key was not admitted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotAdmitted {
    /// The key is neither accepted nor waiting here: it never connected, so there is nothing to accept.
    NeverConnected,
    /// It is waiting, and the trust store knows it connected from another computer.
    NotLocal,
    /// The trust store could not be read or written.
    Store,
}

/// The host's own web server's way to accept the key of the page it serves: the page connects once, is turned away and waits
/// in the trust store, and then asks for itself by id. What the key holds on the control channel afterwards is
/// `[host.native] control`, as the browser extension's key holds through the door.
pub(crate) struct Admission {
    store: Arc<TrustStore>,
    admitted: Arc<Mutex<BTreeSet<PeerId>>>,
    /// The file the admitted keys are kept in, and whether a secret was asked for when they were admitted.
    record: Option<(PathBuf, bool)>,
}

impl Admission {
    /// Admission to `store`, holding the keys it admits in `admitted`.
    pub(crate) const fn new(
        store: Arc<TrustStore>,
        admitted: Arc<Mutex<BTreeSet<PeerId>>>,
    ) -> Self {
        Self {
            store,
            admitted,
            record: None,
        }
    }

    /// Keeps the keys it admits in `file`, so that a host that starts again holds them as it did. `secret` is whether a request
    /// has to carry the add-in's secret: keys admitted without one are not held again once it is asked for.
    pub(crate) fn remembering(mut self, file: PathBuf, secret: bool) -> Self {
        self.record = Some((file, secret));
        self
    }

    /// Accepts the waiting key `id` when it connected from this computer, and holds it as a page's key for this run; a key
    /// accepted already is held the same way.
    pub(crate) fn admit(&self, id: &PeerId) -> Result<(), NotAdmitted> {
        let store = &self.store;
        if !store.is_accepted(id) {
            let waiting = store.waiting().map_err(|_| NotAdmitted::Store)?;
            let entry = waiting
                .iter()
                .find(|entry| entry.id == *id)
                .ok_or(NotAdmitted::NeverConnected)?;
            // A connection's source is not always known to the trust store; one that is known and is not this computer's is not
            // the page's.
            if entry
                .source
                .is_some_and(|source| !source.ip().is_loopback())
            {
                return Err(NotAdmitted::NotLocal);
            }
            store.accept(id).map_err(|_| NotAdmitted::Store)?;
        }
        let all: Vec<String> = {
            let mut admitted = self.admitted.lock().unwrap_or_else(PoisonError::into_inner);
            admitted.insert(*id);
            admitted.iter().map(ToString::to_string).collect()
        };
        tracing::info!(%id, "the page this host serves was admitted: its key is accepted");
        if let Some((file, secret)) = &self.record
            && let Err(error) = write_admitted(file, *secret, &all)
        {
            // The key is held for this run; after a restart the page asks again.
            tracing::warn!("the admitted keys could not be kept: {error}");
        }
        Ok(())
    }
}

/// Keeps `keys` in `file`, replacing what it held.
pub(super) fn write_admitted(
    file: &std::path::Path,
    secret: bool,
    keys: &[String],
) -> std::io::Result<()> {
    let text = serde_json::json!({ "secret": secret, "keys": keys }).to_string();
    crate::replace::write_file(file, text.as_bytes(), crate::replace::SHARED)
}

/// The keys `file` holds that the add-in's page may still hold: those the trust store still accepts (a key taken away with `revoke`
/// is not held again), and, when a request now has to carry the secret, only those admitted when it had to. A file that is not
/// there or cannot be read holds none.
pub(super) fn read_admitted(
    file: &std::path::Path,
    secret: bool,
    store: &TrustStore,
) -> BTreeSet<PeerId> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BTreeSet::new();
    };
    if secret && value["secret"] != serde_json::Value::Bool(true) {
        return BTreeSet::new();
    }
    value["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter_map(|key| key.parse::<PeerId>().ok())
        .filter(|key| store.is_accepted(key))
        .collect()
}

/// Why [`Local::open`] gave no link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalError {
    /// The host serves no entry of that name.
    NoEntry,
    /// The host is stopping.
    Stopped,
    /// A thread for the connection could not be started.
    Thread,
}

impl fmt::Display for LocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoEntry => "this host serves no entry of that name",
            Self::Stopped => "this host is stopping",
            Self::Thread => "a thread for the connection could not be started",
        })
    }
}

impl core::error::Error for LocalError {}

/// A way into every entry of a [`super::Serving`] from inside its own process. A link opened here is served as a paired key's
/// connection is, with the entry's agent, scope, worktrees and `max_peers`, but without a proof or the trust store: whoever
/// holds a `Local` is this process, and no peer on a network can obtain one. It ignores an entry's `peers` list for that
/// reason, and ends with [`super::Stopper::stop`] like any connection.
#[derive(Clone)]
pub struct Local {
    pub(super) shared: Arc<Shared>,
    pub(super) served: Vec<Arc<Served>>,
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Local").finish_non_exhaustive()
    }
}

impl Local {
    /// The names of the entries it reaches, in the configuration's order.
    #[must_use]
    pub fn entries(&self) -> Vec<&str> {
        self.served
            .iter()
            .map(|served| served.name.as_str())
            .collect()
    }

    /// Opens a connection to `entry`: the client's end of it, whose first message starts the entry's agent as a peer's would.
    /// Dropping or closing the end ends the connection and its agent.
    ///
    /// # Errors
    ///
    /// [`LocalError`]: no such entry, the host is stopping, or no thread could be started.
    pub fn open(&self, entry: &str) -> Result<Link, LocalError> {
        self.open_with(entry, None)
    }

    /// The secret this host shares with an application in its process, for the secret prompts of the connections [`Local::open`]
    /// gives it: 32 bytes minted when the host started, kept in memory, sent over no wire and to no agent. The application derives
    /// its half of the binding from it (`coqui_acp::secret::LinkKey`); `None` when the system has no randomness to mint one from.
    #[must_use]
    pub fn link_secret(&self) -> Option<zeroize::Zeroizing<[u8; 32]>> {
        self.shared.secrets.link_secret()
    }

    /// [`Local::open`], for a task of the delegation when `context` is given: what the task's connection is bounded by.
    pub(crate) fn open_with(
        &self,
        entry: &str,
        context: Option<delegate::ChildContext>,
    ) -> Result<Link, LocalError> {
        if entry == control::NAME {
            // Only this process's own link opens it; a task's connection has no entry of that name, whatever the list holds.
            return if reaches_control(entry, context.as_ref()) {
                self.open_control()
            } else {
                Err(LocalError::NoEntry)
            };
        }
        if entry == client_tools::NAME {
            // A program in this process offering its tools; a task's connection has no such entry.
            return if context.is_none() {
                self.open_tools()
            } else {
                Err(LocalError::NoEntry)
            };
        }
        let served = self
            .served
            .iter()
            .find(|served| served.name == entry)
            .ok_or(LocalError::NoEntry)?;
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) = local::pair(self.shared.max_line, Arc::clone(&self.shared.stopped));
        let shared = Arc::clone(&self.shared);
        let served = Arc::clone(served);
        thread::Builder::new()
            .name("canoa-local".to_owned())
            .spawn(move || {
                let peer = shared.method.peer_id();
                serve_proven(
                    &shared,
                    &served,
                    &host,
                    &peer,
                    context.as_ref(),
                    Reach::InProcess,
                );
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }

    /// `@tools`, as this process: it offers its tools to the host's sessions.
    fn open_tools(&self) -> Result<Link, LocalError> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) =
            local::pair(client_tools::MAX_MESSAGE, Arc::clone(&self.shared.stopped));
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("canoa-tools".to_owned())
            .spawn(move || {
                let grants = control::Grant::all().into_iter().collect();
                shared
                    .client_tools
                    .provide(&shared.method.peer_id().to_string(), &grants, &host);
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }

    /// The control channel, as its owner.
    fn open_control(&self) -> Result<Link, LocalError> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) = local::pair(control::MAX_REQUEST, Arc::clone(&self.shared.stopped));
        let control = Arc::clone(&self.shared.control);
        thread::Builder::new()
            .name("canoa-control".to_owned())
            .spawn(move || {
                control::serve(&control, &control::Caller::Owner, &host);
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }
}

/// Whether `entry` names the control channel for a caller that may have it: this process's own link, never a task's connection,
/// which is bounded by the context it was opened with and so gets no entry of that name.
pub(super) fn reaches_control(entry: &str, context: Option<&delegate::ChildContext>) -> bool {
    entry == control::NAME && context.is_none()
}

/// The host as the delegation sees it: the agents it serves, who may use them, and a way to open one in process.
pub(super) struct DelegationHost {
    pub(super) shared: alloc::sync::Weak<Shared>,
    pub(super) served: Vec<Arc<Served>>,
    pub(super) agents: Vec<delegate::Source>,
    pub(super) identity: coqui_link::auth::Identity,
    pub(super) usage: Option<Arc<crate::usage_log::UsageLog>>,
}

impl delegate::Host for DelegationHost {
    fn sources(&self) -> Vec<delegate::Source> {
        self.agents.clone()
    }

    fn may_use(&self, who: &Principal, source: &str) -> bool {
        // Another machine's agent takes the task's text off this one: only the person's own sessions may send it there.
        let listed = &self.agents;
        if listed
            .iter()
            .any(|agent| agent.name == source && agent.remote.is_some())
        {
            return matches!(who, Principal::Owner);
        }
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let Some(served) = self.served.iter().find(|served| served.name == source) else {
            return false;
        };
        match who {
            Principal::Owner => true,
            Principal::Key(peer) => shared.allows(peer, served),
            Principal::Remote(peer) => shared.lists(peer, served),
        }
    }

    fn kind(&self, source: &str) -> Option<String> {
        let served = self.served.iter().find(|served| served.name == source)?;
        let entries = &self.agents;
        match &served.run {
            Run::Agent {
                agent: Agent::Harness(_),
                ..
            } => entries
                .iter()
                .find(|agent| agent.name == source)
                .and_then(|agent| agent.harness.as_deref())
                .map(|harness| format!("harness:{harness}")),
            Run::Agent {
                agent: Agent::Program { program, .. },
                ..
            } => Some(format!("program:{}", program.to_string_lossy())),
            _ => None,
        }
    }

    fn usage_of(&self, source: &str, session: &str) -> Option<serde_json::Value> {
        let served = self.served.iter().find(|served| served.name == source)?;
        let Run::Agent { scope, .. } = &served.run else {
            return None;
        };
        self.usage
            .as_ref()?
            .session_usage(scope.account_id()?, session)
    }

    fn max_message(&self) -> usize {
        self.shared
            .upgrade()
            .map_or(crate::config::DEFAULT_MAX_MESSAGE_BYTES, |shared| {
                shared.max_line
            })
    }

    fn prover(&self, peer: &PeerId) -> Result<Proof, String> {
        Proof::builder(self.identity.clone())
            .responders(Trust::pinned([*peer]))
            .build()
            .map_err(|error| error.to_string())
    }

    fn choices(
        &self,
        source: &str,
        who: &Principal,
        account: Option<&str>,
    ) -> Option<serde_json::Value> {
        let shared = self.shared.upgrade()?;
        let caller = match who {
            Principal::Owner => control::Caller::Owner,
            Principal::Key(peer) | Principal::Remote(peer) => control::Caller::Peer {
                id: peer.to_string(),
                grants: BTreeSet::new(),
            },
        };
        shared.control.delegation_choices(&caller, source, account)
    }

    fn open(&self, source: &str, context: delegate::ChildContext) -> Result<Link, LocalError> {
        let shared = self.shared.upgrade().ok_or(LocalError::Stopped)?;
        Local {
            shared,
            served: self.served.clone(),
        }
        .open_with(source, Some(context))
    }
}

#[cfg(test)]
mod write_tests {
    use crate::replace::proof;

    #[test]
    fn the_admitted_keys_are_whole_with_nothing_beside_them_and_keep_their_mode() {
        let dir = proof::folder("admitted");
        let file = dir.join("deeper").join("admitted.json");
        super::write_admitted(&file, false, &["a".to_owned()]).expect("kept");
        super::write_admitted(&file, true, &["a".to_owned(), "b".to_owned()]).expect("kept");
        let held: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read")).expect("JSON");
        assert_eq!(held["keys"].as_array().map(Vec::len), Some(2));
        assert_eq!(proof::names(&dir.join("deeper")), ["admitted.json"]);
        #[cfg(unix)]
        {
            proof::set_mode(&file, 0o640);
            super::write_admitted(&file, true, &["c".to_owned()]).expect("kept");
            assert_eq!(proof::mode(&file), 0o640);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
