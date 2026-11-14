// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Noise method of the web app's peer bridge: `coqui-peer-noise`'s handshake and session, and
//! `coqui-peer`'s PIN pairing inside it, behind a surface a page can call one step at a time.
//!
//! The page's private key is a non-extractable WebCrypto X25519 key, so this module never holds
//! it: it knows the public half, and the dialing handshake pauses at the one Diffie-Hellman that
//! uses the private key (`pendingDh`), which the page computes with WebCrypto and hands back
//! (`provideDh`).
//!
//! A [`NoiseRun`] is a proof or a pairing. Its `step` takes the peer's last message (none for the
//! first) and gives back the messages to send, in order. A proof ends in a [`NoiseSession`], a
//! pairing in the id of the key it learned. Nothing here reads a clock, a file or the network.

use coqui_peer::pair::{CodeAlphabet, PairRefusal, Pairing, PairingError, Pin, PinRole};
use coqui_peer::{Handshake as _, Method as _, PeerId};
use coqui_peer_noise::{
    Error, Identity, JoinExchange, Noise, NoiseHandshake, PairingHandshake, PairingMethod, Session,
    Trust,
};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

#[cfg(feature = "test-peer")]
mod peer;

/// What a page is told when a step fails. It never holds a key, a PIN or handshake bytes.
fn failed(error: impl core::fmt::Display) -> JsError {
    JsError::new(&error.to_string())
}

/// `The pairing was refused: wrong-code`, the words a refusal has always had on the page.
fn pairing_message(error: Error) -> String {
    let Error::Pairing(PairingError::Refused(refusal)) = &error else {
        return error.to_string();
    };
    let reason = match refusal {
        PairRefusal::WrongCode => "wrong-code",
        PairRefusal::Expired => "expired",
        PairRefusal::Exhausted => "exhausted",
        PairRefusal::Throttled => "throttled",
        PairRefusal::Denied => "denied",
        PairRefusal::Disabled => "disabled",
        _ => "refused",
    };
    format!("The pairing was refused: {reason}")
}

fn pairing_failed(error: Error) -> JsError {
    JsError::new(&pairing_message(error))
}

fn messages(sent: Vec<Vec<u8>>) -> Array {
    sent.iter()
        .map(|message| Uint8Array::from(message.as_slice()))
        .collect()
}

/// This page's Noise key as far as this module knows it: the public half of an X25519 key whose
/// private half stays in WebCrypto. The id is the one the full key has.
#[wasm_bindgen]
pub struct NoiseKey {
    identity: Identity,
}

#[wasm_bindgen]
impl NoiseKey {
    /// The key of a 32-byte public key (RFC 7748's encoding), refused unless it is the one
    /// canonical encoding of a key.
    #[wasm_bindgen(js_name = fromPublicKey)]
    pub fn from_public_key(public: &[u8]) -> Result<NoiseKey, JsError> {
        let public: [u8; 32] = public.try_into().map_err(|_| failed("A key is 32 bytes"))?;
        Ok(NoiseKey {
            identity: Identity::from_public_key(&public).map_err(failed)?,
        })
    }

    #[wasm_bindgen(getter, js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }
}

enum Kind {
    Proof {
        handshake: Option<NoiseHandshake>,
        session: Option<NoiseSession>,
    },
    Pair {
        handshake: Option<PairingHandshake>,
        pin: Option<Pin>,
        exchange: Option<Box<JoinExchange>>,
    },
}

/// A proof of a pinned agent's key, or a PIN pairing, run a step at a time.
#[wasm_bindgen]
pub struct NoiseRun {
    kind: Kind,
    /// 0 the first message, 1 the responder's second, 2 waiting for the page's Diffie-Hellman
    /// result, 3 and on the rest.
    stage: u8,
    peer: Option<String>,
    /// The static public key of the peer a proof proved, as its SubjectPublicKeyInfo.
    peer_key: Option<Vec<u8>>,
    failure: Option<String>,
}

const NOT_AGAIN: &str = "Unexpected message after the run";
const NO_MESSAGE: &str = "Unexpected step without a message";
const WAITING: &str = "Unexpected step before the static key's result";

#[wasm_bindgen]
impl NoiseRun {
    /// Proves the agent whose id is `expected_peer_id`, and no other.
    pub fn prove(key: &NoiseKey, expected_peer_id: &str) -> Result<NoiseRun, JsError> {
        let expected: PeerId = expected_peer_id.parse().map_err(failed)?;
        let handshake = Noise::builder()
            .identity(key.identity.clone())
            .responders(Trust::pinned([expected]))
            .build()
            .map_err(failed)?
            .initiate()
            .map_err(failed)?;
        Ok(NoiseRun {
            kind: Kind::Proof {
                handshake: Some(handshake),
                session: None,
            },
            stage: 0,
            peer: None,
            peer_key: None,
            failure: None,
        })
    }

