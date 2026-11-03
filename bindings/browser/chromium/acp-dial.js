// A proven channel to an agent: the handshake of the agent's method on WebTransport or a WebSocket, then every message sealed.
// Its code ships with the extension, so nothing on the path can replace it. The method is the one stored with the agent's key,
// never one a connection suggests.
import * as webcrypto from "./vendor/coqui-peer-webcrypto.mjs";
import * as noise from "./vendor/coqui-peer-noise.mjs";
import { createMux } from "./vendor/coqui-agentic-peer/mux.mjs";

/** The proof methods, in no order of preference. */
export const METHODS = Object.freeze(["noise", "webcrypto"]);
/** What a key made here, or an agent added with none named, proves by. */
export const DEFAULT_METHOD = "webcrypto";
// Each method's key in a database of its own, so one key is never used by both.
const STORES = Object.freeze({ webcrypto: "dev.boriken.canoa.peer", noise: "dev.boriken.canoa.peer-noise" });
const MODULES = Object.freeze({ webcrypto, noise });
const PROOF_MS = 15_000;
// A pairing waits for the person at the other end only as long as its PIN does.
const PAIR_MS = 120_000;
// A path that drops QUIC shows only as silence, and the WebSocket is still worth trying within the proof's time.
const WEBTRANSPORT_OPEN_MS = 5_000;
// What coqui-acp-bridge takes: a 16 MiB line, as a record of each method.
const MAX_LINE = 16 * 1024 * 1024;
const MAX_MESSAGE = Math.max(MAX_LINE + 64, noise.recordLength(MAX_LINE));
// Nobody has proved anything yet, so nobody gets to send much: the same limit the bridge holds a peer to.
const MAX_PROOF_MESSAGE = 3072;
const MAX_PROOF_WAITING = 8;

const asText = (data) => (typeof data === "string" ? data : new TextDecoder().decode(data));

/** One message as coqui-web frames it on a stream: a little-endian u32 length, then the body. */
export function frame(body) {
  const bytes = typeof body === "string" ? new TextEncoder().encode(body) : new Uint8Array(body);
  const framed = new Uint8Array(4 + bytes.length);
  new DataView(framed.buffer).setUint32(0, bytes.length, true);
  framed.set(bytes, 4);
  return framed;
}

/** Takes the stream's chunks as they come, and returns the whole messages each completes. Throws on what no peer may send. */
export function createDeframer(initialLimit = MAX_MESSAGE) {
  let limit = initialLimit;
  // Grown by doubling, so a large message in small chunks is not copied again at every chunk.
  let buffer = new Uint8Array(4096);
  let start = 0;
  let end = 0;
  const hold = (chunk) => {
    const live = end - start;
    if (end + chunk.length > buffer.length) {
      const larger = live + chunk.length > buffer.length ? new Uint8Array(Math.max(buffer.length * 2, live + chunk.length)) : buffer;
      larger.set(buffer.subarray(start, end));
      buffer = larger;
      start = 0;
      end = live;
    }
    buffer.set(chunk, end);
    end += chunk.length;
  };
  const deframe = (chunk) => {
    hold(chunk);
    const messages = [];
    while (end - start >= 4) {
      const length = new DataView(buffer.buffer, buffer.byteOffset + start, 4).getUint32(0, true);
      if (length === 0 || length > limit) throw new Error("a message of a length no peer may send");
      if (end - start - 4 < length) break;
      messages.push(buffer.slice(start + 4, start + 4 + length).buffer);
      start += 4 + length;
    }
    if (start === end) start = end = 0;
    return messages;
  };
  deframe.raiseLimit = (next) => {
    limit = next;
  };
  return deframe;
}

/** The same small surface as a WebSocket, so the proof does not care which it runs on. */
function webTransportLink({ url, hash }, WebTransportClass) {
  const listeners = { message: [], close: [] };
  const emit = (type, event) => {
    for (const listen of listeners[type]) listen(event);
  };
  const value = Uint8Array.from(hash.slice("sha256:".length).match(/../g), (pair) => parseInt(pair, 16));
  const transport = new WebTransportClass(url, { serverCertificateHashes: [{ algorithm: "sha-256", value }] });
  const deframe = createDeframer(MAX_PROOF_MESSAGE);
  let writer;
  let closed = false;
  const end = () => {
    if (closed) return;
    closed = true;
    try {
      transport.close();
    } catch {
      // Already over.
    }
    emit("close", {});
  };
  const opened = (async () => {
    await transport.ready;
    const stream = await transport.createBidirectionalStream();
    writer = stream.writable.getWriter();
    (async () => {
      const reader = stream.readable.getReader();
      for (;;) {
        const { value: chunk, done } = await reader.read();
        if (done) break;
        for (const data of deframe(chunk)) emit("message", { data });
      }
    })().catch(() => {}).finally(end);
    transport.closed.catch(() => {}).finally(end);
  })();
  opened.catch(() => {
    closed = true;
    try {
      transport.close();
    } catch {
      // It never opened.
    }
  });
  return {
    opened,
    socket: {
      send: (body) => writer.write(frame(body)),
      close: end,
      proven: () => deframe.raiseLimit(MAX_MESSAGE),
      addEventListener: (type, listen) => listeners[type]?.push(listen),
    },
  };
}

