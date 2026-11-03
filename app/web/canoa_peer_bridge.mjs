import * as webcrypto from './vendor/coqui-peer-webcrypto.mjs';
import * as noise from './vendor/coqui-peer-noise.mjs';

// Application UI uses this bridge only for the peer-proof exchange, PIN pairing and the session a proof ends in. It does not
// expose arbitrary signing, browser APIs, credentials or trusted-identity changes to an agent. No unpaired peer is silently
// accepted.
// Each method's key in a database of its own, so one key is never used by both. The extension's UI page shares its origin,
// and these names, with the extension.
const STORES = Object.freeze({webcrypto: 'dev.boriken.canoa.peer', noise: 'dev.boriken.canoa.peer-noise'});
const MODULES = Object.freeze({webcrypto, noise});
const PROOF_TTL = 30_000;
// A pairing waits for the person at the other end only as long as its PIN does.
const PAIR_TTL = 120_000;
function token() {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
}
const encode = text => new TextEncoder().encode(text);
const decode = bytes => new TextDecoder('utf-8', {fatal: true}).decode(bytes);
function needs(received) {
  if (received == null) throw new Error('Unexpected step without a message');
  return received;
}

/**
 * Why this runtime cannot prove by `method`, or null. It asks only the local runtime, before any connection, so nothing on the
 * network can change the answer, and a false answer never selects another method.
 */
export async function unreachable(method) {
  if (method === 'webcrypto') return null;
  if (method === 'noise') return (await noise.supported()) ? null : "this browser's WebCrypto has no X25519, which the Noise method needs";
  return 'this browser cannot run that method';
}

// A WebCrypto proof: hello, the answer to the challenge, then the acceptance. Its messages are text.
async function webcryptoProof(identity, expectedPeerId) {
  const started = await webcrypto.createInitiator(identity, expectedPeerId);
  let stage = 0;
  return async received => {
    switch (stage++) {
      case 0: return {send: [encode(started.hello())]};
      case 1: return {send: [encode(await started.answer(decode(needs(received))))]};
      case 2: {
        const {peerId, session} = await started.finish(decode(needs(received)));
        return {send: [], peerId, session};
      }
      default: throw new Error('Unexpected message after the proof');
    }
  };
}

// A Noise proof: the initiator holds the session once it sends its third message, before the responder decides.
async function noiseProof(identity, expectedPeerId) {
  const started = await noise.createInitiator(identity, expectedPeerId);
  let stage = 0;
  return async received => {
    switch (stage++) {
      case 0: return {send: [await started.hello()]};
      case 1: {
        const {reply, peerId, session} = await started.answer(needs(received));
        return {send: [reply], peerId, session};
      }
      default: throw new Error('Unexpected message after the proof');
    }
  };
}

// A PIN pairing under Noise. Its exchange runs inside the module against a transport, so each step hands it the peer's message
// and gives back what it sent until it next waits for the peer, or ends.
async function pinPairing(identity, pin) {
  const near = await noise.createPairingInitiator(identity);
  const outbox = [];
  let deliver = null;
  let ended = null;
  let wake = null;
  const poke = () => {
    const waiting = wake;
    wake = null;
    waiting?.();
  };
  const transport = {
    async send(bytes) {
      outbox.push(new Uint8Array(bytes));
    },
    recv() {
      return new Promise(resolve => {
        deliver = resolve;
        poke();
      });
    },
  };
  async function settled() {
    if (deliver === null && ended === null) await new Promise(resolve => { wake = resolve; });
    if (ended?.error) {
      const {refusal, message} = ended.error;
      throw new Error(refusal ? `The pairing was refused: ${refusal}` : message);
    }
    return {send: outbox.splice(0), peerId: ended?.peerId ?? null, session: null};
  }
  let stage = 0;
  const step = async received => {
    switch (stage++) {
      case 0: return {send: [await near.hello()]};
      case 1: {
        const {reply, pairing} = await near.answer(needs(received));
        outbox.push(reply);
        pairing.join(String(pin), transport).then(
          ({peerId}) => { ended = {peerId}; poke(); },
          error => { ended = {error}; poke(); },
        );
        return settled();
      }
      default: {
        if (ended !== null || deliver === null) throw new Error('Unexpected message after the pairing');
        const take = deliver;
        deliver = null;
        take(needs(received));
        return settled();
      }
    }
  };
  // The connection ending, as the module's transport says it.
  step.abort = () => deliver?.(null);
  return step;
}

// The calls the app makes, over the identities `loadIdentity(method)` gives: the page's own, kept in this origin's IndexedDB.
export function createPeer(loadIdentity) {
  const active = new Map();
  const sessions = new Map();
  async function identity(method) {
    const why = await unreachable(method);
    if (why !== null) throw new Error(`unreachable from this browser: ${why}`);
    return loadIdentity(method);
  }
  function start(run, ttl) {
    if (active.size >= 16) throw new Error('Too many concurrent peer handshakes');
    const id = token();
    active.set(id, {created: Date.now(), ttl, run});
    return id;
  }
  function running(id) {
    const current = active.get(id);
    if (!current || Date.now() - current.created > current.ttl) {
      current?.run.abort?.();
      active.delete(id);
      throw new Error('Peer proof expired or unknown');
    }
    return current.run;
  }
  function established(id) {
    const session = sessions.get(id);
    if (!session) throw new Error('Session closed or unknown');
    return session;
  }
  return Object.freeze({
    unreachable,
    async localId(method) {
      return (await identity(method)).peerId;
    },
    async begin(method, expectedPeerId) {
      const own = await identity(method);
      if (own.method !== method) throw new Error(`a ${own.method} key cannot prove a ${method} agent`);
      return start(await (method === 'noise' ? noiseProof : webcryptoProof)(own, expectedPeerId), PROOF_TTL);
    },
    async pair(pin) {
      return start(await pinPairing(await identity('noise'), pin), PAIR_TTL);
    },
    // `{send, peerId, session}`: messages to send in order; once the peer is proved its id, and for a proof the session's token.
    async step(id, received) {
      try {
        const {send, peerId, session} = await running(id)(received == null ? null : new Uint8Array(received));
        if (session == null) return {send, peerId: peerId ?? null, session: null};
        const sealing = token();
        sessions.set(sealing, session);
        active.delete(id);
        return {send, peerId, session: sealing};
      } catch (error) {
        active.get(id)?.run.abort?.();
        active.delete(id);
        throw error;
      }
    },
    abort(id) {
      active.get(id)?.run.abort?.();
      active.delete(id);
    },
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

const loaded = new Map();
globalThis.canoaPeer = createPeer(method => {
  if (!loaded.has(method)) {
    loaded.set(method, MODULES[method].loadOrCreateIdentity(STORES[method]).catch(error => {
      loaded.delete(method);
      throw error;
    }));
  }
  return loaded.get(method);
});
