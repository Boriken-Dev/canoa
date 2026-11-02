// A proven channel to an agent: the WebCrypto handshake on WebTransport or a WebSocket, then every message sealed. Its code ships
// with the extension, so nothing on the path can replace it.
import { createInitiator, loadOrCreateIdentity } from "./vendor/coqui-peer-webcrypto.mjs";

const STORE = "dev.boriken.canoa.peer";
const PROOF_MS = 15_000;
// A path that drops QUIC shows only as silence, and the WebSocket is still worth trying within the proof's time.
const WEBTRANSPORT_OPEN_MS = 5_000;
// What coqui-acp-bridge takes: a 16 MiB line and a record's overhead.
const MAX_MESSAGE = 16 * 1024 * 1024 + 64;

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
export function createDeframer(limit = MAX_MESSAGE) {
  let held = new Uint8Array(0);
  return (chunk) => {
    const joined = new Uint8Array(held.length + chunk.length);
    joined.set(held);
    joined.set(chunk, held.length);
    const messages = [];
    let at = 0;
    while (joined.length - at >= 4) {
      const length = new DataView(joined.buffer, joined.byteOffset + at, 4).getUint32(0, true);
      if (length === 0 || length > limit) throw new Error("a message of a length no peer may send");
      if (joined.length - at - 4 < length) break;
      messages.push(joined.slice(at + 4, at + 4 + length).buffer);
      at += 4 + length;
    }
    held = joined.slice(at);
    return messages;
  };
}

/** The same small surface as a WebSocket, so the proof does not care which it runs on. */
function webTransportLink({ url, hash }, WebTransportClass) {
  const listeners = { message: [], close: [] };
  const emit = (type, event) => {
    for (const listen of listeners[type]) listen(event);
  };
  const value = Uint8Array.from(hash.slice("sha256:".length).match(/../g), (pair) => parseInt(pair, 16));
  const transport = new WebTransportClass(url, { serverCertificateHashes: [{ algorithm: "sha-256", value }] });
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
    const deframe = createDeframer();
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

export async function dialAgent(agent, {
  identity,
  WebSocketClass = globalThis.WebSocket,
  WebTransportClass = globalThis.WebTransport,
} = {}) {
  if (typeof agent.peer !== "string") return dialUnproven(agent, WebSocketClass);
  const local = identity ?? (await loadOrCreateIdentity(STORE));
  const proof = await createInitiator(local, agent.peer);
  const socket = await openLink(agent, { WebSocketClass, WebTransportClass });
  const inbox = [];
  let waiter;
  let onMessage = () => {};
  let onClose = () => {};
  let session;
  let cut;
  let opening = Promise.resolve();
  socket.addEventListener("message", (event) => {
    if (session !== undefined) {
      // A record that does not open ends the session, and a plain message never reaches the scope.
      opening = opening.then(async () => {
        const message = await session.open(new Uint8Array(event.data));
        if (message !== null) onMessage(new TextDecoder().decode(message));
      }).catch(() => socket.close());
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
    const exchange = (async () => {
      await socket.send(proof.hello());
      const answer = await proof.answer(asText(await next()));
      await socket.send(answer);
      ({ session } = await proof.finish(asText(await next())));
    })();
    await within(exchange, PROOF_MS, "no proof in time");
  } catch (error) {
    socket.close();
    throw error;
  }
  let sending = Promise.resolve();
  const enqueue = (sealed) => {
    sending = sending.then(async () => socket.send(await sealed)).catch(() => socket.close());
    return sending;
  };
  return {
    proven: true,
    send: (message) => enqueue(session.seal(message)),
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
}
