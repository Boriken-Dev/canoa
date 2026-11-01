import {createInitiator, loadOrCreateIdentity} from './vendor/coqui-peer-webcrypto.mjs';

// Application UI uses this bridge only for the peer-proof exchange and the session it ends in. It does not expose arbitrary
// signing, browser APIs, credentials or trusted-identity changes to an agent. No unpaired peer is silently accepted.
const STORE = 'dev.boriken.canoa.peer';
const active = new Map();
const sessions = new Map();
const ttl = 30_000;
function handshake(token) {
  const current = active.get(token);
  if (!current || Date.now() - current.created > ttl) {
    active.delete(token);
    throw new Error('Peer proof expired or unknown');
  }
  return current.handshake;
}
function established(token) {
  const session = sessions.get(token);
  if (!session) throw new Error('Session closed or unknown');
  return session;
}
function token() {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
}
const peer = Object.freeze({
  async localId() {
    return (await loadOrCreateIdentity(STORE)).peerId;
  },
  async begin(expectedPeerId) {
    if (active.size >= 16) throw new Error('Too many concurrent peer handshakes');
    const identity = await loadOrCreateIdentity(STORE);
    const started = await createInitiator(identity, expectedPeerId);
    const id = token();
    active.set(id, {created: Date.now(), handshake: started});
    return JSON.stringify({token: id, hello: started.hello(), localPeerId: identity.peerId});
  },
  async answer(id, challenge) {
    return handshake(id).answer(challenge);
  },
  async finish(id, accept) {
    try {
      const {peerId, session} = await handshake(id).finish(accept);
      const sealing = token();
      sessions.set(sealing, session);
      return JSON.stringify({peerId, session: sealing});
    } finally {
      active.delete(id);
    }
  },
  abort(id) { active.delete(id); },
  // Calls take their place in the session in call order, so the caller makes them in order.
  seal(id, message) { return established(id).seal(message); },
  // Null for the peer's close.
  open(id, record) { return established(id).open(record); },
  closeRecord(id) { return established(id).close(); },
  verifyEnd(id) { established(id).verifyEnd(); },
  closeSession(id) {
    sessions.get(id)?.destroy();
    sessions.delete(id);
  },
});
globalThis.canoaPeer = peer;
