import {createInitiator, loadOrCreateIdentity} from './vendor/coqui-peer-webcrypto.mjs';

// Application UI uses this bridge only for the peer-proof exchange and the session it ends in. It does not expose arbitrary
// signing, browser APIs, credentials or trusted-identity changes to an agent. No unpaired peer is silently accepted.
const STORE = 'dev.boriken.canoa.peer';
const ttl = 30_000;
function token() {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
}

// The calls the app makes, over the identity `loadIdentity` gives: the page's own, kept in this origin's IndexedDB.
export function createPeer(loadIdentity) {
  const active = new Map();
  const sessions = new Map();
  function handshake(id) {
    const current = active.get(id);
    if (!current || Date.now() - current.created > ttl) {
      active.delete(id);
      throw new Error('Peer proof expired or unknown');
    }
    return current.handshake;
  }
  function established(id) {
    const session = sessions.get(id);
    if (!session) throw new Error('Session closed or unknown');
    return session;
  }
  return Object.freeze({
    async localId() {
      return (await loadIdentity()).peerId;
    },
    async begin(expectedPeerId) {
      if (active.size >= 16) throw new Error('Too many concurrent peer handshakes');
      const identity = await loadIdentity();
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
}

globalThis.canoaPeer = createPeer(() => loadOrCreateIdentity(STORE));
