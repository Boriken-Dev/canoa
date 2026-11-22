// What a page may do with a person's agents: grants, the scope on every message, and the connections. No chrome.* here, so the
// conformance checks in coqui-acp run against this very code. `origin` always comes from the browser, never from the page.
import { Scope } from "./vendor/coqui-acp/scope.mjs";
import { OFFER } from "./acp-tools.js";

const MAX_TEXT = 400;
// The key of a webpage's session folder in `_meta`, in a request (`{site}`) and in the host's answer to `session/new` (`{site, id, folder}`).
const SITE_META = "dev.boriken.canoa/site";
// How many sessions' folders a grant keeps for resuming, and the longest folder it keeps.
const MAX_FOLDERS = 256;
const MAX_FOLDER = 1024;
// Connections one origin may hold or be dialling at once: each dial is a native-host process and a proof, and a page costs the person
// nothing to ask for them.
const MAX_LINKS_PER_ORIGIN = 8;
// The host's own words, which a page is told none of: an answer to initialize carries this connection's approval token under the
// second, and a mode change the person allowed carries it back, so only this side can send one.
const APPROVAL_META = "dev.boriken.canoa/approval";
const OPEN_CATEGORIES = new Set(["model", "thought_level"]);
const MAX_MODE_TEXT = 100;
// Where the names of the modes of `session/set_mode` are kept among the names of options.
const MODE_KEY = "\0mode";
// Windows waiting to ask the person about one connection. Past this an agent is refused without asking: it has no way to make
// one person read more than one at a time, and a request of its own making costs it nothing.
const MAX_WAITING_PERMISSIONS = 8;
const refuse = (name, message) => Object.assign(new Error(message), { refusal: { name, message } });
const NOT_ALLOWED = () => refuse("NotAllowedError", "this page may not do that");

/** The same grant: one origin, one agent, and for a grant of lifetime `once` the one tab it belongs to. */
export function sameGrant(a, b) {
  return a.origin === b.origin && a.agent === b.agent && a.lifetime === b.lifetime && (a.lifetime !== "once" || a.tabId === b.tabId);
}

/**
 * Whether saving `grant` replaces `held`: the same origin and agent, unless both are `once` grants of different tabs, which each
 * tab holds for itself. An `always` grant serves every tab, so it replaces what a tab held, and the other way round.
 */
export function supersedes(grant, held) {
  return grant.origin === held.origin && grant.agent === held.agent && (grant.lifetime === "always" || held.lifetime === "always" || grant.tabId === held.tabId);
}

/** Whether a tab sees a grant: one of lifetime `once` belongs to the tab it was made in. */
export const seenBy = (grant, tabId) => grant.lifetime !== "once" || grant.tabId === tabId;

// Format characters can reorder what is shown around them, or hide part of it.
function text(value) {
  return typeof value === "string" ? value.replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, MAX_TEXT) : undefined;
}

/** Only what really is an origin: these are shown under a warning, so they must not be able to say anything else. */
function origins(value) {
  return list(value).filter((item) => {
    try {
      const url = new URL(item);
      return url.origin === item && url.protocol === "https:";
    } catch {
      return false;
    }
  });
}

/** The host name the browser reported for an origin, which is all of a page the host's folder is named for. */
function siteOf(origin) {
  try {
    const { hostname } = new URL(origin);
    return hostname === "" ? undefined : hostname;
  } catch {
    return undefined;
  }
}

/** The eight 16-bit groups of an IPv6 address written as text, or null when it is not one. */
function ipv6Groups(text) {
  if (!/^[0-9a-f:]+$/.test(text) || (text.match(/::/g) ?? []).length > 1) return null;
  const [head, tail] = text.split("::");
  const left = head === "" ? [] : head.split(":");
  const right = tail === undefined || tail === "" ? [] : tail.split(":");
  const missing = 8 - left.length - right.length;
  if (tail === undefined ? missing !== 0 : missing < 1) return null;
  const all = [...left, ...Array(tail === undefined ? 0 : missing).fill("0"), ...right];
  if (all.some((group) => !/^[0-9a-f]{1,4}$/.test(group))) return null;
  return all.map((group) => Number.parseInt(group, 16));
}

