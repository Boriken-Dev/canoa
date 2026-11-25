// A proven channel to an agent: the handshake of the agent's method on WebTransport or a WebSocket, then every message sealed.
// Its code ships with the extension, so nothing on the path can replace it. The method is the one stored with the agent's key,
// never one a connection suggests.
import * as webcrypto from "./vendor/coqui-peer-webcrypto.mjs";
import * as noise from "./vendor/coqui-peer-noise.mjs";
import { createMux } from "./vendor/cabuya/mux.mjs";

/** The proof methods, in no order of preference. */
export const METHODS = Object.freeze(["noise", "webcrypto"]);
/** What a key made here, or an agent added with none named, proves by. */
export const DEFAULT_METHOD = "noise";
// Each method's key in a database of its own, so one key is never used by both.
const STORES = Object.freeze({ webcrypto: "dev.boriken.canoa.peer", noise: "dev.boriken.canoa.peer-noise" });
const MODULES = Object.freeze({ webcrypto, noise });
const PROOF_MS = 15_000;
/**
 * How long a Noise dial waits after its handshake for the agent to end the connection, which is how an agent turns a key away:
 * a Noise initiator finishes before the responder decides, so this is what lets a refused key fail the dial as a WebCrypto one does.
 */
export const NOISE_SETTLE_MS = 500;
/** How long a pairing may take by default, from its handshake to the end of the exchange: the bridge library's `--run-seconds` default of 120. */
export const PAIR_MS = 120_000;
// A path that drops QUIC shows only as silence, and the WebSocket is still worth trying within the proof's time.
const WEBTRANSPORT_OPEN_MS = 5_000;
// What the bridge library takes: a 16 MiB line, as a record of each method.
const MAX_LINE = 16 * 1024 * 1024;
const MAX_MESSAGE = maxRecord(MAX_LINE);
/** The `_meta` key of the host's answer to `initialize` that holds `[host] max_message_bytes`. */
export const MAX_MESSAGE_META = "dev.boriken.canoa/maxMessageBytes";

/** The longest record, under either method, that a line of `bytes` can become. */
export function maxRecord(bytes) {
  return Math.max(bytes + 64, recordLimit("noise", bytes));
}

/**
 * The longest message the host takes, `[host] max_message_bytes`, when `text` is its answer to `initialize` and names it; else null.
 * The answer is a response, so a message a peer wrote as a request that carries the key is not believed.
 */
export function hostLimit(text) {
  if (typeof text !== "string" || !text.includes(MAX_MESSAGE_META) || !text.includes("protocolVersion")) return null;
  try {
    const message = JSON.parse(text);
    const bytes = message.method === undefined ? message.result?._meta?.[MAX_MESSAGE_META] : undefined;
    return Number.isSafeInteger(bytes) && bytes > 0 ? bytes : null;
  } catch {
    return null;
  }
}

/**
 * The longest record a message of `n` bytes can become under `method`, whatever key limit its sender runs: a Noise sender may
 * change keys before any part, 19 bytes each, on top of `recordLength`'s 19 per part.
 */
export function recordLimit(method, n) {
  if (method !== "noise") return n + 17;
  return noise.recordLength(n) + 19 * Math.max(1, Math.ceil(n / 65_518));
}
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
      limit: (bytes) => deframe.raiseLimit(bytes),
      addEventListener: (type, listen) => listeners[type]?.push(listen),
    },
  };
}

/** The native messaging host's name: what `connectNative` is given to reach the Canoa on this computer. */
export const NATIVE_HOST = "dev.boriken.canoa";
/** The most one native message carries of a link's bytes, in either direction: what the relay takes, under the browser's 1 MiB. */
export const NATIVE_PIECE = 700_000;
const OPEN_MS = 10_000;

const toBase64 = (bytes) => {
  let text = "";
  for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  return btoa(text);
};
const fromBase64 = (text) => Uint8Array.from(atob(text), (character) => character.charCodeAt(0));

/**
 * The same small surface as a WebSocket, over the Canoa on this computer by way of the browser's native messaging host: a port
 * that opens `entry` (an entry of the host, `@control` or `@pair`) and then carries the link's bytes in base64 pieces, which it
 * frames and deframes as a stream. The host on the far side proves itself through it; the relay in between sees sealed records.
 */
