//! The other end of a proof, for browser tests of the page's side: it admits one pinned key.
//! Compiled only with the `test-peer` feature.

use coqui_peer::{Handshake as _, Method as _, PeerId};
use coqui_peer_noise::{Identity, Noise, NoiseHandshake, Trust};
use js_sys::Array;
use wasm_bindgen::prelude::*;

use crate::{NoiseSession, failed, messages};

/// A key with its private half, for the responder: the page's own key never has one here.
#[wasm_bindgen]
pub struct TestKey {
    identity: Identity,
}

#[wasm_bindgen]
impl TestKey {
    pub fn generate() -> Result<TestKey, JsError> {
        Ok(TestKey {
            identity: Identity::generate().map_err(failed)?,
        })
    }

    #[wasm_bindgen(getter, js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }
}

/// A responder of the Noise method that admits `allowed` alone.
#[wasm_bindgen]
pub struct NoiseResponder {
    handshake: Option<NoiseHandshake>,
    peer: Option<String>,
    session: Option<NoiseSession>,
}

#[wasm_bindgen]
impl NoiseResponder {
    pub fn new(key: &TestKey, allowed: &str) -> Result<NoiseResponder, JsError> {
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

    #[wasm_bindgen(getter, js_name = peerId)]
    pub fn peer_id(&self) -> Option<String> {
        self.peer.clone()
    }

    #[wasm_bindgen(js_name = takeSession)]
    pub fn take_session(&mut self) -> Option<NoiseSession> {
        self.session.take()
    }
}