    /// Pairs this key by the typed `pin` with whatever agent proves a key to this page.
    pub fn pair(key: &NoiseKey, pin: &str) -> Result<NoiseRun, JsError> {
        let pin = Pin::parse(&CodeAlphabet::pin(), pin).map_err(failed)?;
        let handshake = PairingMethod::builder()
            .identity(key.identity.clone())
            .build()
            .map_err(failed)?
            .initiate()
            .map_err(failed)?;
        Ok(NoiseRun {
            kind: Kind::Pair {
                handshake: Some(handshake),
                pin: Some(pin),
                exchange: None,
            },
            stage: 0,
            peer: None,
            peer_key: None,
            failure: None,
        })
    }

    /// The next step, given the peer's last message or nothing for the first: the messages to
    /// send now, as an array of `Uint8Array`. The step that takes the responder's second message
    /// sends nothing: [`pending_dh`](NoiseRun::pending_dh) then names the point the page's private
    /// key is to compute X25519 with, and [`provide_dh`](NoiseRun::provide_dh) goes on. After the
    /// step that ends the run, [`peer_id`](NoiseRun::peer_id) names the key it proved or learned.
    pub fn step(&mut self, received: Option<Vec<u8>>) -> Result<Array, JsError> {
        let at = self.stage;
        self.stage = at.saturating_add(1);
        let sent = match &mut self.kind {
            Kind::Proof { handshake, .. } => step_proof(at, handshake, received)?,
            Kind::Pair {
                handshake,
                exchange,
                ..
            } => step_pair(
                at,
                &mut self.peer,
                &mut self.failure,
                handshake,
                exchange,
                received,
            )?,
        };
        Ok(messages(sent))
    }

    /// While the run waits for the page's static key: the 32-byte point to compute X25519 with.
    #[wasm_bindgen(getter, js_name = pendingDh)]
    pub fn pending_dh(&self) -> Option<Vec<u8>> {
        let point = match &self.kind {
            Kind::Proof { handshake, .. } => handshake.as_ref()?.pending_static_dh(),
            Kind::Pair { handshake, .. } => handshake.as_ref()?.pending_static_dh(),
        };
        point.map(|point| point.to_vec())
    }

    /// Goes on with the X25519 result the page's static key computed for
    /// [`pending_dh`](NoiseRun::pending_dh): the messages to send now. A result of all zeros is
    /// refused and ends the run.
    #[wasm_bindgen(js_name = provideDh)]
    pub fn provide_dh(&mut self, shared: &[u8]) -> Result<Array, JsError> {
        if self.stage != 2 {
            return Err(failed(NOT_AGAIN));
        }
        self.stage = 3;
        let sent = match &mut self.kind {
            Kind::Proof { handshake, session } => {
                let active = handshake.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
                active.provide_static_dh(shared).map_err(failed)?;
                let sent = drain(active)?;
                let established = handshake
                    .take()
                    .ok_or_else(|| failed(NOT_AGAIN))?
                    .finish()
                    .map_err(failed)?;
                self.peer = Some(established.peer.to_string());
                self.peer_key = established.key.map(|key| key.bytes);
                *session = Some(NoiseSession {
                    inner: established.session,
                });
                sent
            }
            Kind::Pair {
                handshake,
                pin,
                exchange,
            } => {
                let active = handshake.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
                active.provide_static_dh(shared).map_err(failed)?;
                let mut sent = drain(active)?;
                let session = handshake
                    .take()
                    .ok_or_else(|| failed(NOT_AGAIN))?
                    .finish()
                    .map_err(failed)?
                    .session;
                let pin = pin.take().ok_or_else(|| failed(NOT_AGAIN))?;
                let mut joining = session
                    .joiner(Pairing::pin(pin, PinRole::Typed))
                    .map_err(pairing_failed)?;
                while let Some(record) = joining.transmit().map_err(failed)? {
                    sent.push(record);
                }
                *exchange = Some(Box::new(joining));
                sent
            }
        };
        Ok(messages(sent))
    }

    /// The id of the key this run proved or learned, once it has.
    #[wasm_bindgen(getter, js_name = peerId)]
    pub fn peer_id(&self) -> Option<String> {
        self.peer.clone()
    }

