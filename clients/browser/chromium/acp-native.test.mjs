import assert from "node:assert/strict";
import { test } from "node:test";

import { MAX_MESSAGE_META, NATIVE_PIECE, createDeframer, dialAgent, frame, pairWithPin } from "./acp-dial.js";
import * as noise from "./vendor/coqui-peer-noise.mjs";
import { createResponder, generateIdentity } from "./vendor/coqui-peer-webcrypto.mjs";

const b64 = (bytes) => Buffer.from(bytes).toString("base64");
const unb64 = (text) => new Uint8Array(Buffer.from(text, "base64"));

/**
 * The native messaging host as the browser's port shows it: `open` is answered, then the link's bytes go in base64 pieces both ways.
 * The far side is a real WebCrypto responder, so what reaches it is what the host would be sent.
 */
function relayTo({ identity, entries = ["agent"], refuse, piece = NATIVE_PIECE, onPiece = () => {} }) {
  const seen = { opened: [], pieces: [], records: [] };
  const connectNative = (host) => {
    seen.host = host;
    const listeners = { message: [], disconnect: [] };
    const deframe = createDeframer(64 * 1024 * 1024);
    let auth;
    let session;
    let handshake = 0;
    const say = (message) => queueMicrotask(() => listeners.message.forEach((listen) => listen(message)));
    const deliver = (body) => {
      const bytes = frame(body);
      for (let at = 0; at < bytes.length; at += piece) say({ d: b64(bytes.subarray(at, at + piece)) });
    };
    let chain = Promise.resolve();
    const port = {
      onMessage: { addListener: (listen) => listeners.message.push(listen) },
      onDisconnect: { addListener: (listen) => listeners.disconnect.push(listen) },
      disconnect: () => {
        seen.disconnected = true;
      },
      postMessage: (message) => {
        if (message.op === "open") {
          seen.opened.push(message.entry);
          if (refuse !== undefined) say({ reply: "open", error: refuse });
          else if (!entries.includes(message.entry)) say({ reply: "open", error: "no such entry" });
          else say({ reply: "open", opened: true });
          return;
        }
        assert.equal(typeof message.d, "string", "after the open only pieces go");
        const bytes = unb64(message.d);
        seen.pieces.push(bytes.length);
        onPiece(bytes.length);
        chain = chain.then(async () => {
          for (const data of deframe(bytes)) {
            if (session === undefined) {
              if (handshake === 0) {
                handshake = 1;
                auth = await createResponder(identity, { allowClient: async () => true });
                deliver(await auth.challenge(new TextDecoder().decode(data)));
              } else {
                const accepted = await auth.accept(new TextDecoder().decode(data));
                session = accepted.session;
                deliver(accepted.reply);
              }
            } else {
              const opened = await session.open(new Uint8Array(data));
              if (opened !== null) seen.records.push(new TextDecoder().decode(opened));
            }
          }
        }).catch(() => {});
      },
    };
    seen.say = say;
    seen.seal = async (text) => deliver((await session.seal(text)).buffer);
    seen.hangUp = (why) => {
      listeners.disconnect.forEach((listen) => listen(port, why));
    };
    return port;
  };
  return { connectNative, seen };
}

test("a link to a local entry is proven end to end through the native port, and the entry is named once", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  assert.equal(link.proven, true);
  assert.deepEqual(seen.opened, ["agent"]);
  assert.equal(seen.host, "dev.boriken.canoa");
  const heard = new Promise((resolve) => link.onMessage(resolve));
  await seen.seal('{"hello":"back"}');
  assert.equal(await heard, '{"hello":"back"}');
  await link.send('{"hi":1}');
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(seen.records, ['{"hi":1}']);
});

