//! The other end of a proof, for browser tests of the page's side: it admits one pinned key.
//! Compiled only with the `test-peer` feature.

use coqui_peer::{Handshake as _, Method as _, PeerId};
use coqui_peer_noise::{Noise, NoiseHandshake, Trust};
use js_sys::Array;
use wasm_bindgen::prelude::*;

use crate::{NoiseKey, NoiseSession, failed, messages};

/// A responder of the Noise method that admits `allowed` alone.
#[wasm_bindgen]
pub struct NoiseResponder {
    handshake: Option<NoiseHandshake>,
    peer: Option<String>,
    session: Option<NoiseSession>,
}

#[wasm_bindgen]
impl NoiseResponder {
    pub fn new(key: &NoiseKey, allowed: &str) -> Result<NoiseResponder, JsError> {
        let allowed: PeerId = allowed.parse().map_err(failed)?;
        let handshake = Noise::builder()
            .identity(key.identity.clone())
            .initiators(Trust::pinned([allowed]))
            .build()
            .map_err(failed)?
            .respond(None)
            .map_err(failed)?;
        Ok(NoiseResponder {
            handshake: Some(handshake),
            peer: None,
            session: None,
        })
    }

    /// Takes the dialer's message and gives what to send back.
    pub fn step(&mut self, received: Vec<u8>) -> Result<Array, JsError> {
        let handshake = self
            .handshake
            .as_mut()
            .ok_or_else(|| failed("Unexpected message after the proof"))?;
        handshake.receive(&received).map_err(failed)?;
        let mut sent = Vec::new();
        while let Some(message) = handshake.transmit().map_err(failed)? {
            sent.push(message);
        }
        if handshake.is_complete() {
            let established = self
                .handshake
                .take()
                .ok_or_else(|| failed("Unexpected message after the proof"))?
                .finish()
                .map_err(failed)?;
            self.peer = Some(established.peer.to_string());
            self.session = Some(NoiseSession {
                inner: established.session,
            });
        }
        Ok(messages(sent))
    }

    #[wasm_bindgen(getter)]
    pub fn peer_id(&self) -> Option<String> {
        self.peer.clone()
    }

    pub fn take_session(&mut self) -> Option<NoiseSession> {
        self.session.take()
    }
}