    /// The peer's static public key (its 44-byte SubjectPublicKeyInfo) once a proof has proved it: the key whose digest is
    /// [`peer_id`](NoiseRun::peer_id), so the page can make a Diffie-Hellman with it. Never a private key.
    #[wasm_bindgen(getter, js_name = peerKey)]
    pub fn peer_key(&self) -> Option<Vec<u8>> {
        self.peer_key.clone()
    }

    /// Why a pairing ended in a refusal. The messages its step gave are the refusal to send the
    /// responder first.
    #[wasm_bindgen(getter)]
    pub fn failure(&self) -> Option<String> {
        self.failure.clone()
    }

    /// The session a proof ended in, once; none for a pairing or before the proof ends.
    #[wasm_bindgen(js_name = takeSession)]
    pub fn take_session(&mut self) -> Option<NoiseSession> {
        match &mut self.kind {
            Kind::Proof { session, .. } => session.take(),
            Kind::Pair { .. } => None,
        }
    }
}

/// Everything a handshake has to send until it waits for the peer.
fn drain<H: coqui_peer::Handshake>(handshake: &mut H) -> Result<Vec<Vec<u8>>, JsError> {
    let mut sent = Vec::new();
    while let Some(message) = handshake.transmit().map_err(failed)? {
        sent.push(message);
    }
    Ok(sent)
}

/// A Noise proof: hello, then the responder's second message, which pauses the run at the
/// static key's Diffie-Hellman. The initiator holds the session once it sends its third message,
/// before the responder decides.
fn step_proof(
    at: u8,
    slot: &mut Option<NoiseHandshake>,
    received: Option<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, JsError> {
    match at {
        0 => drain(slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?),
        1 => {
            let message = received.ok_or_else(|| failed(NO_MESSAGE))?;
            let handshake = slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
            handshake.receive(&message).map_err(failed)?;
            Ok(Vec::new())
        }
        2 => Err(failed(WAITING)),
        _ => Err(failed("Unexpected message after the proof")),
    }
}

/// A PIN pairing: the Noise pairing handshake, which pauses like a proof, then the exchange
/// inside it, a record at a time.
fn step_pair(
    at: u8,
    peer: &mut Option<String>,
    failure: &mut Option<String>,
    slot: &mut Option<PairingHandshake>,
    exchange: &mut Option<Box<JoinExchange>>,
    received: Option<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, JsError> {
    match at {
        0 => return drain(slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?),
        1 => {
            let message = received.ok_or_else(|| failed(NO_MESSAGE))?;
            let handshake = slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
            handshake.receive(&message).map_err(failed)?;
            return Ok(Vec::new());
        }
        2 => return Err(failed(WAITING)),
        _ => {}
    }
    let message = received.ok_or_else(|| failed(NO_MESSAGE))?;
    let joining = exchange
        .as_mut()
        .filter(|_| peer.is_none() && failure.is_none())
        .ok_or_else(|| failed("Unexpected message after the pairing"))?;
    if let Err(error) = joining.receive(&message) {
        // The responder is told why, as far as the page can still send it; the page then reports
        // the failure.
        let mut sent = Vec::new();
        while let Ok(Some(record)) = joining.transmit() {
            sent.push(record);
        }
        *failure = Some(pairing_message(error));
        return Ok(sent);
    }
    let mut sent = Vec::new();
    while let Some(record) = joining.transmit().map_err(failed)? {
        sent.push(record);
    }
    if joining.is_complete() {
        let (paired, close) = exchange
            .take()
            .ok_or_else(|| failed(NOT_AGAIN))?
            .finish()
            .map_err(pairing_failed)?;
        sent.push(close);
        *peer = Some(paired.peer.id().to_string());
    }
    Ok(sent)
}

/// The session a proof ends in: every later message is one sealed record, in call order.
#[wasm_bindgen]
pub struct NoiseSession {
    inner: Session,
}

#[wasm_bindgen]
impl NoiseSession {
    /// One message as one record.
    pub fn seal(&mut self, message: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner.seal(message).map_err(failed)
    }

    /// The message of the next record, or none for the peer's close. A bad record ends the
    /// session.
    pub fn open(&mut self, record: &[u8]) -> Result<Option<Vec<u8>>, JsError> {
        self.inner.open(record).map_err(failed)
    }

    /// The record that ends this direction, to send last.
    pub fn close(&mut self) -> Result<Vec<u8>, JsError> {
        self.inner.close().map_err(failed)
    }
}