/**
 * Whether an address is this computer's: all of 127/8, 0.0.0.0, ::1, ::, localhost and what ends in .localhost, and a name or address
 * the host said this computer has (`names`, lower case, an IPv6 address bare), compared whole.
 */
export function hereAddress(hostname, names = []) {
  // A name ends in a dot when it is absolute: the same name.
  const host = String(hostname).toLowerCase().replace(/\.$/, "");
  if (host === "localhost" || host.endsWith(".localhost") || host === "[::1]" || host === "[::]") return true;
  if (names.includes(host.replace(/^\[|\]$/g, ""))) return true;
  const groups = host.startsWith("[") ? ipv6Groups(host.slice(1, -1)) : null;
  if (groups !== null) {
    // ::ffff:a.b.c.d and the deprecated ::a.b.c.d are an IPv4 address: this computer's when that is.
    const mapped = groups.slice(0, 5).every((group) => group === 0) && (groups[5] === 0xffff || groups[5] === 0);
    if (mapped && (groups[6] >> 8 === 127 || (groups[6] === 0 && groups[7] === 0))) return true;
  }
  const parts = host.split(".");
  if (parts.length !== 4 || parts.some((part) => !/^[0-9]{1,3}$/.test(part))) return false;
  const [a] = parts.map(Number);
  return a === 127 || host === "0.0.0.0";
}

/** An agent reached on this computer's own address. */
function loopbackAgent(agent, names) {
  try {
    return hereAddress(new URL(agent.url).hostname, names);
  } catch {
    return false;
  }
}

/** What the store says this computer is: names it has, and the keys of the Canoa the person paired with here (an array is names alone). */
async function hereOf(store) {
  const here = (await store.thisComputer?.()) ?? [];
  if (Array.isArray(here)) return { names: here, peers: [] };
  return { names: Array.isArray(here.names) ? here.names : [], peers: Array.isArray(here.peers) ? here.peers : [] };
}

/** An agent that runs on this computer as the person: the host's entries and what was found on a descriptor or a local address. */
function runsHere(agent, names, peers = []) {
  return agent.local === true || agent.source === "local" || (typeof agent.peer === "string" && peers.includes(agent.peer)) || loopbackAgent(agent, names);
}

function list(value) {
  return Array.isArray(value) ? value.filter((item) => typeof item === "string").map(text).slice(0, 16) : [];
}

async function hash(textValue) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(textValue));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * Whoever said an agent moved did not hold the key, or is not there, when that address fails: it goes back to where it last was. A proof
 * at the new address settles the move.
 */
export async function dialRecord(store, dial, agent, options) {
  try {
    const channel = await dial(agent, options);
    if (agent.previous) await store.settleMove(agent.key);
    return channel;
  } catch {
    if (!agent.previous) throw refuse("NetworkError", "the agent could not be reached or did not prove itself");
    try {
      const channel = await dial({ ...agent, ...agent.previous }, options);
      await store.revertMove(agent.key);
      return channel;
    } catch {
      throw refuse("NetworkError", "the agent could not be reached or did not prove itself");
    }
  }
}

/**
 * @param store agents({ stale }) (stale: also the local host's entries a grant holds while it is stopped), addAgent(agent), markProven(key), grants(origin), saveGrant(grant), deleteGrant(grant), secret()
 * @param dial (agent) => a proven channel: { send(text), onMessage(fn), onClose(fn), close() }
 * @param askPerson (details) => { allow: agent keys, add: new agents, cwd, permissions, lifetime } or null, in the provider's window
 * @param askPermission (details) => an option id, or null
 * @param askMode ({ origin, site, label, kind, option, value }) => true when the person allows that session's mode to change, in the provider's window
 * @param ownOrigin Canoa's own chat, the one origin whose sessions get the browser's tools. A page's never do: it could steer an agent
 *   into driving the browser.
 * @param offerTools ({ mux, peer, label }) => what gives a link's sessions those tools, from acp-tools.js
 */
