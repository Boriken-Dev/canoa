// A proven channel to an agent: the WebCrypto handshake on a WebSocket, then every message sealed. Its code ships with the
// extension, so nothing on the path can replace it.
import { createInitiator, loadOrCreateIdentity } from "./vendor/coqui-peer-webcrypto.mjs";

const STORE = "dev.boriken.canoa.peer";
const PROOF_MS = 15_000;

const asText = (data) => (typeof data === "string" ? data : new TextDecoder().decode(data));

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

export async function dialAgent(agent, { identity, WebSocketClass = globalThis.WebSocket } = {}) {
  if (typeof agent.peer !== "string") return dialUnproven(agent, WebSocketClass);
  const local = identity ?? (await loadOrCreateIdentity(STORE));
  const proof = await createInitiator(local, agent.peer);
  const socket = new WebSocketClass(agent.url);
  socket.binaryType = "arraybuffer";
  const inbox = [];
  let waiter;
  let onMessage = () => {};
  let onClose = () => {};
  let session;
  let cut;
  let opening = Promise.resolve();
  const opened = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("the connection failed")), { once: true });
  });
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
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error("no proof in time")), PROOF_MS);
  });
  try {
    await Promise.race([opened, deadline]);
    socket.send(proof.hello());
    const answer = await proof.answer(asText(await Promise.race([next(), deadline])));
    socket.send(answer);
    ({ session } = await proof.finish(asText(await Promise.race([next(), deadline]))));
  } catch (error) {
    socket.close();
    throw error;
  } finally {
    clearTimeout(timer);
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
