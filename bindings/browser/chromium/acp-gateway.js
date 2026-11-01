// What a page may do with a person's agents: grants, the scope on every message, and the connections. No chrome.* here, so the
// conformance checks in coqui-acp run against this very code. `origin` always comes from the browser, never from the page.
import { Scope } from "./vendor/coqui-acp/scope.mjs";

const MAX_TEXT = 400;
const refuse = (name, message) => Object.assign(new Error(message), { refusal: { name, message } });
const NOT_ALLOWED = () => refuse("NotAllowedError", "this page may not do that");

function text(value) {
  return typeof value === "string" ? value.slice(0, MAX_TEXT) : undefined;
}

function list(value) {
  return Array.isArray(value) ? value.filter((item) => typeof item === "string").map(text).slice(0, 16) : [];
}

async function hash(textValue) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(textValue));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * @param store agents(), addAgent(agent), markProven(key), grants(origin), saveGrant(grant), deleteGrant(grant), secret()
 * @param dial (agent) => a proven channel: { send(text), onMessage(fn), onClose(fn), close() }
 * @param askPerson (details) => { allow: agent keys, add: new agents, cwd, permissions, lifetime } or null, in the provider's window
 * @param askPermission (details) => an option id, or null
 */
export function createGateway({ store, dial, askPerson, askPermission }) {
  const ports = new Set();
  const live = new Set();

  async function publicId(origin, key) {
    return hash(`${await store.secret()}\0${origin}\0${key}`);
  }

  async function describe(origin, grant, agent) {
    return {
      id: await publicId(origin, agent.key),
      label: agent.label,
      source: agent.source,
      identity: {
        peer: agent.peer ?? null,
        verified: agent.proven === true,
        method: agent.proven === true ? "dev.boriken.coqui.peer-webcrypto/v1" : null,
      },
      offers: { acpVersions: agent.acpVersions ?? [], transports: agent.transports ?? ["websocket"] },
      status: "unknown",
      scope: { permissions: grant.permissions === true, lifetime: grant.lifetime, mcpOrigins: grant.mcpOrigins ?? [] },
    };
  }

  async function held(origin) {
    const agents = await store.agents();
    const pairs = [];
    for (const grant of await store.grants(origin)) {
      const agent = agents.find((candidate) => candidate.key === grant.agent);
      if (agent !== undefined) pairs.push({ grant, agent });
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
      if (activated !== true) throw NOT_ALLOWED();
      const known = await store.agents();
      const decision = await askPerson({
        origin,
        tabId,
        purpose: text(options.purpose),
        mcpOrigins: list(options.mcpOrigins),
        peers: list(options.peers),
        permissions: options.permissions === true,
        agents: known.map(({ key, label, source, peer, proven }) => ({ key, label, source, peer, proven })),
      });
      if (!decision) throw NOT_ALLOWED();
      const added = [];
      for (const agent of decision.add ?? []) added.push(await store.addAgent(agent));
      const keys = [...(decision.allow ?? []), ...added.map((agent) => agent.key)];
      const agents = await store.agents();
      const granted = [];
      for (const key of keys) {
        const agent = agents.find((candidate) => candidate.key === key);
        if (agent === undefined) continue;
        const earlier = (await store.grants(origin)).find((grant) => grant.agent === key);
        const grant = {
          origin,
          agent: key,
          tabId,
          lifetime: decision.lifetime === "always" ? "always" : "once",
          cwd: typeof decision.cwd === "string" ? decision.cwd : null,
          permissions: decision.permissions === true,
          mcpOrigins: list(decision.mcpOrigins),
          sessions: earlier?.sessions ?? [],
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
      let channel;
      try {
        channel = await dial(agent);
      } catch {
        throw refuse("NetworkError", "the agent could not be reached or did not prove itself");
      }
      if (channel.proven) await store.markProven(agent.key);
      const sessions = new Set(grant.sessions ?? []);
      const scope = new Scope({ ...grant, sessions }, origin);
      const conn = entry.nextConn++;
      const link = { conn, entry, channel, scope, grant, sessions, known: sessions.size };
      entry.links.set(conn, link);
      live.add(link);
      channel.onMessage((message) => {
        const step = scope.fromAgent(message);
        saveSessions(link);
        if (step.forward !== undefined) port.postMessage({ conn, msg: step.forward });
        if (step.reply !== undefined) channel.send(step.reply);
        if (step.ask !== undefined) {
          Promise.resolve(askPermission({ origin, label: agent.label, request: step.ask }))
            .catch(() => null)
            .then((option) => channel.send(scope.answerPermission(step.ask, option)));
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
    if (link.sessions.size === link.known) return;
    link.known = link.sessions.size;
    link.grant.sessions = [...link.sessions];
    store.saveGrant(link.grant);
  }

  function finish(link, reason, notify = true) {
    if (!live.delete(link)) return;
    link.entry.links.delete(link.conn);
    link.channel.close();
    if (notify) link.entry.port.postMessage({ conn: link.conn, closed: reason });
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
        if (step.forward !== undefined) link.channel.send(step.forward);
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

  return {
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