export function createGateway({ store, dial, askPerson, askPermission, askMode = async () => false, ownOrigin, offerTools }) {
  const ports = new Set();
  const live = new Set();
  // One window per origin: a person's one click lasts a few seconds, and a page could otherwise open as many as it likes in them.
  const asking = new Set();
  // Dials in progress for each origin.
  const dialing = new Map();
  // Origins with a window open about a mode: one at a time, as for a request, so a page cannot raise window after window.
  const modeAsking = new Set();
  // Active Office pane tool names per (origin, tabId), set only by the extension worker from verified senders.
  const pageTools = new Map();
  const OFFICE_TOOLS = new Set(["outlook", "word", "excel"]);
  const pageToolsKey = (origin, tabId) => `${origin}\0${tabId}`;

  async function publicId(origin, key) {
    return hash(`${await store.secret()}\0${origin}\0${key}`);
  }

  /** The key of the Canoa on this computer is the same everywhere, so each site is told a name of its own for it. */
  async function peerFor(origin, agent) {
    if (agent.local !== true || origin === ownOrigin || typeof agent.peer !== "string") return agent.peer ?? null;
    return `sha256:${await hash(`${await store.secret()}\0${origin}\0peer\0${agent.peer}`)}`;
  }

  async function describe(origin, grant, agent) {
    return {
      id: await publicId(origin, agent.key),
      label: agent.label,
      source: agent.source,
      identity: {
        peer: await peerFor(origin, agent),
        verified: agent.proven === true,
        method: agent.proven === true ? (agent.method === "noise" ? "dev.boriken.coqui.peer-noise/v1" : "dev.boriken.coqui.peer-webcrypto/v1") : null,
      },
      offers: { acpVersions: agent.acpVersions ?? [], transports: agent.webtransport ? ["webtransport", "websocket"] : ["websocket"] },
      status: "unknown",
      scope: { permissions: grant.permissions === true, lifetime: grant.lifetime, mcpOrigins: grant.mcpOrigins ?? [] },
    };
  }

  /** What a tab holds of an origin's grants: those of lifetime `once` are the tab's own. */
  async function held(origin, tabId) {
    const agents = await store.agents({ stale: true });
    const pairs = [];
    for (const grant of await store.grants(origin)) {
      if (!seenBy(grant, tabId)) continue;
      const agent = agents.find((candidate) => candidate.key === grant.agent);
      if (agent === undefined) continue;
      // A grant on the host's entry is for the host whose key was paired when it was made: another Canoa under that name has none.
      if (agent.local === true && origin !== ownOrigin && grant.peer !== agent.peer) continue;
      pairs.push({ grant, agent });
    }
    return pairs;
  }

  async function find(origin, tabId, id) {
    for (const pair of await held(origin, tabId)) {
      if ((await publicId(origin, pair.agent.key)) === id) return pair;
    }
    throw NOT_ALLOWED();
  }

  function announce(origin) {
    for (const entry of ports) {
      if (entry.origin === origin) entry.port.postMessage({ event: "agentschange" });
    }
  }

  const operations = {
    async agents({ origin, tabId }) {
      const pairs = await held(origin, tabId);
      return Promise.all(pairs.map(({ grant, agent }) => describe(origin, grant, agent)));
    },

    async request({ origin, tabId, activated, leaving }, options = {}) {
      if (activated !== true || asking.has(origin) || leaving.signal.aborted) throw NOT_ALLOWED();
      asking.add(origin);
      let decision;
      try {
        const own = origin === ownOrigin;
        const known = await store.agents();
        // The window closes when the page goes: `signal` ends then.
        decision = await askPerson({
          origin,
          own,
          tabId,
          signal: leaving.signal,
          purpose: text(options.purpose),
          mcpOrigins: origins(options.mcpOrigins),
          peers: list(options.peers),
          permissions: options.permissions === true,
          agents: known.map(({ key, label, source, peer, proven, local }) => ({ key, label, source, peer, proven, local: local === true })),
        });
      } finally {
        asking.delete(origin);
      }
      // A document that has gone is granted nothing, whatever was decided for it.
      if (!decision || leaving.signal.aborted) throw NOT_ALLOWED();
      const own = origin === ownOrigin;
      const added = [];
      for (const agent of decision.add ?? []) added.push(await store.addAgent(agent));
      const keys = [...(decision.allow ?? []), ...added.map((agent) => agent.key)];
      const agents = await store.agents();
      const granted = [];
      const { names, peers } = await hereOf(store);
      for (const key of keys) {
        const agent = agents.find((candidate) => candidate.key === key);
        if (agent === undefined) continue;
        if (leaving.signal.aborted) throw NOT_ALLOWED();
        const earlier = (await store.grants(origin)).find((grant) => grant.agent === key && seenBy(grant, tabId));
        // The Canoa on this computer makes a page's folder itself, from the site; the person names none and the page cannot.
        const folder = agent.local === true && !own;
        // Agents on this computer, the host's entries and those found on a loopback address alike, run as the person.
        const onThisComputer = folder || (!own && runsHere(agent, names, peers));
        const grant = {
          origin,
          agent: key,
          tabId,
          lifetime: decision.lifetime === "always" ? "always" : "once",
          cwd: !folder && typeof decision.cwd === "string" ? decision.cwd : null,
          ...(folder ? { folder: "provider", peer: agent.peer } : {}),
          // The chat is where a person answers, so it needs no separate say. Nobody else can answer for the agents on this computer,
          // which run as the person: a site cannot approve what they do, or put them in a mode that does not ask.
          permissions: own || (decision.permissions === true && !onThisComputer),
          mcpOrigins: origins(decision.mcpOrigins),
          // A grant made again for another key starts empty: its sessions were another computer's.
          sessions: (folder && earlier?.peer !== agent.peer ? undefined : earlier?.sessions) ?? [],
          ...(folder ? { folders: (earlier?.peer === agent.peer ? earlier.folders : undefined) ?? {} } : {}),
        };
        // The terms the person has just chosen replace the old ones for connections already open too: they end here, so none
        // holds the old grant or can write it back.
        for (const link of [...live]) {
          if (supersedes(grant, link.grant)) finish(link, "revoked");
        }
        await store.saveGrant(grant);
        granted.push(await describe(origin, grant, agent));
      }
      if (granted.length === 0) throw NOT_ALLOWED();
      announce(origin);
      return granted;
    },

    async connect(entry, { id } = {}, call) {
      const { origin, port, tabId } = entry;
      const { grant, agent } = await find(origin, tabId, id);
      // A page's session with a local entry is placed by the host, in a folder for the site the browser reported. That is settled,
      // and the origin's share of connections is checked, before anything is dialled.
      const placedByHost = grant.folder === "provider" || (agent.local === true && origin !== ownOrigin);
      const site = placedByHost ? siteOf(origin) : grant.site;
      if (placedByHost && site === undefined) throw NOT_ALLOWED();
      const holding = [...live].filter((held) => held.grant.origin === origin).length + (dialing.get(origin) ?? 0);
      if (origin !== ownOrigin && holding >= MAX_LINKS_PER_ORIGIN) throw NOT_ALLOWED();
      // Counted until the connection is in `live`, or it failed: a dial is not free.
      dialing.set(origin, (dialing.get(origin) ?? 0) + 1);
      let counted = true;
      const release = () => {
        if (counted) dialing.set(origin, dialing.get(origin) - 1);
        counted = false;
      };
      let channel;
      try {
        return await connected();
      } catch (error) {
        try {
          channel?.close();
        } catch {
          // Already over.
        }
        throw error;
      } finally {
        release();
      }

      async function connected() {
      // Only where both hold: a page's connection has no streams at all, and an agent that names no link would read one as ACP.
      const options = { link: origin === ownOrigin && offerTools !== undefined && agent.offers?.includes(OFFER) === true };
      channel = await dialRecord(store, dial, agent, options);
      // The page stopped waiting: what was opened is closed, and nobody is told.
      if (call?.cancelled) throw NOT_ALLOWED();
      if (channel.proven) await store.markProven(agent.key);
      const sessions = new Set(grant.sessions ?? []);
      const folders = new Map(placedByHost ? Object.entries(grant.folders ?? {}) : []);
      // Whatever the stored grant says, a site cannot approve for the agents on this computer.
      const { names, peers } = await hereOf(store);
      // Canoa's own pages are the one grant that may pass more than a page does in a session request.
      const scope = new Scope({ ...grant, folder: placedByHost ? "provider" : undefined, own: origin === ownOrigin, permissions: origin !== ownOrigin && (placedByHost || runsHere(agent, names, peers)) ? false : grant.permissions, sessions }, origin);
      const conn = entry.nextConn++;
      const link = {
        conn, entry, channel, scope, grant, sessions, site, placedByHost, folders, label: agent.label, categories: new Map(), names: new Map(), approval: undefined, initId: undefined, denied: false,
        known: sessions.size, knownFolders: folders.size, waiting: 0, waits: new Set(), asking: Promise.resolve(), outgoing: Promise.resolve(),
      };
      if (channel.proven && channel.mux !== undefined) link.tools = offerTools({ mux: channel.mux, peer: agent.peer, label: agent.label });
      entry.links.set(conn, link);
      live.add(link);
      release();
      channel.onMessage((message) => {
        if (!live.has(link)) return;
        link.tools?.fromAgent(message);
        const step = scope.fromAgent(message);
        if (origin !== ownOrigin && step.forward !== undefined) step.forward = scrub(link, step.forward);
        saveSessions(link);
        if (step.forward !== undefined) port.postMessage({ conn, msg: step.forward });
        if (step.reply !== undefined) channel.send(step.reply);
        if (step.ask !== undefined) {
          // A request about no session of this page's has nothing to show, and one window at a time is all a person can read.
          if (!sessions.has(step.ask.params?.sessionId) || link.waiting >= MAX_WAITING_PERMISSIONS) {
            channel.send(scope.answerPermission(step.ask, null));
          } else {
            link.waiting += 1;
            // The page's `session/cancel` for this session ends the wait: the window closes and the agent is answered cancelled.
            const wait = { session: step.ask.params.sessionId, stop: new AbortController() };
            link.waits.add(wait);
            link.asking = link.asking
              .then(() => (live.has(link) && !wait.stop.signal.aborted ? askPermission({ origin, label: agent.label, request: step.ask, signal: wait.stop.signal }) : null))
              .catch(() => null)
              .then((option) => {
                link.waiting -= 1;
                link.waits.delete(wait);
                if (live.has(link)) channel.send(scope.answerPermission(step.ask, wait.stop.signal.aborted ? null : option));
              });
          }
        }
      });
      // "agent" when the agent ended it in order, "failed" when the link was cut with no close from it.
      channel.onClose(() => finish(link, channel.ending === "cut" ? "failed" : "agent"));
      let told;
      try {
        told = await describe(origin, grant, { ...agent, proven: channel.proven });
      } catch (error) {
        finish(link, "failed", false);
        throw error;
      }
      if (call?.cancelled) {
        finish(link, "page", false);
        throw NOT_ALLOWED();
      }
      // The agent can end the link between its proof and this answer (a key it turned away, once the proof finished): the close is
      // told by connection number, which the page has only from this answer, so it would be lost and the page would wait for ever.
      if (!live.has(link)) throw refuse("NetworkError", "the agent ended the connection right after the proof");
      return { conn, agent: told };
      }
    },

    async revoke({ origin, tabId }, { id } = {}) {
      let pair;
      try {
        pair = await find(origin, tabId, id);
      } catch {
        return null;
      }
      for (const link of [...live]) {
        if (link.grant.origin === origin && link.grant.agent === pair.agent.key) finish(link, "revoked");
      }
      await store.deleteGrant(pair.grant);
      announce(origin);
      return null;
    },
  };

  function saveSessions(link) {
    // A link that has ended has no grant to write back: the person may have just revoked it.
    if (!live.has(link) || (link.sessions.size === link.known && link.folders.size === link.knownFolders)) return;
    link.known = link.sessions.size;
    link.knownFolders = link.folders.size;
    link.grant.sessions = [...link.sessions];
    if (link.placedByHost) link.grant.folders = Object.fromEntries(link.folders);
    store.saveGrant(link.grant);
  }

  function finish(link, reason, notify = true) {
    if (!live.delete(link)) return;
    link.entry.links.delete(link.conn);
    link.tools?.end();
    link.channel.close();
    if (notify) link.entry.port.postMessage({ conn: link.conn, closed: reason });
  }

  /**
   * A session of the person's own Canoa runs in a scratch folder the host makes for `site`, never in one this side names. A page's
   * session is reopened in the folder the host reported for it, which only this side knows: the page never sees a path.
   */
  function inSite(link, text) {
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      return text;
    }
    if (!["session/new", "session/load", "session/resume"].includes(message?.method)) return text;
    // Canoa's own pages say so, which no page can: its sessions are the person's own, not a site's.
    const own = link.entry.origin === ownOrigin;
    const tools = own || link.entry.tabId === undefined ? undefined : pageTools.get(pageToolsKey(link.entry.origin, link.entry.tabId));
    const siteMeta = own ? { site: link.site, own: true } : tools?.length ? { site: link.site, tools } : { site: link.site };
    const params = { ...message.params, _meta: { [SITE_META]: siteMeta } };
    if (link.placedByHost && message.method !== "session/new") {
      const folder = link.folders.get(params.sessionId);
      if (typeof folder === "string") params.cwd = folder;
    }
    return JSON.stringify({ ...message, params });
  }

  /**
   * What the host says to a page's link passes through here: this side keeps what is for it (the folder of a new session, the
   * connection's approval token, which options are of which category) and the page is told none of the host's own words.
   */
  function scrub(link, text) {
    if (!text.includes("dev.boriken.") && !text.includes("configOptions") && !text.includes("availableModes")) return text;
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      return text;
    }
    for (const options of [message?.result?.configOptions, message?.params?.update?.configOptions]) {
      for (const option of Array.isArray(options) ? options : []) {
        if (typeof option?.id !== "string" || link.categories.size >= 256) continue;
        if (typeof option.category === "string") link.categories.set(option.id, option.category);
        const values = new Map();
        for (const choice of (Array.isArray(option.options) ? option.options : []).slice(0, 64)) {
          if (typeof choice?.value === "string" && typeof choice.name === "string") values.set(choice.value, choice.name);
        }
        link.names.set(option.id, { name: typeof option.name === "string" ? option.name : undefined, values });
      }
    }
    // The modes of the older shape: `modes.availableModes` of the answer to session/new.
    const modes = message?.result?.modes?.availableModes;
    if (Array.isArray(modes)) {
      const values = new Map();
      for (const mode of modes.slice(0, 64)) {
        if (typeof mode?.id === "string" && typeof mode.name === "string") values.set(mode.id, mode.name);
      }
      link.names.set(MODE_KEY, { name: "Mode", values });
    }
    const meta = message?.result?._meta;
    if (meta === null || typeof meta !== "object") return text;
    const ours = Object.keys(meta).filter((key) => key.startsWith("dev.boriken."));
    if (ours.length === 0) return text;
    // The token is taken from the answer to this side's own first initialize, once, and from nothing else.
    if (typeof meta[APPROVAL_META] === "string" && link.approval === undefined && link.initId !== undefined && message.id === link.initId) link.approval = meta[APPROVAL_META];
    const placed = meta[SITE_META];
    const session = message.result.sessionId;
    if (link.placedByHost && typeof session === "string" && typeof placed?.folder === "string" && placed.folder.length <= MAX_FOLDER) {
      link.folders.delete(session);
      link.folders.set(session, placed.folder);
      while (link.folders.size > MAX_FOLDERS) link.folders.delete(link.folders.keys().next().value);
    }
    for (const key of ours) delete meta[key];
    if (Object.keys(meta).length === 0) delete message.result._meta;
    return JSON.stringify(message);
  }

  /** A mode change by a page on a link the host places sessions on, or an option that could be one; null for any other message. */
  function modeChange(link, text) {
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      return null;
    }
    const params = message?.params;
    if (params === null || typeof params !== "object" || typeof params.sessionId !== "string" || !link.sessions.has(params.sessionId)) return null;
    if (message.method === "session/set_mode") return { id: message.id, method: message.method, params, kind: "mode", option: undefined, value: params.modeId };
    if (message.method !== "session/set_config_option") return null;
    // An option the agent never described is taken to be a mode's.
    if (OPEN_CATEGORIES.has(link.categories.get(params.configId))) return null;
    return { id: message.id, method: message.method, params, kind: "option", option: params.configId, value: params.value };
  }

  /**
   * The person decides a page's mode change, in the provider's window and not the page: Allow sends the change itself, marked with
   * the connection's token (which the page was never told and cannot send), for that session only and not remembered; anything
   * else, and no answer, is a refusal.
   */
  function askModeChange(link, change) {
    const refuse = () => link.entry.port.postMessage({
      conn: link.conn,
      msg: JSON.stringify({ jsonrpc: "2.0", id: change.id ?? null, error: { code: -32601, message: "the person did not allow that" } }),
    });
    const text = change.value;
    const origin = link.grant.origin;
    if (change.id === undefined || change.id === null) return;
    // No window for a value that is not text, where the host gave no token, after a Deny on this link or a "do not ask again" for
    // this site, or while another is open for this origin: a page that cannot make the person click cannot make windows either.
    if (typeof text !== "string" || text.length === 0 || text.length > MAX_MODE_TEXT || link.approval === undefined
      || link.denied || link.grant.noModes === true || modeAsking.has(origin)) {
      refuse();
      return;
    }
    modeAsking.add(origin);
    const known = link.names.get(change.kind === "mode" ? MODE_KEY : change.option);
    Promise.resolve()
      .then(() => (live.has(link) ? askMode({
        origin,
        site: link.site,
        label: link.label,
        kind: change.kind,
        option: typeof change.option === "string" ? change.option : undefined,
        optionName: known?.name,
        value: text,
        valueName: known?.values.get(text),
      }) : false))
      .catch(() => false)
      .then((answer) => {
        modeAsking.delete(origin);
        if (!live.has(link)) return;
        if (answer === "never") {
          link.grant.noModes = true;
          store.saveGrant(link.grant);
        }
        if (answer !== true) {
          link.denied = true;
          refuse();
          return;
        }
        const { sessionId, modeId, configId, value } = change.params;
        const params = change.method === "session/set_mode" ? { sessionId, modeId } : { sessionId, configId, value };
        forward(link, JSON.stringify({ jsonrpc: "2.0", id: change.id, method: change.method, params: { ...params, _meta: { [APPROVAL_META]: link.approval } } }));
      });
  }

  function fromPage(entry, message) {
    if (message === null || typeof message !== "object") return;
    if (message.conn !== undefined) {
      const link = entry.links.get(message.conn);
      if (link === undefined) return;
      if (message.op === "close") {
        finish(link, "page", false);
      } else if (typeof message.msg === "string") {
        if (link.initId === undefined && message.msg.includes("initialize")) {
          try {
            const sent = JSON.parse(message.msg);
            if (sent?.method === "initialize" && (typeof sent.id === "number" || typeof sent.id === "string")) link.initId = sent.id;
          } catch {
            // Not JSON: the scope refuses it below.
          }
        }
        const change = link.placedByHost ? modeChange(link, message.msg) : null;
        if (change !== null) {
          askModeChange(link, change);
          return;
        }
        const step = link.scope.fromPage(message.msg);
        const cancelled = step.cancelled;
        if (cancelled !== undefined) {
          for (const wait of link.waits) {
            if (wait.session === cancelled) wait.stop.abort();
          }
        }
        if (step.forward !== undefined) forward(link, link.site === undefined ? step.forward : inSite(link, step.forward));
        if (step.reply !== undefined) entry.port.postMessage({ conn: link.conn, msg: step.reply });
      }
      return;
    }
    // The page stopped waiting for a call: one still running is told, so it can close what it opened.
    if (typeof message.cancel === "number") {
      const call = entry.calls.get(message.cancel);
      if (call !== undefined) call.cancelled = true;
      return;
    }
    if (message.op === "ping" || typeof message.id !== "number") return;
    const operation = Object.hasOwn(operations, message.op) ? operations[message.op] : undefined;
    const call = { cancelled: false };
    entry.calls.set(message.id, call);
    const running = operation === undefined
      ? Promise.reject(NOT_ALLOWED())
      : operation({ ...entry, activated: message.activated === true }, message.args, call);
    running.finally(() => entry.calls.delete(message.id)).then(
      (result) => entry.port.postMessage({ id: message.id, result }),
      (error) => entry.port.postMessage({ id: message.id, error: error.refusal ?? { name: "NetworkError", message: "that failed" } }),
    );
  }

  /** In order, though adding a server to a session takes a moment. */
  function forward(link, text) {
    if (link.tools === undefined) {
      link.channel.send(text);
      return;
    }
    link.outgoing = link.outgoing
      .then(() => link.tools.declared(text))
      .then((declared) => (live.has(link) ? link.channel.send(declared) : undefined))
      .catch(() => finish(link, "failed"));
  }

  function closeLinks(matches) {
    const origins = new Set();
    for (const link of [...live]) {
      if (matches(link.grant)) {
        origins.add(link.grant.origin);
        finish(link, "revoked");
      }
    }
    return origins;
  }

  return {
    /** The person took a grant away, outside any page. Its connections end first, so none can write it back. */
    async revokeByPerson(grant) {
      closeLinks((held) => held.origin === grant.origin && held.agent === grant.agent);
      await store.deleteGrant(grant);
      announce(grant.origin);
    },

    /** The person removed an agent, and with it every site's grant. */
    async removeByPerson(key) {
      const origins = closeLinks((held) => held.agent === key);
      await store.removeAgent(key);
      for (const origin of origins) announce(origin);
    },

    /** Sets the Office providers allowed for sessions opened by a verified `(origin, tabId)` pane tab. */
    setPageTools(origin, tabId, apps) {
      if (origin === ownOrigin || typeof origin !== "string" || tabId === undefined) return;
      const allowed = [...new Set([...(apps ?? [])].filter((name) => OFFICE_TOOLS.has(name)))].sort();
      const key = pageToolsKey(origin, tabId);
      if (allowed.length === 0) pageTools.delete(key);
      else pageTools.set(key, allowed);
    },

    /** One document's port. `origin` and `tabId` are what the browser reported about its sender. */
    attach(port, { origin, tabId }) {
      // `leaving` ends with the document, which closes its request window.
      const entry = { port, origin, tabId, links: new Map(), nextConn: 1, calls: new Map(), leaving: new AbortController() };
      ports.add(entry);
      port.onMessage.addListener((message) => fromPage(entry, message));
      port.onDisconnect.addListener(() => {
        entry.leaving.abort();
        ports.delete(entry);
        for (const link of [...entry.links.values()]) finish(link, "navigation", false);
      });
    },
  };
}
