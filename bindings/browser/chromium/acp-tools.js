// Giving an agent the extension's tools over its link: for each session Canoa's own chat opens, a stdio server the bridge makes, which
// carries MCP's own messages back here on a stream of the same link. No chrome.* here, so it can be tested alone.
export const OFFER = "mcp-offer/1";
export const SERVER_NAME = "canoa-browser";

const OPENS_SESSION = new Set(["session/new", "session/load", "session/resume"]);
const MAX_RUNNING = 16;
const MAX_ENTRY_TEXT = 4096;
const LIST_CHANGED = JSON.stringify({ jsonrpc: "2.0", method: "notifications/tools/list_changed" });

const shortText = (value) => typeof value === "string" && value.length > 0 && value.length <= MAX_ENTRY_TEXT;

/** What the bridge answered, taken only in the shape of a stdio server. The bridge chooses the command, as it chose the agent. */
function readEntry(bytes) {
  const entry = JSON.parse(new TextDecoder().decode(bytes));
  const fine = entry?.name === SERVER_NAME && shortText(entry.command)
    && Array.isArray(entry.args) && entry.args.length <= 16 && entry.args.every(shortText)
    && Array.isArray(entry.env) && entry.env.length <= 16 && entry.env.every((item) => shortText(item?.name) && shortText(item?.value));
  if (!fine) throw new Error("not a server entry");
  return { name: entry.name, command: entry.command, args: entry.args, env: entry.env.map(({ name, value }) => ({ name, value })) };
}

const failure = (id, code, message) => JSON.stringify({ jsonrpc: "2.0", id: id ?? null, error: { code, message } });

/**
 * @param mux the link to the agent's bridge
 * @param handle (request) => the answer to one MCP message, or null for a notification
 */
export function createTools({ mux, handle }) {
  const streams = new Set();

  async function serve(stream) {
    let running = 0;
    const answer = async (text) => {
      let request;
      try {
        request = JSON.parse(text);
      } catch {
        return failure(null, -32700, "not JSON");
      }
      if (running >= MAX_RUNNING) return failure(request?.id, -32000, "too many calls at once");
      running += 1;
      try {
        const reply = await handle(request);
        return reply === null ? null : JSON.stringify(reply);
      } catch {
        return failure(request?.id, -32603, "that failed");
      } finally {
        running -= 1;
      }
    };
    try {
      for (;;) {
        const message = await stream.receive();
        if (message === null) break;
        // Not waited for: asking the person takes minutes, and an agent's pings and other calls go on meanwhile.
        answer(new TextDecoder().decode(message))
          .then((reply) => (reply === null ? undefined : stream.send(reply)))
          .catch(() => {});
      }
    } catch {
      // The stream ended: the link went, or the grant did.
    } finally {
      streams.delete(stream);
      stream.close();
    }
  }

  async function offer() {
    const stream = await mux.open(OFFER, new TextEncoder().encode(SERVER_NAME));
    try {
      const first = await stream.receive();
      if (first === null) throw new Error("the bridge ended the offer");
      const entry = readEntry(first);
      streams.add(stream);
      serve(stream);
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
      if (!OPENS_SESSION.has(message.method)) return text;
      let entry;
      try {
        entry = await offer();
      } catch {
        return text;
      }
      const servers = Array.isArray(message.params?.mcpServers) ? message.params.mcpServers : [];
      return JSON.stringify({ ...message, params: { ...message.params, mcpServers: [...servers, entry] } });
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
