import assert from "node:assert/strict";
import { test } from "node:test";

import { dialAgent, methodOf, pairWithPin, recordLimit, unreachable } from "./acp-dial.js";
import { addedMethod, mcpConfigCommand, readCandidates, sameKey } from "./acp-discovery.js";
import * as noise from "./vendor/coqui-peer-noise.mjs";
import * as webcrypto from "./vendor/coqui-peer-webcrypto.mjs";

/** A WebSocket in memory whose far side is `far(bytes, reply, end)`, called for each message the browser sends. */
function socketTo(far) {
  return class Socket {
    binaryType = "";
    listeners = { open: [], message: [], close: [], error: [] };
    constructor() {
      setTimeout(() => this.emit("open", {}));
    }
    addEventListener(type, listen) {
      this.listeners[type].push(listen);
    }
    emit(type, event) {
      for (const listen of this.listeners[type]) listen(event);
    }
    // The far side takes each message after it finished with the one before, as one connection would.
    turn = Promise.resolve();
    send(data) {
      const bytes = typeof data === "string" ? new TextEncoder().encode(data) : new Uint8Array(data);
      const reply = (answer) => setTimeout(() => this.emit("message", { data: new Uint8Array(answer).slice().buffer }));
      this.turn = this.turn.then(() => far(bytes, reply, () => this.close()));
    }
    close() {
      if (this.closed) return;
      this.closed = true;
      setTimeout(() => this.emit("close", {}));
    }
  };
}

test("an agent's method is the one stored with its key, and one stored before methods is WebCrypto's", async () => {
  assert.equal(methodOf({}), "webcrypto");
  assert.equal(methodOf({ method: "noise" }), "noise");
  assert.equal(await unreachable("webcrypto"), null);
  // Node 22 and later have X25519, as Chrome 133, Firefox 130 and Safari 18.4 do.
  assert.equal(await unreachable("noise"), null);
  assert.match(await unreachable("tls"), /cannot run/);
});

test("a Noise agent is proved by the Noise handshake and every message after it is sealed", async () => {
  const server = await noise.generateIdentity();
  const client = await noise.generateIdentity();
  const responder = await noise.createResponder(server, { allowClient: async (id) => id === client.peerId });
  let step = 0;
  let session;
  const heard = [];
  const Socket = socketTo(async (bytes, reply) => {
    step += 1;
    if (step === 1) reply(await responder.challenge(bytes));
    else if (step === 2) ({ session } = await responder.accept(bytes));
    else {
      const opened = await session.open(bytes);
      if (opened === null) return;
      heard.push(new TextDecoder().decode(opened));
      reply(await session.seal(`echo ${heard.at(-1)}`));
    }
  });
  const link = await dialAgent({ url: "ws://127.0.0.1:1/", peer: server.peerId, method: "noise" }, { identity: client, WebSocketClass: Socket });
  const answer = new Promise((resolve) => link.onMessage(resolve));
  await link.send("hello");
  assert.equal(await answer, "echo hello");
  assert.deepEqual(heard, ["hello"]);
});

test("a key of one method never proves an agent of the other, and nothing falls back", async () => {
  const webKey = await webcrypto.generateIdentity();
  const noiseAgent = await noise.generateIdentity();
  const Socket = socketTo(() => assert.fail("nothing is sent with the wrong key"));
  await assert.rejects(
    dialAgent({ url: "ws://127.0.0.1:1/", peer: noiseAgent.peerId, method: "noise" }, { identity: webKey, WebSocketClass: Socket }),
    /a webcrypto key cannot prove a noise agent/,
  );
  // A Noise agent answering a WebCrypto proof fails it: the browser never retries by another method.
  const sent = [];
  const Closing = socketTo((bytes, _reply, end) => {
    sent.push(bytes);
    end();
  });
  await assert.rejects(dialAgent({ url: "ws://127.0.0.1:1/", peer: noiseAgent.peerId }, { identity: webKey, WebSocketClass: Closing }));
  assert.equal(sent.length, 1);
  assert.match(new TextDecoder().decode(sent[0]), /"profile":"dev.boriken.coqui.peer-webcrypto\/v1"/);
});

