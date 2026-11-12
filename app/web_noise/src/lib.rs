// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Noise method of the web app's peer bridge: `coqui-peer-noise`'s handshake and session, and
//! `coqui-peer`'s PIN pairing inside it, behind a surface a page can call one step at a time.
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
fn pairing_failed(error: Error) -> JsError {
    let Error::Pairing(PairingError::Refused(refusal)) = &error else {
        return failed(error);
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
    JsError::new(&format!("The pairing was refused: {reason}"))
}

fn messages(sent: Vec<Vec<u8>>) -> Array {
    sent.iter()
        .map(|message| Uint8Array::from(message.as_slice()))
        .collect()
}

/// This page's Noise key: an X25519 key, whose SHA-256 is the peer id.
#[wasm_bindgen]
pub struct NoiseKey {
    identity: Identity,
}

#[wasm_bindgen]
impl NoiseKey {
    /// A new key from the browser's random generator.
    pub fn generate() -> Result<NoiseKey, JsError> {
        Ok(NoiseKey {
            identity: Identity::generate().map_err(failed)?,
        })
    }

    /// The key of RFC 8410 PKCS#8 PEM text, exactly as [`to_pem`](NoiseKey::to_pem) writes it.
    pub fn from_pem(text: &str) -> Result<NoiseKey, JsError> {
        Ok(NoiseKey {
            identity: Identity::from_pkcs8_pem(text).map_err(failed)?,
        })
    }

    /// The private key as PKCS#8 PEM text, for the page's own storage.
    pub fn to_pem(&self) -> Result<String, JsError> {
        Ok(self.identity.to_pkcs8_pem().map_err(failed)?.to_string())
    }

    #[wasm_bindgen(getter)]
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
        exchange: Option<JoinExchange>,
    },
}

/// A proof of a pinned agent's key, or a PIN pairing, run a step at a time.
#[wasm_bindgen]
pub struct NoiseRun {
    kind: Kind,
    stage: u8,
    peer: Option<String>,
}

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
        })
    }

    /// The next step, given the peer's last message or nothing for the first: the messages to
    /// send now, as an array of `Uint8Array`. After the step that ends the run, [`peer_id`] names
    /// the key it proved or learned.
    ///
    /// [`peer_id`]: NoiseRun::peer_id
    pub fn step(&mut self, received: Option<Vec<u8>>) -> Result<Array, JsError> {
        let sent = match &mut self.kind {
            Kind::Proof { handshake, session } => step_proof(
                &mut self.stage,
                &mut self.peer,
                handshake,
                session,
                received,
            )?,
            Kind::Pair {
                handshake,
                pin,
                exchange,
            } => step_pair(
                &mut self.stage,
                &mut self.peer,
                handshake,
                pin,
                exchange,
                received,
            )?,
        };
        Ok(messages(sent))
    }

    /// The id of the key this run proved or learned, once it has.
    #[wasm_bindgen(getter)]
    pub fn peer_id(&self) -> Option<String> {
        self.peer.clone()
    }

    /// The session a proof ended in, once; none for a pairing or before the proof ends.
    pub fn take_session(&mut self) -> Option<NoiseSession> {
        match &mut self.kind {
            Kind::Proof { session, .. } => session.take(),
            Kind::Pair { .. } => None,
        }
    }
}

const NOT_AGAIN: &str = "Unexpected message after the run";
const NO_MESSAGE: &str = "Unexpected step without a message";

/// Everything a handshake has to send until it waits for the peer.
fn drain<H: coqui_peer::Handshake>(handshake: &mut H) -> Result<Vec<Vec<u8>>, JsError> {
    let mut sent = Vec::new();
    while let Some(message) = handshake.transmit().map_err(failed)? {
        sent.push(message);
    }
    Ok(sent)
}

/// A Noise proof: the initiator holds the session once it sends its third message, before the
/// responder decides.
fn step_proof(
    stage: &mut u8,
    peer: &mut Option<String>,
    slot: &mut Option<NoiseHandshake>,
    session: &mut Option<NoiseSession>,
    received: Option<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, JsError> {
    let at = *stage;
    *stage = stage.saturating_add(1);
    match at {
        0 => drain(slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?),
        1 => {
            let message = received.ok_or_else(|| failed(NO_MESSAGE))?;
            let handshake = slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
            handshake.receive(&message).map_err(failed)?;
            let sent = drain(handshake)?;
            let established = slot
                .take()
                .ok_or_else(|| failed(NOT_AGAIN))?
                .finish()
                .map_err(failed)?;
            *peer = Some(established.peer.to_string());
            *session = Some(NoiseSession {
                inner: established.session,
            });
            Ok(sent)
        }
        _ => Err(failed("Unexpected message after the proof")),
    }
}

/// A PIN pairing: the Noise pairing handshake, then the exchange inside it, a record at a time.
fn step_pair(
    stage: &mut u8,
    peer: &mut Option<String>,
    slot: &mut Option<PairingHandshake>,
    pin: &mut Option<Pin>,
    exchange: &mut Option<JoinExchange>,
    received: Option<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, JsError> {
    let at = *stage;
    *stage = stage.saturating_add(1);
    if at == 0 {
        return drain(slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?);
    }
    let message = received.ok_or_else(|| failed(NO_MESSAGE))?;
    if at == 1 {
        let handshake = slot.as_mut().ok_or_else(|| failed(NOT_AGAIN))?;
        handshake.receive(&message).map_err(failed)?;
        let mut sent = drain(handshake)?;
        let session = slot
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
        *exchange = Some(joining);
        return Ok(sent);
    }
    let joining = exchange
        .as_mut()
        .filter(|_| peer.is_none())
        .ok_or_else(|| failed("Unexpected message after the pairing"))?;
    joining.receive(&message).map_err(pairing_failed)?;
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
