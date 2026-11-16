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
// Windows waiting to ask the person about one connection. Past this an agent is refused without asking: it has no way to make
// one person read more than one at a time, and a request of its own making costs it nothing.
const MAX_WAITING_PERMISSIONS = 8;
const refuse = (name, message) => Object.assign(new Error(message), { refusal: { name, message } });
const NOT_ALLOWED = () => refuse("NotAllowedError", "this page may not do that");

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
 * @param ownOrigin Canoa's own chat, the one origin whose sessions get the browser's tools. A page's never do: it could steer an agent
 *   into driving the browser.
 * @param offerTools ({ mux, peer, label }) => what gives a link's sessions those tools, from acp-tools.js
 */
export function createGateway({ store, dial, askPerson, askPermission, ownOrigin, offerTools }) {
  const ports = new Set();
  const live = new Set();
  // One window per origin: a person's one click lasts a few seconds, and a page could otherwise open as many as it likes in them.
  const asking = new Set();
  // Dials in progress for each origin.
  const dialing = new Map();

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

  async function held(origin) {
    const agents = await store.agents({ stale: true });
    const pairs = [];
    for (const grant of await store.grants(origin)) {
      const agent = agents.find((candidate) => candidate.key === grant.agent);
      if (agent === undefined) continue;
      // A grant on the host's entry is for the host whose key was paired when it was made: another Canoa under that name has none.
      if (agent.local === true && origin !== ownOrigin && grant.peer !== agent.peer) continue;
      pairs.push({ grant, agent });
    }
    return pairs;
  }

  async function find(origin, id) {
    for (const pair of await held(origin)) {
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
    async agents({ origin }) {
      const pairs = await held(origin);
      return Promise.all(pairs.map(({ grant, agent }) => describe(origin, grant, agent)));
    },

    async request({ origin, tabId, activated }, options = {}) {
      if (activated !== true || asking.has(origin)) throw NOT_ALLOWED();
      asking.add(origin);
      let decision;
      try {
        const own = origin === ownOrigin;
        const known = await store.agents();
        decision = await askPerson({
          origin,
          own,
          tabId,
          purpose: text(options.purpose),
          mcpOrigins: origins(options.mcpOrigins),
          peers: list(options.peers),
          permissions: options.permissions === true,
          agents: known.map(({ key, label, source, peer, proven, local }) => ({ key, label, source, peer, proven, local: local === true })),
        });
      } finally {
        asking.delete(origin);
      }
      if (!decision) throw NOT_ALLOWED();
      const own = origin === ownOrigin;
      const added = [];
      for (const agent of decision.add ?? []) added.push(await store.addAgent(agent));
      const keys = [...(decision.allow ?? []), ...added.map((agent) => agent.key)];
      const agents = await store.agents();
      const granted = [];
      for (const key of keys) {
        const agent = agents.find((candidate) => candidate.key === key);
        if (agent === undefined) continue;
        const earlier = (await store.grants(origin)).find((grant) => grant.agent === key);
        // The Canoa on this computer makes a page's folder itself, from the site; the person names none and the page cannot.
        const folder = agent.local === true && !own;
        const grant = {
          origin,
          agent: key,
          tabId,
          lifetime: decision.lifetime === "always" ? "always" : "once",
          cwd: !folder && typeof decision.cwd === "string" ? decision.cwd : null,
          ...(folder ? { folder: "provider", peer: agent.peer } : {}),
          // The chat is where a person answers, so it needs no separate say.
          // The chat is where a person answers, so it needs no separate say. Nobody else can answer for the agents on this computer,
          // which run as the person: a site cannot approve what they do, or put them in a mode that does not ask.
          permissions: own || (decision.permissions === true && !folder),
          mcpOrigins: origins(decision.mcpOrigins),
          sessions: earlier?.sessions ?? [],
          ...(folder ? { folders: earlier?.folders ?? {} } : {}),
        };
        await store.saveGrant(grant);
        granted.push(await describe(origin, grant, agent));
      }
      if (granted.length === 0) throw NOT_ALLOWED();
      announce(origin);
      return granted;
    },

    async connect(entry, { id } = {}) {
      const { origin, port } = entry;
      const { grant, agent } = await find(origin, id);
      // A page's session with a local entry is placed by the host, in a folder for the site the browser reported. That is settled,
      // and the origin's share of connections is checked, before anything is dialled.
      const placedByHost = grant.folder === "provider" || (agent.local === true && origin !== ownOrigin);
      const site = placedByHost ? siteOf(origin) : grant.site;
      if (placedByHost && site === undefined) throw NOT_ALLOWED();
      const holding = [...live].filter((held) => held.grant.origin === origin).length + (dialing.get(origin) ?? 0);
      if (holding >= MAX_LINKS_PER_ORIGIN) throw NOT_ALLOWED();
      dialing.set(origin, (dialing.get(origin) ?? 0) + 1);
      // Only where both hold: a page's connection has no streams at all, and an agent that names no link would read one as ACP.
      const options = { link: origin === ownOrigin && offerTools !== undefined && agent.offers?.includes(OFFER) === true };
      let channel;
      try {
        channel = await dialRecord(store, dial, agent, options);
      } finally {
        dialing.set(origin, dialing.get(origin) - 1);
      }
      if (channel.proven) await store.markProven(agent.key);
      const sessions = new Set(grant.sessions ?? []);
      const folders = new Map(placedByHost ? Object.entries(grant.folders ?? {}) : []);
      // Whatever the stored grant says, a site cannot approve for the agents on this computer.
      const scope = new Scope({ ...grant, folder: placedByHost ? "provider" : undefined, permissions: placedByHost ? false : grant.permissions, sessions }, origin);
      const conn = entry.nextConn++;
      const link = {
        conn, entry, channel, scope, grant, sessions, site, placedByHost, folders,
        known: sessions.size, knownFolders: folders.size, waiting: 0, asking: Promise.resolve(), outgoing: Promise.resolve(),
      };
      if (channel.proven && channel.mux !== undefined) link.tools = offerTools({ mux: channel.mux, peer: agent.peer, label: agent.label });
      entry.links.set(conn, link);
      live.add(link);
      channel.onMessage((message) => {
        if (!live.has(link)) return;
        link.tools?.fromAgent(message);
        const step = scope.fromAgent(message);
        if (placedByHost && step.forward !== undefined) step.forward = learnFolder(link, step.forward);
        saveSessions(link);
        if (step.forward !== undefined) port.postMessage({ conn, msg: step.forward });
        if (step.reply !== undefined) channel.send(step.reply);
        if (step.ask !== undefined) {
          // A request about no session of this page's has nothing to show, and one window at a time is all a person can read.
          if (!sessions.has(step.ask.params?.sessionId) || link.waiting >= MAX_WAITING_PERMISSIONS) {
            channel.send(scope.answerPermission(step.ask, null));
          } else {
            link.waiting += 1;
            link.asking = link.asking
              .then(() => (live.has(link) ? askPermission({ origin, label: agent.label, request: step.ask }) : null))
              .catch(() => null)
              .then((option) => {
                link.waiting -= 1;
                if (live.has(link)) channel.send(scope.answerPermission(step.ask, option));
              });
          }
        }
      });
      channel.onClose(() => finish(link, "agent"));
      return { conn, agent: await describe(origin, grant, { ...agent, proven: channel.proven }) };
    },

    async revoke({ origin }, { id } = {}) {
      let pair;
      try {
        pair = await find(origin, id);
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
    const params = { ...message.params, _meta: { [SITE_META]: { site: link.site } } };
    if (link.placedByHost && message.method !== "session/new") {
      const folder = link.folders.get(params.sessionId);
      if (typeof folder === "string") params.cwd = folder;
    }
    return JSON.stringify({ ...message, params });
  }

  /** Keeps the folder the host made for a new session, and takes it out of what the page is told. */
  function learnFolder(link, text) {
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      return text;
    }
    const meta = message?.result?._meta;
    if (meta === null || typeof meta !== "object") return text;
    const placed = meta[SITE_META];
    // What the host reports about itself (its folder, its profile) is for this side; the page is told none of it.
    const ours = Object.keys(meta).filter((key) => key.startsWith("dev.boriken."));
    if (ours.length === 0) return text;
    const session = message.result.sessionId;
    if (typeof session === "string" && typeof placed?.folder === "string" && placed.folder.length <= MAX_FOLDER) {
      link.folders.delete(session);
      link.folders.set(session, placed.folder);
      while (link.folders.size > MAX_FOLDERS) link.folders.delete(link.folders.keys().next().value);
    }
    for (const key of ours) delete meta[key];
    if (Object.keys(meta).length === 0) delete message.result._meta;
    return JSON.stringify(message);
  }

  function fromPage(entry, message) {
    if (message === null || typeof message !== "object") return;
    if (message.conn !== undefined) {
      const link = entry.links.get(message.conn);
      if (link === undefined) return;
      if (message.op === "close") {
        finish(link, "page", false);
      } else if (typeof message.msg === "string") {
        const step = link.scope.fromPage(message.msg);
        if (step.forward !== undefined) forward(link, link.site === undefined ? step.forward : inSite(link, step.forward));
        if (step.reply !== undefined) entry.port.postMessage({ conn: link.conn, msg: step.reply });
      }
      return;
    }
    if (message.op === "ping" || typeof message.id !== "number") return;
    const operation = Object.hasOwn(operations, message.op) ? operations[message.op] : undefined;
    const running = operation === undefined
      ? Promise.reject(NOT_ALLOWED())
      : operation({ ...entry, activated: message.activated === true }, message.args);
    running.then(
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
      .catch(() => finish(link, "agent"));
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

    /** One document's port. `origin` and `tabId` are what the browser reported about its sender. */
    attach(port, { origin, tabId }) {
      const entry = { port, origin, tabId, links: new Map(), nextConn: 1 };
      ports.add(entry);
      port.onMessage.addListener((message) => fromPage(entry, message));
      port.onDisconnect.addListener(() => {
        ports.delete(entry);
        for (const link of [...entry.links.values()]) finish(link, "navigation", false);
      });
    },
  };
}