/** A responder's side of PIN pairing, as `coqui-acp-bridge pair-accept --method noise` runs it. */
async function pairingDesk() {
  const identity = await noise.generateIdentity();
  const desk = noise.createPairingDesk();
  const { pin } = desk.showPin();
  const accepted = [];
  const incoming = [];
  let waiting;
  const recv = () => new Promise((resolve) => (incoming.length ? resolve(incoming.shift()) : (waiting = resolve)));
  let responder;
  let send;
  let done;
  const Socket = socketTo(async (bytes, reply) => {
    send = reply;
    if (responder === undefined) {
      responder = await noise.createPairingResponder(identity);
      reply(await responder.challenge(bytes));
    } else if (done === undefined) {
      const { pairing } = await responder.accept(bytes);
      done = pairing.respond(desk, { send: async (record) => send(record), recv }, { accept: async (peerId) => accepted.push(peerId) })
        .catch((error) => ({ error }));
    } else if (waiting) {
      const take = waiting;
      waiting = undefined;
      take(bytes);
    } else {
      incoming.push(bytes);
    }
  });
  return { identity, pin, accepted, Socket, finished: () => done };
}

test("the browser pairs by the PIN a Noise machine shows, and learns the key that machine proved", async () => {
  const desk = await pairingDesk();
  const peer = await pairWithPin({ url: "ws://127.0.0.1:1/pair", pin: desk.pin }, { identity: await noise.generateIdentity(), WebSocketClass: desk.Socket });
  assert.equal(peer, desk.identity.peerId);
  assert.equal(desk.accepted.length, 1);
});

test("a wrong PIN pairs nobody, and says so", async () => {
  const desk = await pairingDesk();
  const wrong = desk.pin === "000000" ? "000001" : "000000";
  await assert.rejects(pairWithPin({ url: "ws://127.0.0.1:1/pair", pin: wrong }, { identity: await noise.generateIdentity(), WebSocketClass: desk.Socket }), (error) => error.refusal === "wrong-code");
  assert.deepEqual(desk.accepted, []);
});

test("a candidate names its method, none is WebCrypto's, and one this browser cannot run is not offered", () => {
  const peer = `sha256:${"a".repeat(64)}`;
  const item = (method) => ({ label: "Agent", url: "wss://10.0.0.5:8765/acp", peer, source: "dns-sd", ...(method === undefined ? {} : { method }) });
  assert.equal(readCandidates({ candidates: [item()] })[0].method, "webcrypto");
  assert.equal(readCandidates({ candidates: [item("noise")] })[0].method, "noise");
  assert.equal(readCandidates({ candidates: [item("webcrypto")] })[0].method, "webcrypto");
  assert.deepEqual(readCandidates({ candidates: [item("tls"), item(7)] }), []);
});

test("a link ended without the agent's close record is reported as cut, and one closed in order as closed", async () => {
  for (const sendsClose of [false, true]) {
    const server = await noise.generateIdentity();
    const client = await noise.generateIdentity();
    const responder = await noise.createResponder(server, { allowClient: async () => true });
    let step = 0;
    let session;
    const Socket = socketTo(async (bytes, reply, end) => {
      step += 1;
      if (step === 1) reply(await responder.challenge(bytes));
      else if (step === 2) ({ session } = await responder.accept(bytes));
      else {
        await session.open(bytes);
        if (sendsClose) reply(await session.close());
        setTimeout(end, 50);
      }
    });
    const link = await dialAgent({ url: "ws://127.0.0.1:1/", peer: server.peerId, method: "noise" }, { identity: client, WebSocketClass: Socket });
    const ended = new Promise((resolve) => link.onClose(() => resolve(link.ending)));
    await link.send("hello");
    assert.equal(await ended, sendsClose ? "closed" : "cut");
  }
});

