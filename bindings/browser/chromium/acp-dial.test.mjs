import assert from "node:assert/strict";
import { test } from "node:test";

import { createDeframer, dialAgent, frame } from "./acp-dial.js";
import { createMux } from "./vendor/coqui-agentic-peer/mux.mjs";
import { createResponder, generateIdentity } from "./vendor/coqui-peer-webcrypto.mjs";

const text = (buffer) => new TextDecoder().decode(buffer);

test("a frame is a little-endian length and its body, as coqui-web's vectors say", () => {
  assert.deepEqual([...frame("hi")], [2, 0, 0, 0, 0x68, 0x69]);
});

test("messages arrive whole however the stream cuts them", () => {
  const bytes = new Uint8Array([...frame("hello"), ...frame("x"), ...frame("world")]);
  for (let size = 1; size <= bytes.length; size += 1) {
    const deframe = createDeframer();
    const seen = [];
    for (let at = 0; at < bytes.length; at += size) seen.push(...deframe(bytes.subarray(at, at + size)));
    assert.deepEqual(seen.map(text), ["hello", "x", "world"], `in pieces of ${size}`);
  }
});

test("a limit can be raised once a peer has proved itself, and a large message in small chunks is not quadratic", () => {
  const deframe = createDeframer(8);
  assert.throws(() => createDeframer(8)(frame("x".repeat(9))));
  deframe.raiseLimit(8 * 1024 * 1024);
  const whole = frame("y".repeat(8 * 1024 * 1024));
  const begun = Date.now();
  let seen = [];
  for (let at = 0; at < whole.length; at += 4096) seen = seen.concat(deframe(whole.subarray(at, at + 4096)));
  assert.equal(seen.length, 1);
  assert.equal(seen[0].byteLength, 8 * 1024 * 1024);
  assert.ok(Date.now() - begun < 2000, `took ${Date.now() - begun} ms`);
});

test("messages survive the buffer moving under them", () => {
  const deframe = createDeframer();
  const seen = [];
  for (let round = 0; round < 50; round += 1) {
    const body = String(round).repeat(1000);
    const framed = frame(body);
    seen.push(...deframe(framed.subarray(0, 2000)), ...deframe(framed.subarray(2000)));
  }
  assert.equal(seen.length, 50);
  assert.equal(text(seen[49]), "49".repeat(1000));
});

test("an empty message or one past the limit is refused before it is held", () => {
  assert.throws(() => createDeframer()(new Uint8Array(4)));
  assert.throws(() => createDeframer(4)(frame("hello")));
  assert.throws(() => createDeframer()(new Uint8Array([255, 255, 255, 255])));
});

/** A far side on a socket in memory: the real proof, then a real mux on the sealed link. */
async function dialedLink(options) {
  const serverIdentity = await generateIdentity();
  const clientIdentity = await generateIdentity();
  const auth = await createResponder(serverIdentity, { allowClient: async () => true });
  let session;
  const heard = { frames: [] };
  const far = { onMessage: () => {}, closed: () => {} };
  class Socket {
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
    deliver(data) {
      this.emit("message", { data });
    }
    async send(data) {
      if (!session) {
        const text = typeof data === "string" ? data : new TextDecoder().decode(data);
        if (!heard.hello) {
          heard.hello = true;
          this.deliver(await auth.challenge(text));
        } else {
          const accepted = await auth.accept(text);
          session = accepted.session;
          far.socket = this;
          this.deliver(accepted.reply);
        }
        return;
      }
      const opened = await session.open(new Uint8Array(data));
      if (opened !== null) {
        heard.frames.push(opened[0]);
        far.onMessage(new Uint8Array(opened));
      }
    }
    close() {
      this.emit("close", {});
    }
  }
  const link = await dialAgent({ peer: serverIdentity.peerId, url: "ws://x" }, { identity: clientIdentity, bare: true, WebSocketClass: Socket, WebTransportClass: undefined, ...options });
  const mux = createMux({
    send: async (bytes) => far.socket.deliver((await session.seal(bytes)).buffer),
    onMessage: (take) => {
      far.onMessage = take;
    },
    onClose: () => {},
  }, { side: "answered" });
  return { link, mux, heard };
}

test("a bare link opens no stream of its own, and refuses one the far side opens, since nothing here would accept it", async () => {
  const { link, mux, heard } = await dialedLink({});
  assert.equal(typeof link.mux.open, "function");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(heard.frames, [], "no acp/1, and nothing else");
  const opened = await mux.open("anything");
  await assert.rejects(opened.receive(), (error) => error.reason === "refused");
});

test("a heartbeat, when asked for, goes out as the mux's own ping", async () => {
  const { heard } = await dialedLink({ heartbeat: { interval: 5, misses: 100 } });
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.ok(heard.frames.includes(4), JSON.stringify(heard.frames));
});