function webSocketLink(url, WebSocketClass) {
  const socket = new WebSocketClass(url);
  socket.binaryType = "arraybuffer";
  const opened = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
  });
  return { opened, socket };
}

function within(promise, ms, message) {
  let timer;
  const limit = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), ms);
  });
  return Promise.race([promise, limit]).finally(() => clearTimeout(timer));
}

/**
 * WebTransport where the agent offers it, then its WebSocket. Trying the other on any failure to open gives nothing away: TLS
 * authenticates nothing here, and whichever opens has the same proof to pass.
 */
async function openLink(agent, { WebSocketClass, WebTransportClass }) {
  if (agent.webtransport && WebTransportClass) {
    const link = webTransportLink(agent.webtransport, WebTransportClass);
    try {
      await within(link.opened, WEBTRANSPORT_OPEN_MS, "WebTransport did not open");
      return link.socket;
    } catch {
      link.socket.close();
    }
  }
  const link = webSocketLink(agent.url, WebSocketClass);
  await within(link.opened, PROOF_MS, "the WebSocket did not open");
  return link.socket;
}

/** An agent the person added with no peer id: plain ACP in text frames, showing only what TLS shows of it. */
async function dialUnproven(agent, WebSocketClass) {
  const socket = new WebSocketClass(agent.url);
  let onMessage = () => {};
  let onClose = () => {};
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
  });
  socket.addEventListener("message", (event) => {
    if (typeof event.data === "string") onMessage(event.data);
  });
  socket.addEventListener("close", () => onClose());
  return {
    proven: false,
    send: async (message) => socket.send(message),
    onMessage(handler) {
      onMessage = handler;
    },
    onClose(handler) {
      onClose = handler;
    },
    close: () => socket.close(),
  };
}

/** Streams on a proven link. `heartbeat` asks now and then and notices silence, off unless set. */
function bareLink(proven, heartbeat) {
  const closing = [];
  const mux = createMux({
    send: (bytes) => proven.sendBytes(bytes),
    onMessage: (take) => proven.onBytes(take),
    onClose: (listen) => proven.onClose(() => {
      listen();
      for (const close of closing) close();
    }),
  // Nothing here accepts a stream the far side opens, so none is held for it.
  }, { side: "dialed", limits: { heartbeat, maxPendingOpens: 0 } });
  return {
    proven: true,
    mux,
    onClose(handler) {
      closing.push(handler);
    },
    close: () => proven.close(),
  };
}

/** ACP on one stream of a link whose other streams `mux` opens. Refused or ended, it closes as any channel does. */
async function acpOverLink(proven) {
  const { mux, onClose } = bareLink(proven);
  const stream = await mux.open("acp/1");
  return {
    proven: true,
    mux,
    send: (message) => stream.send(message),
    // Not read until there is someone to give it to.
    onMessage(handler) {
      (async () => {
        for (;;) {
          const message = await stream.receive();
          if (message === null) break;
          handler(new TextDecoder().decode(message));
        }
      })().catch(() => {}).finally(() => proven.close());
    },
    onClose,
    close: () => proven.close(),
  };
}

/** The method an agent proves by: one stored before agents named a method is WebCrypto's, the only method then. */
export function methodOf(agent) {
  return agent?.method === undefined ? "webcrypto" : agent.method;
}

/**
 * Why this browser cannot reach an agent of `method`, or null. It asks only the local runtime, before any connection, so nothing on
 * the network can change the answer; and a false answer never selects another method.
 */
export async function unreachable(method) {
  if (method === "webcrypto") return null;
  if (method === "noise") return (await noise.supported()) ? null : "this browser's WebCrypto has no X25519, which the Noise method needs";
  return "this browser cannot run that method";
}

async function localIdentity(method) {
  const why = await unreachable(method);
  if (why !== null) throw new Error(`unreachable from this browser: ${why}`);
  return MODULES[method].loadOrCreateIdentity(STORES[method]);
}

/**
 * This browser's own peer id under `method`, which an agent's machine allows and which names it in what `coqui-acp-bridge mcp-config`
 * prints; null where this browser cannot run the method.
 */
export async function ownPeerId(method = DEFAULT_METHOD) {
  if ((await unreachable(method)) !== null) return null;
  return (await localIdentity(method)).peerId;
}

/** Messages on a socket in order; `next` answers null once it closed. Throws on what no unproven peer may send. */
function queue(socket) {
  const inbox = [];
  let waiter;
  let closed = false;
  socket.addEventListener("message", (event) => {
    if ((event.data.byteLength ?? event.data.length) > MAX_PROOF_MESSAGE || inbox.length >= MAX_PROOF_WAITING) {
      socket.close();
    } else if (waiter !== undefined) {
      const take = waiter;
      waiter = undefined;
      take(event.data);
    } else {
      inbox.push(event.data);
    }
  });
  socket.addEventListener("close", () => {
    closed = true;
    waiter?.(null);
    waiter = undefined;
  });
  return () => new Promise((resolve) => {
    if (inbox.length > 0) resolve(inbox.shift());
    else if (closed) resolve(null);
    else waiter = resolve;
  });
}