test("a message past one piece is cut into pieces no longer than the limit and arrives whole", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  const big = "z".repeat(2 * 1024 * 1024);
  await link.send(big);
  for (let wait = 0; wait < 50 && seen.records.length === 0; wait += 1) await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(seen.records[0]?.length, big.length);
  assert.ok(seen.pieces.every((size) => size <= NATIVE_PIECE), seen.pieces.join(","));
  assert.ok(seen.pieces.length >= 4, `${seen.pieces.length} pieces`);
  // A piece, base64 and the message around it, is under the 1 MiB the browser takes.
  assert.ok(JSON.stringify({ d: b64(new Uint8Array(NATIVE_PIECE)) }).length < 1024 * 1024);
  // And pieces come back in pieces: a record in 5 byte pieces is the same record.
  const small = relayTo({ identity, piece: 5 });
  const other = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative: small.connectNative });
  const heard = new Promise((resolve) => other.onMessage(resolve));
  await small.seen.seal("x".repeat(500));
  assert.equal((await heard).length, 500);
});

test("a host that will not open the entry, or goes away, fails the dial with its reason", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const refused = relayTo({ identity, refuse: "no such entry" });
  await assert.rejects(dialAgent({ native: { entry: "x" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative: refused.connectNative }), /no such entry/);
  assert.equal(refused.seen.disconnected, true, "the port is closed");
  await assert.rejects(dialAgent({ native: { entry: "x" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative: () => { throw new Error("Specified native messaging host not found."); } }), /not found/);
  await assert.rejects(dialAgent({ native: { entry: "x" }, peer: identity.peerId, method: "webcrypto" }, { identity: client }), /no native messaging|has no native/);
});

test("a link cut mid-turn ends as a cut link, and nothing is sent again", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  const closed = new Promise((resolve) => link.onClose(resolve));
  const before = seen.pieces.length;
  seen.hangUp("gone");
  await closed;
  assert.equal(link.ending, "cut");
  assert.equal(seen.pieces.length, before, "nothing was retried");
});

test("a host that sends what no host sends ends the link instead of being believed", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  const closed = new Promise((resolve) => link.onClose(resolve));
  // A length no peer may send.
  seen.say({ d: b64(new Uint8Array([255, 255, 255, 255, 1])) });
  await closed;
  assert.equal(link.ending, "cut");
});

test("pairing by PIN can run over the native port, to the host's pairing door", async () => {
  const { connectNative, seen } = relayTo({ identity: await generateIdentity(), entries: ["@pair"] });
  // The far side here is no pairing responder: the pairing has only to reach the pairing door, and ends when the link does.
  setTimeout(() => seen.hangUp("gone"), 50);
  await assert.rejects(
    pairWithPin({ native: true, pin: "123456" }, { identity: await noise.generateIdentity(), wait: 500, connectNative }),
    /closed during the proof/,
  );
  assert.deepEqual(seen.opened, ["@pair"]);
});

const MIB = 1024 * 1024;
const initialized = (limit) => JSON.stringify({
  jsonrpc: "2.0",
  id: 1,
  result: { protocolVersion: 1, ...(limit === undefined ? {} : { _meta: { [MAX_MESSAGE_META]: limit } }) },
});

test("a message past the default limit ends the link, as no host takes one", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  const closed = new Promise((resolve) => link.onClose(resolve));
  link.onMessage(() => assert.fail("a message over the limit was delivered"));
  await seen.seal("x".repeat(20 * MIB));
  await closed;
});

test("the limit the host names in its answer to initialize opens the link to longer messages, and a longer one is refused before it is sealed", async () => {
  const identity = await generateIdentity();
  const client = await generateIdentity();
  const { connectNative, seen } = relayTo({ identity });
  const link = await dialAgent({ native: { entry: "agent" }, peer: identity.peerId, method: "webcrypto" }, { identity: client, connectNative });
  const heard = [];
  link.onMessage((message) => heard.push(message.length));
  await seen.seal(initialized(32 * MIB));
  await seen.seal("y".repeat(20 * MIB));
  for (let wait = 0; wait < 100 && heard.length < 2; wait += 1) await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(heard[1], 20 * MIB);
  assert.equal(link.ending, undefined, "the link is still open");
  await assert.rejects(link.send("z".repeat(33 * MIB)), /max_message_bytes/);
  await link.send("ok");
  for (let wait = 0; wait < 50 && seen.records.length === 0; wait += 1) await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(seen.records, ["ok"]);
});
