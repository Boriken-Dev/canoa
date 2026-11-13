//! Secret prompts: an agent asks the person for a password or a key, or shows them one, and neither the model, its transcript
//! nor a log ever holds the value. The host serves its agent three tools (`secret_request`, `secret_use`, `secret_show`, from
//! `coqui-acp-secrets`) over a loopback HTTP server behind a bearer minted for one session, and asks the person's app, on the
//! connection the session belongs to, through the `_coqui_acp/secret_*` extension. A value travels sealed to a key only its
//! receiver holds, bound to the identities this connection proved; the agent holds a handle.
//!
//! The person's app is the connection's own peer, whose key the host already knows from pairing. A connection whose peer is the
//! host's own identity (a task's link, an application hosting the library) has no other identity to bind answers with, so its
//! sessions are given no secret tools. Every message of the extension is the host's to send: an agent's is dropped by the scope.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, sockets and the clock are std's"
)]

mod http;
mod layers;
mod link;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use coqui_acp::secret::{BINDING_DH, BINDING_ECDSA, IdentityId, Prover, Verifier, unprotected};
use coqui_acp_secrets::{Secrets, SessionTools};
use coqui_agentic_peer::auth::Identity;
use coqui_peer_webcrypto::{PeerId, TrustStore};
use zeroize::Zeroizing;

pub(crate) use layers::{Layer, Raw};
pub(crate) use link::Link;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a bearer opens: one session's tools, once the agent has named the session.
pub(crate) struct Grant {
    tools: Mutex<Option<SessionTools>>,
    /// Calls this bearer has under way.
    calls: AtomicUsize,
}

/// The bearers the tool server accepts, by their digest: a token is never kept.
#[derive(Default)]
pub(crate) struct Tokens {
    grants: Mutex<HashMap<[u8; 32], Arc<Grant>>>,
}

impl Tokens {
    fn find(&self, digest: &[u8; 32]) -> Option<Arc<Grant>> {
        lock(&self.grants).get(digest).cloned()
    }

    fn add(&self, digest: [u8; 32]) -> Arc<Grant> {
        let grant = Arc::new(Grant {
            tools: Mutex::new(None),
            calls: AtomicUsize::new(0),
        });
        lock(&self.grants).insert(digest, Arc::clone(&grant));
        grant
    }

    fn remove(&self, digest: &[u8; 32]) {
        lock(&self.grants).remove(digest);
    }
}

/// What the host holds for every connection: its identity, the keys it has paired, and the tool server.
pub(crate) struct Hub {
    identity: Identity,
    store: Arc<TrustStore>,
    tokens: Arc<Tokens>,
    server: Mutex<Option<http::Listener>>,
}

/// The identity of the host as a binding method and the keys to make and check one with, for one paired peer.
struct Keys {
    prover: Prover,
    verifier: Verifier,
    binding: &'static str,
}

impl Hub {
    pub(crate) fn new(identity: &Identity, store: &Arc<TrustStore>) -> Arc<Self> {
        Arc::new(Self {
            identity: identity.clone(),
            store: Arc::clone(store),
            tokens: Arc::new(Tokens::default()),
            server: Mutex::new(None),
        })
    }

    /// The secrets of one connection whose peer proved itself as `peer`, or `None` when it has no identity of its own to be
    /// bound to: the host's own, or a key the trust store does not hold.
    pub(crate) fn link(self: &Arc<Self>, peer: &PeerId) -> Option<Arc<Link>> {
        if *peer == self.identity.peer_id() {
            return None;
        }
        let material = self.store.accepted_key(peer)?;
        let owner = IdentityId::of(&material.bytes);
        let keys = match self.keys(&material.bytes, owner) {
            Ok(keys) => keys,
            Err(why) => {
                tracing::info!(%peer, "no secret tools for this connection: {why}");
                return None;
            }
        };
        let secrets = Secrets::builder(keys.prover, keys.verifier)
            .observer(link::observe)
            .build();
        Some(Link::new(Arc::clone(self), secrets, owner, keys.binding))
    }

    /// The host's key as the prover and the verifier of a binding with `owner`, whose `SubjectPublicKeyInfo` is `owner_spki`.
    fn keys(&self, owner_spki: &[u8], owner: IdentityId) -> Result<Keys, &'static str> {
        let pem = match &self.identity {
            Identity::WebCrypto(identity) => identity.to_pkcs8_pem().map_err(|_| ()),
            Identity::Noise(identity) => identity.to_pkcs8_pem().map_err(|_| ()),
            _ => return Err("the host's key is of a method secret prompts do not serve"),
        }
        .map_err(|()| "the host's key could not be read")?;
        let der = Zeroizing::new(
            coqui_peer::pem::decode(&pem)
                .ok_or("the host's key is not PEM")?
                .bytes,
        );
        match &self.identity {
            Identity::WebCrypto(_) => {
                let prover = unprotected::ecdsa_p256_pkcs8(&der).map_err(|_| "a P-256 key")?;
                let verifier = Verifier::builder(prover.id()).trust(owner).build();
                Ok(Keys {
                    prover,
                    verifier,
                    binding: BINDING_ECDSA,
                })
            }
            _ => {
                // RFC 8410: the 32 bytes of the key follow a 16-byte header.
                let secret = der
                    .get(der.len().saturating_sub(32)..)
                    .filter(|_| der.len() == 48)
                    .ok_or("an X25519 key")?;
                let prover =
                    unprotected::x25519_pkcs8(&der, owner_spki).map_err(|_| "an X25519 key")?;
                let (spki, agree) =
                    unprotected::x25519_agreement(secret).map_err(|_| "an X25519 key")?;
                let verifier = Verifier::builder(prover.id())
                    .trust(owner)
                    .agreement(&spki, agree)
                    .map_err(|_| "an X25519 key")?
                    .build();
                Ok(Keys {
                    prover,
                    verifier,
                    binding: BINDING_DH,
                })
            }
        }
    }

    /// The address of the tool server, started on first use.
    fn url(&self) -> Option<String> {
        let mut server = lock(&self.server);
        if server.is_none() {
            match http::Listener::start(&self.tokens) {
                Ok(started) => *server = Some(started),
                Err(error) => {
                    tracing::warn!("the secret tools' server could not start: {error}");
                    return None;
                }
            }
        }
        server
            .as_ref()
            .map(|server| format!("http://127.0.0.1:{}/mcp", server.addr().port()))
    }
}