function nativeLink({ entry, host = NATIVE_HOST }, connectNative) {
  const listeners = { message: [], close: [] };
  const emit = (type, event) => {
    for (const listen of listeners[type]) listen(event);
  };
  const deframe = createDeframer(MAX_PROOF_MESSAGE);
  let port;
  let closed = false;
  let settle;
  const opened = new Promise((resolve, reject) => {
    settle = { resolve, reject };
  });
  const end = (why) => {
    if (closed) return;
    closed = true;
    try {
      port.disconnect();
    } catch {
      // Already over.
    }
    settle.reject(new Error(why ?? "the native link closed"));
    emit("close", {});
  };
  try {
    port = connectNative(host);
  } catch (error) {
    closed = true;
    settle.reject(error);
    return { opened, socket: { send() {}, close() {}, proven() {}, addEventListener() {} } };
  }
  port.onMessage.addListener((message) => {
    if (message?.reply === "open") {
      if (message.opened === true) settle.resolve();
      else end(typeof message.error === "string" ? message.error : "the native host would not open that");
      return;
    }
    if (typeof message?.d === "string") {
      try {
        for (const data of deframe(fromBase64(message.d))) emit("message", { data });
      } catch {
        end("the native host sent what no host sends");
      }
    } else if (message?.closed === true) {
      end("the host closed the link");
    }
  });
  port.onDisconnect.addListener(() => end("the native host went away"));
  port.postMessage({ op: "open", entry });
  return {
    opened,
    socket: {
      send(body) {
        const framed = frame(body);
        for (let at = 0; at < framed.length; at += NATIVE_PIECE) {
          port.postMessage({ d: toBase64(framed.subarray(at, at + NATIVE_PIECE)) });
        }
      },
      close: () => end(),
      proven: () => deframe.raiseLimit(MAX_MESSAGE),
      limit: (bytes) => deframe.raiseLimit(bytes),
      addEventListener: (type, listen) => listeners[type]?.push(listen),
    },
  };
}

