// Giving an agent the extension's tools over its link: for each session Canoa's own chat opens, a stdio server the bridge makes, which
// carries MCP's own messages back here on a stream of the same link. No chrome.* here, so it can be tested alone.
export const OFFER = "mcp-offer/1";
export const SERVER_NAME = "canoa-browser";

const OPENS_SESSION = new Set(["session/new", "session/load", "session/resume"]);
const MAX_RUNNING = 16;
const MAX_ENTRY_TEXT = 4096;
const LIST_CHANGED = JSON.stringify({ jsonrpc: "2.0", method: "notifications/tools/list_changed" });

const shortText = (value) => typeof value === "string" && value.length > 0 && value.length <= MAX_ENTRY_TEXT;
const CLOSES_SESSION = new Set(["session/close"]);

/** What the bridge answered, taken only in the shape of a stdio server. The bridge chooses the command, as it chose the agent. */
function readEntry(bytes) {
  const entry = JSON.parse(new TextDecoder().decode(bytes));
  const fine = entry?.type === "stdio" && entry.name === SERVER_NAME && shortText(entry.command)
    && Array.isArray(entry.args) && entry.args.length <= 16 && entry.args.every(shortText)
    && Array.isArray(entry.env) && entry.env.length <= 16 && entry.env.every((item) => shortText(item?.name) && shortText(item?.value));
  if (!fine) throw new Error("not a server entry");
  return { type: "stdio", name: entry.name, command: entry.command, args: entry.args, env: entry.env.map(({ name, value }) => ({ name, value })) };
}

const failure = (id, code, message) => JSON.stringify({ jsonrpc: "2.0", id: id ?? null, error: { code, message } });

/**
 * @param mux the link to the agent's bridge
 * @param handle (request) => the answer to one MCP message, or null for a notification
 */
export function createTools({ mux, handle }) {
  const streams = new Set();
  // A session's own stream, so ending the session ends what reached it. A new one's id comes in the agent's answer.
  const bySession = new Map();
  const opening = new Map();

  function serve(stream) {
    // Until its answer has gone: an agent that keeps asking and never reads must not grow what is waiting here.
    let running = 0;
    const answer = async (request) => {
      try {
        const reply = await handle(request);
        if (reply !== null) await stream.send(JSON.stringify(reply));
      } catch {
        await stream.send(failure(request?.id, -32603, "that failed"));
      }
    };
    const pump = async () => {
    try {
      for (;;) {
        const message = await stream.receive();
        if (message === null) break;
        let request;
        try {
          request = JSON.parse(new TextDecoder().decode(message));
        } catch {
          stream.trySend(failure(null, -32700, "not JSON"));
          continue;
        }
        if (running >= MAX_RUNNING) {
          // Without waiting, and lost if there is no room: this is the case of a peer not reading.
          stream.trySend(failure(request?.id, -32000, "too many calls at once"));
          continue;
        }
        running += 1;
        // Not waited for: asking the person takes minutes, and an agent's pings and other calls go on meanwhile.
        answer(request).catch(() => {}).finally(() => {
          running -= 1;
        });
      }
    } catch {
      // The stream ended: the link went, or the grant did.
    } finally {
      streams.delete(stream);
      stream.close();
    }
    };
    pump();
  }

  async function offer(message) {
    const stream = await mux.open(OFFER, new TextEncoder().encode(SERVER_NAME));
    try {
      const first = await stream.receive();
      if (first === null) throw new Error("the bridge ended the offer");
      const entry = readEntry(first);
      streams.add(stream);
      serve(stream);
      if (typeof message.params?.sessionId === "string") bySession.set(message.params.sessionId, stream);
      else if (message.id !== undefined) opening.set(message.id, stream);
      return entry;
    } catch (error) {
      stream.close();
      throw error;
    }
  }

  return {
    /** `text` as it goes to the agent: a message opening a session gets the server. Without one, the chat still works. */
    async declared(text) {
      const message = JSON.parse(text);
      if (CLOSES_SESSION.has(message.method)) {
        const held = bySession.get(message.params?.sessionId);
        bySession.delete(message.params?.sessionId);
        held?.close();
      }
      if (!OPENS_SESSION.has(message.method)) return text;
      let entry;
      try {
        entry = await offer(message);
      } catch {
        return text;
      }
      const servers = Array.isArray(message.params?.mcpServers) ? message.params.mcpServers : [];
      return JSON.stringify({ ...message, params: { ...message.params, mcpServers: [...servers, entry] } });
    },

    /** What the agent said, for the id of a session this opened. */
    fromAgent(text) {
      if (opening.size === 0) return;
      const message = JSON.parse(text);
      if (!opening.has(message.id) || message.method !== undefined) return;
      const stream = opening.get(message.id);
      opening.delete(message.id);
      if (typeof message.result?.sessionId === "string") bySession.set(message.result.sessionId, stream);
      else stream.close();
    },

    /** Tells every agent here to read its tool list again. Dropped for a stream that is full: its next call sees the truth anyway. */
    changed() {
      for (const stream of streams) {
        try {
          stream.trySend(LIST_CHANGED);
        } catch {
          // Already ending.
        }
      }
    },

    end() {
      for (const stream of [...streams]) stream.close();
      streams.clear();
    },
  };
}