test("a frame limit takes the longest record a Noise sender can make, key changes and all", async () => {
  const MESSAGE = 16 * 1024 * 1024;
  const server = await noise.generateIdentity();
  const client = await noise.generateIdentity();
  const initiator = await noise.createInitiator(client, server.peerId, { keyLimit: 1 << 20 });
  const responder = await noise.createResponder(server, { allowClient: async () => true });
  const challenge = await responder.challenge(await initiator.hello());
  const { reply, session } = await initiator.answer(challenge);
  await responder.accept(reply);
  const record = await session.seal(new Uint8Array(MESSAGE));
  assert.ok(record.length <= recordLimit("noise", MESSAGE), `${record.length - recordLimit("noise", MESSAGE)} over`);
  assert.equal(recordLimit("webcrypto", 10), 27);
});

test("an added agent's method is the one named, else WebCrypto's as for a record, and a known key under another method is refused", () => {
  assert.equal(addedMethod(undefined), "webcrypto");
  assert.equal(addedMethod(null), "webcrypto");
  assert.equal(addedMethod("noise"), "noise");
  assert.throws(() => addedMethod("tls"), /noise or webcrypto/);
  const known = [{ key: "a", peer: "sha256:1", method: "noise" }, { key: "b", peer: "sha256:2" }];
  assert.equal(sameKey(known, "sha256:1", "noise").key, "a");
  assert.equal(sameKey(known, "sha256:2", "webcrypto").key, "b");
  assert.equal(sameKey(known, "sha256:3", "noise"), undefined);
  assert.throws(() => sameKey(known, "sha256:1", "webcrypto"), /a noise key/);
  assert.throws(() => sameKey(known, "sha256:2", "noise"), /a webcrypto key/);
});

test("the mcp-config command names this browser's key for the machine's method, and none it cannot prove by", () => {
  const ids = { noise: "sha256:n", webcrypto: "sha256:w" };
  assert.equal(mcpConfigCommand(ids, "webcrypto"), "coqui-acp-bridge mcp-config --peer sha256:w");
  assert.equal(mcpConfigCommand(ids, "noise"), "coqui-acp-bridge mcp-config --peer sha256:n");
  assert.equal(mcpConfigCommand({ noise: null, webcrypto: "sha256:w" }, "noise"), null);
  assert.equal(mcpConfigCommand(ids, undefined), "coqui-acp-bridge mcp-config --peer sha256:w");
});

/** A Noise agent that proves itself, then turns this key away by ending the connection `after` ms past the handshake. */
async function refusingAgent(after) {
  const server = await noise.generateIdentity();
  const client = await noise.generateIdentity();
  const responder = await noise.createResponder(server, { allowClient: async () => true });
  let step = 0;
  const Socket = socketTo(async (bytes, reply, end) => {
    step += 1;
    if (step === 1) reply(await responder.challenge(bytes));
    else if (step === 2) {
      await responder.accept(bytes);
      setTimeout(end, after);
    }
  });
  return { agent: { url: "ws://127.0.0.1:1/", peer: server.peerId, method: "noise" }, client, Socket };
}

test("a Noise agent that turns this key away right after the handshake fails the dial, as a refused WebCrypto proof does", async () => {
  const { agent, client, Socket } = await refusingAgent(5);
  const started = Date.now();
  await assert.rejects(dialAgent(agent, { identity: client, WebSocketClass: Socket }), /turned this browser's key away/);
  assert.ok(Date.now() - started < 3000, `${Date.now() - started} ms`);
});

test("a link that ended before anyone listened still tells whoever listens later", async () => {
  const { agent, client, Socket } = await refusingAgent(20);
  const link = await dialAgent(agent, { identity: client, WebSocketClass: Socket, settle: 0 });
  await new Promise((resolve) => setTimeout(resolve, 200));
  const heard = await Promise.race([
    new Promise((resolve) => link.onClose(() => resolve("closed"))),
    new Promise((resolve) => setTimeout(() => resolve("never"), 1000)),
  ]);
  assert.equal(heard, "closed");
  assert.equal(link.ending, "cut");
});
