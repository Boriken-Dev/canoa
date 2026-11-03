//! Pairing: which keys may connect. A key that proves itself and is not accepted waits in the trust store, and a person
//! accepts it after comparing its id with the one their client shows.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;

use coqui_peer_webcrypto::{PeerId, TrustStore};

/// Where a key stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeerState {
    /// It may connect.
    Accepted,
    /// It proved it holds its key and was turned away; accepting it pairs it.
    Waiting {
        /// Where it connected from, when known.
        source: Option<SocketAddr>,
    },
}

/// One key the trust store knows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Peer {
    /// Its id.
    pub id: PeerId,
    /// Where it stands.
    pub state: PeerState,
}

/// Why the trust store could not do what was asked.
#[derive(Debug)]
#[non_exhaustive]
pub enum PeersError {
    /// The text is not a peer id.
    NotAnId,
    /// No key with that id is waiting.
    NotWaiting,
    /// That key is accepted already.
    AlreadyAccepted,
    /// The store could not be read or written; its words.
    Store(String),
}

impl fmt::Display for PeersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnId => f.write_str("that is not a peer id"),
            Self::NotWaiting => {
                f.write_str("no key with that id is waiting: connect with it once, then accept it")
            }
            Self::AlreadyAccepted => f.write_str("that key is accepted already"),
            Self::Store(why) => write!(f, "the trust store failed: {why}"),
        }
    }
}

impl core::error::Error for PeersError {}

fn store_error(error: impl fmt::Display) -> PeersError {
    PeersError::Store(error.to_string())
}

/// Every accepted key, then every waiting one, each sorted by id.
///
/// # Errors
///
/// [`PeersError::Store`] when the store cannot be read.
pub fn list(store: &TrustStore) -> Result<Vec<Peer>, PeersError> {
    let mut peers: Vec<Peer> = store
        .accepted()
        .map_err(store_error)?
        .into_iter()
        .map(|id| Peer {
            id,
            state: PeerState::Accepted,
        })
        .collect();
    peers.extend(
        store
            .waiting()
            .map_err(store_error)?
            .into_iter()
            .map(|entry| Peer {
                id: entry.id,
                state: PeerState::Waiting {
                    source: entry.source,
                },
            }),
    );
    Ok(peers)
}

/// Accepts the waiting key `id`.
///
/// # Errors
///
/// [`PeersError::NotAnId`], [`PeersError::NotWaiting`], [`PeersError::AlreadyAccepted`], or [`PeersError::Store`].
pub fn accept(store: &TrustStore, id: &str) -> Result<PeerId, PeersError> {
    let id: PeerId = id.trim().parse().map_err(|_| PeersError::NotAnId)?;
    store.accept(&id).map_err(|error| match error {
        coqui_peer::Error::NotWaiting => PeersError::NotWaiting,
        coqui_peer::Error::AlreadyAccepted => PeersError::AlreadyAccepted,
        other => store_error(other),
    })?;
    Ok(id)
}

/// Removes `id`, accepted or waiting. A link it holds ends within about a second. `false` when the store did not know it.
///
/// # Errors
///
/// [`PeersError::NotAnId`] or [`PeersError::Store`].
pub fn revoke(store: &TrustStore, id: &str) -> Result<bool, PeersError> {
    let id: PeerId = id.trim().parse().map_err(|_| PeersError::NotAnId)?;
    store.remove(&id).map_err(store_error)
}