/** Opens the link to a local entry, with the time the open may take. */
async function openNative(native, connectNative) {
  if (typeof connectNative !== "function") throw new Error("this browser has no native messaging");
  const link = nativeLink(native, connectNative);
  try {
    await within(link.opened, OPEN_MS, "the native host did not open the link");
  } catch (error) {
    link.socket.close();
    throw error;
  }
  return link.socket;
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
async function openLink(agent, { WebSocketClass, WebTransportClass, connectNative }) {
  if (agent.native) return openNative(agent.native, connectNative);
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
  // "closed" when the socket closed in order, "cut" when it did not.
  let ending;
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
  });
  socket.addEventListener("message", (event) => {
    if (typeof event.data === "string") onMessage(event.data);
  });
  socket.addEventListener("close", (event) => {
    ending = event.wasClean === true ? "closed" : "cut";
    onClose();
  });
  return {
    proven: false,
    get ending() {
      return ending;
    },
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
  // A link can end before anyone asks to hear of it; whoever asks later is told at once.
  let over = false;
  const mux = createMux({
    send: (bytes) => proven.sendBytes(bytes),
    onMessage: (take) => proven.onBytes(take),
    onClose: (listen) => proven.onClose(() => {
      over = true;
      listen();
      for (const close of closing) close();
    }),
  // Nothing here accepts a stream the far side opens, so none is held for it.
  }, { side: "dialed", limits: { heartbeat, maxPendingOpens: 0 } });
  return {
    proven: true,
    mux,
    get ending() {
      return proven.ending;
    },
    onClose(handler) {
      if (over) queueMicrotask(handler);
      else closing.push(handler);
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
    get ending() {
      return proven.ending;
    },
    send: (message) => proven.refusal(message) ?? stream.send(message),
    // Not read until there is someone to give it to.
    onMessage(handler) {
      let followed = false;
      (async () => {
        for (;;) {
          const message = await stream.receive();
          if (message === null) break;
          const text = new TextDecoder().decode(message);
          const bytes = followed ? null : hostLimit(text);
          if (bytes !== null) {
            followed = true;
            proven.follow(bytes);
            mux.limits.maxMessage = bytes;
          }
          handler(text);
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
 * This browser's own peer id under `method`, which an agent's machine allows and which names it in what `canoa mcp-config`
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
 * Pairs this browser's Noise key with the agent's machine at `url`, a `canoa pair --method noise`, by the PIN it
 * shows, and answers the key that machine proved. A wrong PIN is spent: never try it again, and ask for a new one.
 */
export async function pairWithPin({ url, pin, native }, {
  identity: given,
  wait = PAIR_MS,
  WebSocketClass = globalThis.WebSocket,
  connectNative = globalThis.chrome?.runtime?.connectNative?.bind(globalThis.chrome.runtime),
} = {}) {
  const identity = given ?? (await localIdentity("noise"));
  let socket;
  if (native) {
    // The Canoa on this computer, by the native host: its pairing door, open only while the person asked for a PIN.
    socket = await openNative({ entry: "@pair" }, connectNative);
  } else {
    socket = new WebSocketClass(url);
    socket.binaryType = "arraybuffer";
    await within(new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
    }), PROOF_MS, "the WebSocket did not open");
  }
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
    const { peerId } = await within(pairing.join(String(pin), transport), wait, "the pairing did not finish in time");
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
  settle = NOISE_SETTLE_MS,
  WebSocketClass = globalThis.WebSocket,
  WebTransportClass = globalThis.WebTransport,
  connectNative = globalThis.chrome?.runtime?.connectNative?.bind(globalThis.chrome.runtime),
} = {}) {
  if (typeof agent.peer !== "string") return dialUnproven(agent, WebSocketClass);
  const method = methodOf(agent);
  const local = identity ?? (await localIdentity(method));
  if (local.method !== method) throw new Error(`a ${local.method} key cannot prove a ${method} agent`);
  const proof = await MODULES[method].createInitiator(local, agent.peer);
  const socket = await openLink(agent, { WebSocketClass, WebTransportClass, connectNative });
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
  // How the link ended: "closed" when the agent's close record came or this side ended it, "cut" when the transport just
  // stopped, which a relay can make look like an orderly end.
  let ending;
  let socketClosed = false;
  let ended = false;
  socket.addEventListener("close", () => {
    socketClosed = true;
    cut?.(new Error("the connection closed during the proof"));
    opening.finally(() => {
      if (ending === undefined && session !== undefined) {
        try {
          session.verifyEnd();
          ending = "closed";
        } catch {
          ending = "cut";
        }
      }
      ending ??= "cut";
      ended = true;
      onClose();
    });
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
    if (method === "noise" && settle > 0) {
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, settle);
        socket.addEventListener("close", () => {
          clearTimeout(timer);
          resolve();
        }, { once: true });
      });
    }
    if (socketClosed) throw new Error("the agent turned this browser's key away, or ended the connection, right after the proof");
    socket.proven?.();
  } catch (error) {
    socket.close();
    throw error;
  }
  let sending = Promise.resolve();
  // The host's own limit, once its answer to `initialize` has said it: a longer message is refused here, before it is sealed, so the
  // link and the session stay usable.
  let hostBytes = null;
  const follow = (bytes) => {
    hostBytes = bytes;
    socket.limit?.(maxRecord(bytes));
  };
  const refusal = (message) => {
    if (hostBytes === null || (typeof message === "string" ? message.length * 3 : message.length) <= hostBytes) return null;
    const size = typeof message === "string" ? new TextEncoder().encode(message).length : message.length;
    return size <= hostBytes
      ? null
      : Promise.reject(new Error(`a message of ${size} bytes is over the limit the host takes, ${hostBytes}: [host] max_message_bytes in its configuration`));
  };
  const enqueue = (sealed) => {
    sending = sending.then(async () => socket.send(await sealed)).catch(() => socket.close());
    return sending;
  };
  const proven = {
    proven: true,
    /** Once closed: "closed" in order, "cut" when the transport stopped without the agent's close record. */
    get ending() {
      return ending;
    },
    send: (message) => refusal(message) ?? enqueue(session.seal(message)),
    sendBytes: (bytes) => enqueue(session.seal(bytes)),
    follow,
    refusal,
    onBytes(handler) {
      onBytes = handler;
    },
    onMessage(handler) {
      let followed = false;
      onMessage = (text) => {
        const bytes = followed ? null : hostLimit(text);
        if (bytes !== null) {
          followed = true;
          follow(bytes);
        }
        handler(text);
      };
    },
    onClose(handler) {
      onClose = handler;
      if (ended) queueMicrotask(handler);
    },
    close() {
      ending ??= "closed";
      enqueue(session.close()).finally(() => {
        socket.close();
        session.destroy();
      });
    },
  };
  if (bare) return bareLink(proven, heartbeat);
  return link ? acpOverLink(proven) : proven;
}
