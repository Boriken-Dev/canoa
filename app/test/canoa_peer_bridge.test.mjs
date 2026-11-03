// web/canoa_peer_bridge.mjs a step at a time, as the app's Dart drives it, against each method's own responder.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createPeer, unreachable} from '../web/canoa_peer_bridge.mjs';
import * as webcrypto from '../web/vendor/coqui-peer-webcrypto.mjs';
import * as noise from '../web/vendor/coqui-peer-noise.mjs';

function browser() {
  const identities = {webcrypto: webcrypto.generateIdentity(), noise: noise.generateIdentity()};
  return createPeer(method => identities[method]);
}

/** Runs `token`'s steps, handing each message sent to `peer`, which answers the messages it sends back. */
async function drive(bridge, token, peer) {
  const wire = [];
  let received = null;
  for (let steps = 0; steps < 16; steps++) {
    const {send, peerId, session} = await bridge.step(token, received);
    wire.push(...send);
    const answers = [];
    for (const message of send) answers.push(...await peer(message));
    if (peerId !== null) return {peerId, session, wire};
    assert.equal(answers.length, 1, 'the peer answers each step once');
    received = answers[0];
  }
  throw new Error('no end');
}

test('each method has a key of its own, and a method nobody knows is unreachable', async () => {
  const bridge = browser();
  const ids = [await bridge.localId('noise'), await bridge.localId('webcrypto')];
  assert.notEqual(ids[0], ids[1]);
  assert.equal(await unreachable('noise'), null);
  assert.equal(await unreachable('webcrypto'), null);
  assert.match(await unreachable('tls'), /cannot run/);
  await assert.rejects(bridge.localId('tls'), /unreachable/);
});

test('a WebCrypto proof sends text, as before, and ends in a session', async () => {
  const bridge = browser();
  const server = await webcrypto.generateIdentity();
  const client = await bridge.localId('webcrypto');
  const responder = await webcrypto.createResponder(server, {allowClient: async id => id === client});
  let serverSession;
  let stage = 0;
  const {peerId, session, wire} = await drive(bridge, await bridge.begin('webcrypto', server.peerId), async message => {
    const text = new TextDecoder('utf-8', {fatal: true}).decode(message);
    if (stage++ === 0) return [new TextEncoder().encode(await responder.challenge(text))];
    const accepted = await responder.accept(text);
    serverSession = accepted.session;
    return [new TextEncoder().encode(accepted.reply)];
  });
  assert.equal(peerId, server.peerId);
  assert.match(new TextDecoder().decode(wire[0]), /"profile":"dev.boriken.coqui.peer-webcrypto\/v1"/);
  const record = await bridge.seal(session, new TextEncoder().encode('hello'));
  assert.equal(new TextDecoder().decode(await serverSession.open(record)), 'hello');
});

test('a Noise proof sends its binary handshake and ends in a session both ways', async () => {
  const bridge = browser();
  const server = await noise.generateIdentity();
  const client = await bridge.localId('noise');
  const responder = await noise.createResponder(server, {allowClient: async id => id === client});
  let serverSession;
  const {peerId, session, wire} = await drive(bridge, await bridge.begin('noise', server.peerId), async message => {
    if (serverSession === undefined && message.length === 32) return [await responder.challenge(message)];
    ({session: serverSession} = await responder.accept(message));
    return [];
  });
  assert.equal(peerId, server.peerId);
  assert.deepEqual(wire.map(message => message.length), [32, 64]);
  const record = await bridge.seal(session, new TextEncoder().encode('hello'));
  assert.equal(new TextDecoder().decode(await serverSession.open(record)), 'hello');
  const back = await serverSession.seal('back');
  assert.equal(new TextDecoder().decode(await bridge.open(session, back)), 'back');
  await assert.rejects(bridge.step('unknown', null), /expired or unknown/);
});

test('a Noise proof of a WebCrypto agent fails, and nothing tries the other method', async () => {
  const bridge = browser();
  const server = await webcrypto.generateIdentity();
  const responder = await webcrypto.createResponder(server, {allowClient: async () => true});
  const token = await bridge.begin('noise', server.peerId);
  const {send} = await bridge.step(token, null);
  assert.equal(send[0].length, 32);
  await assert.rejects((async () => {
    await responder.challenge(new TextDecoder().decode(send[0]));
  })());
  await assert.rejects(bridge.step(token, new Uint8Array(96)));
  await assert.rejects(bridge.step(token, new Uint8Array(96)), /expired or unknown/);
});

/** `coqui-acp-bridge pair-accept --method noise`'s side: a desk showing a PIN, answering each message the joiner sends. */
async function pairingDesk() {
  const identity = await noise.generateIdentity();
  const desk = noise.createPairingDesk();
  const {pin} = desk.showPin();
  const accepted = [];
  const responder = await noise.createPairingResponder(identity);
  const inbox = [];
  let waiting;
  let outbox = [];
  let done;
  let wake;
  const recv = () => new Promise(resolve => (inbox.length ? resolve(inbox.shift()) : (waiting = resolve)));
  const transport = {
    send: async record => {
      outbox.push(record);
      wake?.();
    },
    recv,
  };
  // What the desk sends after one message, once it waits for the next or has ended.
  const settle = async () => {
    for (let spins = 0; spins < 200 && outbox.length === 0; spins++) await new Promise(resolve => { wake = resolve; setTimeout(resolve, 5); });
    const sent = outbox;
    outbox = [];
    return sent;
  };
  let stage = 0;
  const peer = async message => {
    switch (stage++) {
      case 0: return [await responder.challenge(message)];
      case 1: {
        const {pairing} = await responder.accept(message);
        done = pairing.respond(desk, transport, {accept: async peerId => accepted.push(peerId)}).catch(error => ({error}));
        return [];
      }
      default:
        if (waiting) {
          const take = waiting;
          waiting = undefined;
          take(message);
        } else {
          inbox.push(message);
        }
        return settle();
    }
  };
  return {identity, pin, accepted, peer, finished: () => done};
}

/** A pairing's steps, where a step may send several messages and the desk answers the last. */
async function pair(bridge, pin, desk) {
  const token = await bridge.pair(pin);
  let received = null;
  try {
    for (let steps = 0; steps < 16; steps++) {
      const {send, peerId} = await bridge.step(token, received);
      let answers = [];
      for (const message of send) answers = answers.concat(await desk.peer(message));
      if (peerId !== null) return peerId;
      assert.equal(answers.length, 1);
      received = answers[0];
    }
    throw new Error('no end');
  } finally {
    bridge.abort(token);
  }
}

test('the app pairs by the PIN a Noise machine shows, and learns the key that machine proved', async () => {
  const bridge = browser();
  const desk = await pairingDesk();
  assert.equal(await pair(bridge, desk.pin, desk), desk.identity.peerId);
  assert.deepEqual(desk.accepted, [await bridge.localId('noise')]);
});

test('a wrong PIN pairs nobody, and says it was refused', async () => {
  const bridge = browser();
  const desk = await pairingDesk();
  const wrong = desk.pin === '000000' ? '000001' : '000000';
  await assert.rejects(pair(bridge, wrong, desk), /refused: wrong-code/);
  assert.deepEqual(desk.accepted, []);
});
