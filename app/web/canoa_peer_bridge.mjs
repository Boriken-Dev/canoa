import {createInitiator, loadOrCreateIdentity} from './canoa_peer.mjs';

// Application UI uses this bridge only for the peer-proof exchange. It does
// not expose arbitrary signing, browser APIs, credentials or trusted-identity
// changes to an agent. No unpaired peer is silently accepted.
const active = new Map();
const ttl = 30_000;
function session(token) {
  const current = active.get(token);
  if (!current || Date.now() - current.created > ttl) {
    active.delete(token);
    throw new Error('Peer proof expired or unknown');
  }
  return current.handshake;
}
function token() {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
}
const peer = Object.freeze({
  async begin(expectedPeerId) {
    if (active.size >= 16) throw new Error('Too many concurrent peer handshakes');
    const identity = await loadOrCreateIdentity();
    const handshake = await createInitiator(identity, expectedPeerId);
    const id = token();
    active.set(id, {created: Date.now(), handshake});
    return JSON.stringify({token: id, hello: handshake.hello(), localPeerId: identity.peerId});
  },
  async answer(id, challenge) {
    return session(id).answer(challenge);
  },
  async finish(id, accept) {
    try { return await session(id).finish(accept); }
    finally { active.delete(id); }
  },
  abort(id) { active.delete(id); },
});
globalThis.canoaPeer = peer;