/**
 * Pairs this browser's Noise key with the agent's machine at `url`, a `coqui-acp-bridge pair-accept --method noise`, by the PIN it
 * shows, and answers the key that machine proved. A wrong PIN is spent: never try it again, and ask for a new one.
 */
export async function pairWithPin({ url, pin }, { identity: given, WebSocketClass = globalThis.WebSocket } = {}) {
  const identity = given ?? (await localIdentity("noise"));
  const socket = new WebSocketClass(url);
  socket.binaryType = "arraybuffer";
  await within(new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
  }), PROOF_MS, "the WebSocket did not open");
  const next = queue(socket);
  const bytes = (data) => (data === null ? null : new Uint8Array(data));
  try {
    const near = await noise.createPairingInitiator(identity);
    socket.send(await near.hello());
    const answered = await within(next(), PROOF_MS, "no answer in time");
    if (answered === null) throw new Error("the connection closed during the proof");
    const { reply, pairing } = await near.answer(bytes(answered));
    socket.send(reply);
    const transport = { send: async (message) => socket.send(message), recv: async () => bytes(await next()) };
    const { peerId } = await within(pairing.join(String(pin), transport), PAIR_MS, "the pairing did not finish in time");
    return peerId;
  } finally {
    socket.close();
  }
}

/**
 * `link` runs ACP on a stream of a link, for an agent that says it serves one: anything else would read the first frame as ACP. `bare`
 * is such a link with nothing on it yet.
 */
export async function dialAgent(agent, {
  identity,
  link = false,
  bare = false,
  heartbeat,
  WebSocketClass = globalThis.WebSocket,
  WebTransportClass = globalThis.WebTransport,
} = {}) {
  if (typeof agent.peer !== "string") return dialUnproven(agent, WebSocketClass);
  const method = methodOf(agent);
  const local = identity ?? (await localIdentity(method));
  if (local.method !== method) throw new Error(`a ${local.method} key cannot prove a ${method} agent`);
  const proof = await MODULES[method].createInitiator(local, agent.peer);
  const socket = await openLink(agent, { WebSocketClass, WebTransportClass });
  const inbox = [];
  let waiter;
  let onMessage = () => {};
  let onBytes = (bytes) => onMessage(new TextDecoder().decode(bytes));
  let onClose = () => {};
  let session;
  let cut;
  let opening = Promise.resolve();
  socket.addEventListener("message", (event) => {
    if (session !== undefined) {
      // A record that does not open ends the session, and a plain message never reaches the scope.
      opening = opening.then(async () => {
        const message = await session.open(new Uint8Array(event.data));
        if (message !== null) onBytes(message);
      }).catch(() => socket.close());
    } else if ((event.data.byteLength ?? event.data.length) > MAX_PROOF_MESSAGE || inbox.length >= MAX_PROOF_WAITING) {
      socket.close();
    } else if (waiter !== undefined) {
      const take = waiter;
      waiter = undefined;
      take(event.data);
    } else {
      inbox.push(event.data);
    }
  });
  socket.addEventListener("close", () => {
    cut?.(new Error("the connection closed during the proof"));
    opening.finally(() => onClose());
  });
  const next = () => new Promise((resolve, reject) => {
    if (inbox.length > 0) {
      resolve(inbox.shift());
      return;
    }
    waiter = resolve;
    cut = reject;
  });
  try {
    const exchange = method === "noise"
      ? (async () => {
        await socket.send(await proof.hello());
        const proved = await proof.answer(new Uint8Array(await next()));
        await socket.send(proved.reply);
        ({ session } = proved);
      })()
      : (async () => {
        await socket.send(proof.hello());
        const answer = await proof.answer(asText(await next()));
        await socket.send(answer);
        ({ session } = await proof.finish(asText(await next())));
      })();
    await within(exchange, PROOF_MS, "no proof in time");
    socket.proven?.();
  } catch (error) {
    socket.close();
    throw error;
  }
  let sending = Promise.resolve();
  const enqueue = (sealed) => {
    sending = sending.then(async () => socket.send(await sealed)).catch(() => socket.close());
    return sending;
  };
  const proven = {
    proven: true,
    send: (message) => enqueue(session.seal(message)),
    sendBytes: (bytes) => enqueue(session.seal(bytes)),
    onBytes(handler) {
      onBytes = handler;
    },
    onMessage(handler) {
      onMessage = handler;
    },
    onClose(handler) {
      onClose = handler;
    },
    close() {
      enqueue(session.close()).finally(() => {
        socket.close();
        session.destroy();
      });
    },
  };
  if (bare) return bareLink(proven, heartbeat);
  return link ? acpOverLink(proven) : proven;
}
