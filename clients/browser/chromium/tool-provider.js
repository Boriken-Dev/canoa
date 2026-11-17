// The extension as a tool provider of the Canoa on this computer: one proven link to its `@tools` door, over which the host forwards
// each of its sessions' MCP messages, one stream per session. A session of the Canoa app, of the page it serves or of an Office pane
// is given the browser's tools without the extension seeing any ACP: it only answers what arrives on a stream, and every stream is its
// own principal, so what the person allows reaches that session and ends with it. Another provider (an Office add-in) registers the
// same way under another name. No chrome.* here, so it can be tested alone.

/** What this provider calls itself to the host; the host names the MCP server for it, `canoa-browser`. */
export const PROVIDER = "browser";

const STREAM = /^[0-9a-f]{32}$/;
const MAX_STREAMS = 256;
const MAX_RUNNING = 16;
const MAX_TEXT = 80;
const REGISTER_MS = 10_000;
const LIST_CHANGED = { jsonrpc: "2.0", method: "notifications/tools/list_changed" };

const text = (value) => (typeof value === "string" ? value.replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, MAX_TEXT) : "");
const failure = (id, code, message) => ({ jsonrpc: "2.0", id: id ?? null, error: { code, message } });

/** What the person is told of where an agent runs, from what the host said of the session: the host's claim, shown as such. */
export function whereOf(session) {
  if (session?.via === "paired") return "a program paired with Canoa on this computer";
  return "the Canoa app on this computer";
}

/**
 * @param dial () => the proven link to `@tools`: { send(text), onMessage(fn), onClose(fn), close() }
 * @param serve ({ principal, label, where, request, still }) => the answer to one MCP message, or null for none
 * @param forget (principal) => ends what the person gave that session: its grants, and a window still asking
 * @param onStatus (state) => told what to show: "connected", "off" or "unavailable"
 */
export function createToolProvider({ dial, serve, forget = () => {}, onStatus = () => {} }) {
  let live = null;
  let connecting = null;

  const principalOf = (stream) => `local:${stream}`;

  function end(entry, state) {
    if (entry.ended) return;
    entry.ended = true;
    if (live === entry) live = null;
    for (const stream of entry.streams.values()) forget(principalOf(stream.id));
    entry.streams.clear();
    try {
      entry.link.close();
    } catch {
      // Already over.
    }
    onStatus(state);
  }

  const send = (entry, message) => {
    if (entry.ended) return;
    try {
      Promise.resolve(entry.link.send(JSON.stringify(message))).catch(() => end(entry, "unavailable"));
    } catch {
      end(entry, "unavailable");
    }
  };

  function open(entry, message) {
    if (!STREAM.test(message.stream ?? "") || entry.streams.has(message.stream)) return;
    if (entry.streams.size >= MAX_STREAMS) {
      send(entry, { type: "close", stream: message.stream });
      return;
    }
    const session = message.session ?? {};
    entry.streams.set(message.stream, {
      id: message.stream,
      label: text(session.agent) || "An agent",
      where: whereOf(session),
      running: 0,
    });
  }

  function request(entry, message) {
    const stream = entry.streams.get(message.stream);
    const body = message.message;
    if (stream === undefined || body === null || typeof body !== "object" || Array.isArray(body)) return;
    if (stream.running >= MAX_RUNNING) {
      send(entry, { type: "mcp", stream: stream.id, message: failure(body.id, -32000, "too many calls at once") });
      return;
    }
    stream.running += 1;
    // Not waited for: asking the person takes minutes, and an agent's pings and other calls go on meanwhile.
    (async () => {
      try {
        const reply = await serve({
          principal: principalOf(stream.id),
          label: stream.label,
          where: stream.where,
          request: body,
          still: () => !entry.ended && entry.streams.get(stream.id) === stream,
        });
        if (reply !== null && reply !== undefined) send(entry, { type: "mcp", stream: stream.id, message: reply });
      } catch {
        send(entry, { type: "mcp", stream: stream.id, message: failure(body.id, -32603, "that failed") });
      } finally {
        stream.running -= 1;
      }
    })();
  }

  function heard(entry, raw) {
    let message;
    try {
      message = JSON.parse(raw);
    } catch {
      return;
    }
    if (message === null || typeof message !== "object") return;
    if (message.type === "open") open(entry, message);
    else if (message.type === "mcp") request(entry, message);
    else if (message.type === "close") {
      const stream = entry.streams.get(message.stream);
      if (stream !== undefined) {
        entry.streams.delete(message.stream);
        forget(principalOf(stream.id));
      }
    }
  }

  async function connect() {
    const link = await dial();
    const entry = { link, streams: new Map(), ended: false };
    let registered;
    let timer;
    const answer = new Promise((resolve, reject) => {
      registered = { resolve, reject };
      timer = setTimeout(() => reject(new Error("the host did not answer the registration")), REGISTER_MS);
    });
    answer.catch(() => {});
    link.onMessage((raw) => {
      if (!entry.registered) {
        try {
          const message = JSON.parse(raw);
          if (message?.type === "registered") {
            entry.registered = true;
            registered.resolve();
          } else {
            registered.reject(new Error(text(message?.why) || "the host refused the registration"));
          }
        } catch {
          registered.reject(new Error("the host answered something else"));
        }
        return;
      }
      heard(entry, raw);
    });
    link.onClose(() => {
      registered.reject(new Error("the link ended"));
      end(entry, "unavailable");
    });
    try {
      await link.send(JSON.stringify({ type: "register", provider: PROVIDER }));
      await answer;
    } catch (error) {
      end(entry, "unavailable");
      throw error;
    } finally {
      clearTimeout(timer);
    }
    live = entry;
    onStatus("connected");
    return entry;
  }

  return {
    /** Connects if it is not, once at a time. Rejects with why it could not, which a caller may leave for the next try. */
    ensure() {
      if (live !== null) return Promise.resolve(live);
      connecting ??= connect().finally(() => {
        connecting = null;
      });
      return connecting;
    },

    get connected() {
      return live !== null;
    },

    /** The principals of the sessions the host has open now. */
    principals() {
      return live === null ? [] : [...live.streams.values()].map((stream) => principalOf(stream.id));
    },

    /** Tells every session its tool list may have changed: a grant was given, ended or taken away. */
    changed() {
      if (live === null) return;
      for (const stream of live.streams.values()) send(live, { type: "mcp", stream: stream.id, message: LIST_CHANGED });
    },

    /** Leaves the host, as when the person unpairs. */
    stop() {
      if (live !== null) end(live, "off");
    },
  };
}
