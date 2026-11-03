// A paired machine's programs reaching the browser's tools with no ACP on the link: a person turns it on for one agent, and from then
// on this keeps a proven link there and one more `mcp-offer/1` stream open than are in use. Each program that comes gets a stream of
// its own. Pages have no way to this: it starts from a setting only the permissions page can change. No chrome.* here, so it can be
// tested alone.
import { OFFER } from "./acp-tools.js";

const METADATA = new TextEncoder().encode("standing/1");
const READY = "standing";
const CLAIMED = "claimed";
/** Programs at once on one machine. The mux holds 16 streams of one protocol, and asking for a tab goes one at a time anyway. */
export const MAX_PROGRAMS = 8;
/** How soon to try again after a link goes, while the worker is awake. Past these, the next wake-up tries. */
const RETRY_MS = [1_000, 5_000];
/** How long a goodbye has to cross before the link goes: what is still queued on a link that has gone is dropped there. */
const GOODBYE_MS = 250;

/** What a program hears when a person turns this off, before its server ends. */
export const GOODBYE = JSON.stringify({
  jsonrpc: "2.0",
  method: "notifications/message",
  params: { level: "warning", logger: "canoa-browser", data: "The person turned this off under Canoa browser permissions, so this server is ending." },
});

/** Only the extension's own permissions page says what this does: a page's scripts, and any other page of the extension, are refused. */
export function fromPermissionsPage(sender, runtime) {
  return sender?.id === runtime.id && sender.url === runtime.getURL("permissions.html");
}

function saying(bytes, word) {
  if (bytes === null) return false;
  try {
    return JSON.parse(new TextDecoder().decode(bytes))?.[word] !== undefined;
  } catch {
    return false;
  }
}

/**
 * @param setting () => the key of the agent this is on for, or null
 * @param agentOf (key) => that agent's record
 * @param dial (agent) => a proven link: { mux, onClose(fn), close() }, which refuses what cannot prove the key
 * @param toolsFor ({ mux, agent }) => what serves MCP on its streams: { adopt(stream, onEnd), end(goodbye) }
 * @param onStatus ({ state, peer }) => told what to show the person
 */
export function createStanding({ setting, agentOf, dial, toolsFor, onStatus = () => {}, setTimer = setTimeout }) {
  let live = null;
  let settling = null;
  let tries = 0;

  const say = (state, agent) => onStatus({ state, peer: agent?.peer ?? null });

  function retry() {
    if (tries < RETRY_MS.length) setTimer(() => ensure().catch(() => {}), RETRY_MS[tries]);
    tries += 1;
  }

  function lost(entry, state) {
    if (entry.ended) return;
    entry.ended = true;
    if (live === entry) live = null;
    entry.tools.end();
    entry.link.close();
    say(state, entry.agent);
    retry();
  }

  /** Waits for a program on one stream, then opens the next, so a stream is always free until the limit. */
  async function standBy(entry) {
    if (entry.ended || entry.waiting || entry.programs >= MAX_PROGRAMS) return;
    entry.waiting = true;
    let stream;
    try {
      stream = await entry.link.mux.open(OFFER, METADATA);
      entry.spare = stream;
      if (!saying(await stream.receive(), READY)) throw new Error("that is no standing stream");
      tries = 0;
      say("connected", entry.agent);
      if (!saying(await stream.receive(), CLAIMED)) throw new Error("the stream ended unused");
    } catch (error) {
      stream?.close();
      entry.waiting = false;
      lost(entry, error?.reason === "refused" ? "refused" : "unreachable");
      return;
    }
    entry.waiting = false;
    entry.spare = null;
    if (entry.ended) {
      stream.close();
      return;
    }
    entry.programs += 1;
    entry.tools.adopt(stream, () => {
      entry.programs -= 1;
      standBy(entry);
    });
    standBy(entry);
  }

  async function settle() {
    const wanted = await setting();
    if (live !== null && live.key !== wanted) {
      const entry = live;
      live = null;
      entry.ended = true;
      // First, so that nothing coming now can take it in the moment the goodbye has to cross.
      entry.spare?.close();
      entry.tools.end(GOODBYE);
      setTimer(() => entry.link.close(), GOODBYE_MS);
    }
    if (wanted === null) {
      tries = 0;
      say("off");
      return;
    }
    if (live !== null) return;
    const agent = await agentOf(wanted);
    if (typeof agent?.peer !== "string") {
      say("unreachable");
      return;
    }
    say("connecting", agent);
    let link;
    try {
      link = await dial(agent);
    } catch {
      say("unreachable", agent);
      retry();
      return;
    }
    const entry = { key: wanted, agent, link, tools: toolsFor({ mux: link.mux, agent }), programs: 0, waiting: false, spare: null, ended: false };
    live = entry;
    link.onClose(() => lost(entry, "unreachable"));
    standBy(entry);
  }

  /** Brings what runs here in line with the setting, one attempt at a time. */
  function ensure() {
    if (settling === null) {
      settling = settle().finally(() => {
        settling = null;
      });
    }
    return settling;
  }

  return {
    ensure,
    /** The setting has just changed: an attempt still running read the old one, so this waits for it and looks again. */
    changed: () => (settling ?? Promise.resolve()).catch(() => {}).then(ensure),
  };
}
