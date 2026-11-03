// The gateway on Chrome's APIs: storage, the windows that ask the person, and getting the two scripts into a site the person allowed.
import { DEFAULT_METHOD, METHODS, dialAgent, methodOf, ownPeerId, pairWithPin, unreachable } from "./acp-dial.js";
import { COMPANION, offersOf, readCandidates } from "./acp-discovery.js";
import { createGateway, dialRecord } from "./acp-gateway.js";
import { createStanding, fromPermissionsPage } from "./acp-standing.js";
import { createTools, OFFER } from "./acp-tools.js";
import { dispatchBrowserTool } from "./browser-tools.js";
import { onGrantsChanged, readGrants, updateGrants } from "./browser-grants.js";
import { handleMcpRequest } from "./mcp.js";
import { askableCapabilities, createAccess } from "./mcp-access.js";
import { debuggedTabs, revokePeer } from "./permissions.js";
import { detachOnTabClosed } from "./devtools.js";

const AGENTS = "canoa_acp_agents";
const GRANTS = "canoa_acp_grants";
const ONCE = "canoa_acp_once";
const SECRET = "canoa_acp_secret";
const PORT_NAME = "canoa-acp";
const ASK_MS = 120_000;
const TOOL_MS = 30_000;
const SEARCH_SECONDS = 3;
const STANDING = "canoa_standing";
const STANDING_STATUS = "canoa_standing_status";
const STANDING_ALARM = "canoa-standing";
// Notices a link that has gone silent, which a socket alone may not for a long time.
const HEARTBEAT = { interval: 20_000, misses: 3 };
// A browser ends an idle worker after 30 seconds, and with it the socket. A WebSocket's traffic counts as activity and a WebTransport's
// does not, and an extension API call counts on either. Held only while the link is up, so nothing keeps a worker for a machine that is off.
const AWAKE_MS = 20_000;
// Wakes a worker that ended anyway and finds the link gone. 30 seconds is the least an alarm may be set for.
const ALARM_MINUTES = 0.5;
const FOUND = ["dns-sd", "local"];

const read = async (area, key, fallback) => (await chrome.storage[area].get(key))[key] ?? fallback;
const write = (area, key, value) => chrome.storage[area].set({ [key]: value });
const sameGrant = (a, b) => a.origin === b.origin && a.agent === b.agent;
const areaOf = (grant) => (grant.lifetime === "always" ? ["local", GRANTS] : ["session", ONCE]);

// Each change reads a whole list and writes it back, so two at once could bring back what the other removed.
let changes = Promise.resolve();
const oneAtATime = (change) => {
  const done = changes.then(change, change);
  changes = done.catch(() => {});
  return done;
};

async function replaceAgent(key, change) {
  const agents = await read("local", AGENTS, []);
  await write("local", AGENTS, agents.map((agent) => (agent.key === key ? change(agent) : agent)));
}

async function dropGrant(grant) {
  for (const [area, name] of [["local", GRANTS], ["session", ONCE]]) {
    await write(area, name, (await read(area, name, [])).filter((held) => !sameGrant(held, grant)));
  }
}

const store = {
  agents: () => read("local", AGENTS, []),
  addAgent: (details) => oneAtATime(() => addAgent(details)),
  removeAgent: (key) => oneAtATime(async () => {
    await write("local", AGENTS, (await store.agents()).filter((agent) => agent.key !== key));
    for (const [area, name] of [["local", GRANTS], ["session", ONCE]]) {
      await write(area, name, (await read(area, name, [])).filter((grant) => grant.agent !== key));
    }
  }),
  markProven: (key) => oneAtATime(() => replaceAgent(key, (agent) => ({ ...agent, proven: true }))),
  settleMove: (key) => oneAtATime(() => replaceAgent(key, ({ previous, ...agent }) => agent)),
  revertMove: (key) => oneAtATime(() => replaceAgent(key, ({ previous, ...agent }) => ({ ...agent, ...previous }))),
  async allGrants() {
    return [...(await read("local", GRANTS, [])), ...(await read("session", ONCE, []))];
  },
  grants: async (origin) => (await store.allGrants()).filter((grant) => grant.origin === origin),
  saveGrant: (grant) => oneAtATime(async () => {
    await dropGrant(grant);
    const [area, name] = areaOf(grant);
    await write(area, name, [...(await read(area, name, [])), grant]);
  }),
  deleteGrant: (grant) => oneAtATime(() => dropGrant(grant)),
  async secret() {
    let secret = await read("local", SECRET);
    if (secret === undefined) {
      secret = crypto.randomUUID() + crypto.randomUUID();
      await write("local", SECRET, secret);
    }
    return secret;
  },
};

async function addAgent({ label, url, peer, method, source, webtransport, offers }) {
    const agents = await store.agents();
    // The same key at a new address is the same agent, whose grants carry over and whose name stays the person's. Anyone can
    // name a public key, so nothing counts as shown until a proof there, and the old address is kept until then.
    const held = peer ? agents.find((agent) => agent.peer === peer && methodOf(agent) === (METHODS.includes(method) ? method : DEFAULT_METHOD)) : undefined;
    if (held !== undefined) {
      const previous = held.previous ?? { url: held.url, webtransport: held.webtransport, proven: held.proven };
      const moved = { ...held, url: String(url), webtransport, offers: offersOf({ offers }), proven: false, previous };
      await write("local", AGENTS, agents.map((agent) => (agent === held ? moved : agent)));
      return moved;
    }
    const agent = { key: crypto.randomUUID(), label: String(label).slice(0, 80), url: String(url), source: FOUND.includes(source) ? source : "manual" };
    if (peer) {
      agent.peer = String(peer);
      // Stored with the key and never changed: the proof runs by this method or not at all.
      agent.method = METHODS.includes(method) ? method : DEFAULT_METHOD;
    }
    if (webtransport) agent.webtransport = { url: String(webtransport.url), hash: String(webtransport.hash) };
    agent.offers = offersOf({ offers });
    await write("local", AGENTS, [...agents, agent]);
    return agent;
}

/** Asks the companion who is around. No companion is an answer too: the person can still add an agent by hand. */
async function nearby() {
  try {
    const reply = await chrome.runtime.sendNativeMessage(COMPANION, { op: "discover", seconds: SEARCH_SECONDS });
    return { candidates: readCandidates(reply, await store.agents()) };
  } catch {
    return { candidates: [], noCompanion: true };
  }
}

const asking = new Map();

/** Opens the provider's own window, where a page cannot draw over it. Closing it, or no answer in time, is a refusal. */
async function ask(details, width, height) {
  const id = crypto.randomUUID();
  return new Promise((resolve) => {
    const done = (answer) => {
      const entry = asking.get(id);
      if (entry === undefined) return;
      asking.delete(id);
      clearTimeout(entry.timer);
      if (entry.windowId !== undefined) chrome.windows.remove(entry.windowId).catch(() => {});
      resolve(answer);
    };
    asking.set(id, { details, done, timer: setTimeout(() => done(null), ASK_MS) });
    chrome.windows.create({ url: `acp-request.html?ask=${id}`, type: "popup", width, height }).then(
      (window) => {
        const entry = asking.get(id);
        if (entry !== undefined) entry.windowId = window.id;
      },
      () => done(null),
    );
  });
}

const OWN_ORIGIN = new URL(chrome.runtime.getURL("")).origin;
const debuggerBuild = () => chrome.runtime.getManifest().permissions.includes("debugger");
const requestAccess = createAccess({
  tabs: () => chrome.tabs.query({}),
  ask: async (details) => ask({ kind: "browser", ...details, programs: await programsShare(details.peer) }, 520, 620),
  updateGrants,
  debuggerBuild: debuggerBuild(),
});
const toolHosts = new Set();
onGrantsChanged(() => {
  for (const tools of toolHosts) tools.changed();
});

/** A page's own code runs in some tools, and a call to it that never ends would hold its place for ever. */
const within = (run, ms) => (details) => {
  let timer;
  const limit = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error("the page did not answer in time")), ms);
  });
  return Promise.race([run(details), limit]).finally(() => clearTimeout(timer));
};

/**
 * The browser's tools for one proven link. Each call is decided from the grants as they are then. `via` says a program with no session
 * is behind it, and `still` whether that is still turned on.
 */
function offerTools({ mux, peer, label, via, still }) {
  const capabilities = askableCapabilities(debuggerBuild());
  const tools = createTools({
    mux,
    handle: async (request) => handleMcpRequest({
      request,
      peerId: peer,
      trustedPeers: new Set([peer]),
      grants: await readGrants(),
      dispatch: within(dispatchBrowserTool, TOOL_MS),
      access: {
        capabilities,
        logs: via !== undefined,
        request: ({ capability, reason }) => requestAccess({ peer, label, capability, reason, via, still }),
      },
    }),
  });
  toolHosts.add(tools);
  return {
    ...tools,
    end(goodbye) {
      toolHosts.delete(tools);
      tools.end(goodbye);
    },
  };
}

const gateway = createGateway({
  store,
  dial: (agent, options) => dialAgent(agent, options),
  askPerson: (details) => ask({ kind: "grant", ...details }, 520, 680),
  askPermission: (details) => ask({ kind: "permission", ...details }, 480, 420),
  ownOrigin: OWN_ORIGIN,
  offerTools,
});

let awake;
function stayAwake(on) {
  if (on && awake === undefined) {
    awake = setInterval(() => {
      chrome.runtime.getPlatformInfo().catch(() => {});
    }, AWAKE_MS);
  } else if (!on && awake !== undefined) {
    clearInterval(awake);
    awake = undefined;
  }
}

const standing = createStanding({
  setting: () => read("local", STANDING, null),
  agentOf: async (key) => (await store.agents()).find((agent) => agent.key === key),
  dial: async (agent) => {
    const link = await dialRecord(store, dialAgent, agent, { bare: true, heartbeat: HEARTBEAT });
    await store.markProven(agent.key);
    return link;
  },
  toolsFor: ({ mux, agent }) => offerTools({
    mux,
    peer: agent.peer,
    label: agent.label,
    via: "program",
    still: async () => (await read("local", STANDING, null)) === agent.key,
  }),
  onStatus: (status) => {
    stayAwake(status.state === "connected");
    write("session", STANDING_STATUS, status).catch(() => {});
  },
});

/** What the permissions page shows: every agent that has a key to prove, whether it says it can do this, and how it is going. */
async function standingView() {
  const agents = await store.agents();
  const peer = await ownPeerId();
  const ids = Object.fromEntries(await Promise.all(METHODS.map(async (method) => [method, await ownPeerId(method)])));
  return {
    ids,
    on: await read("local", STANDING, null),
    status: await read("session", STANDING_STATUS, { state: "off" }),
    browser: peer,
    machines: agents.filter((agent) => typeof agent.peer === "string").map(({ key, label, peer: id, offers }) => ({ key, label, peer: id, serves: offers?.includes(OFFER) === true })),
  };
}

function closeWindowsAsking(peer) {
  for (const entry of asking.values()) {
    if (entry.details.kind === "browser" && entry.details.peer === peer) entry.done(null);
  }
}

/** Nothing a machine's agents were given before reaches its programs by a person turning this on, and nothing outlasts turning it off. */
async function forgetPeer(peer) {
  closeWindowsAsking(peer);
  let debugged = [];
  await updateGrants((grants) => {
    debugged = debuggedTabs(grants, peer);
    return revokePeer(grants, peer);
  });
  for (const tabId of debugged) await detachOnTabClosed(tabId).catch(() => {});
}

/** Turns it on for one agent, or off with null. */
async function setStanding(key) {
  const agents = await store.agents();
  const peerOf = (candidate) => agents.find((agent) => agent.key === candidate)?.peer;
  if (key !== null) {
    const agent = agents.find((candidate) => candidate.key === key);
    if (typeof agent?.peer !== "string" || agent.offers?.includes(OFFER) !== true) throw new Error("That machine cannot do this.");
  }
  const before = await read("local", STANDING, null);
  await write("local", STANDING, key);
  try {
    if (key === null) await chrome.alarms.clear(STANDING_ALARM);
    else await chrome.alarms.create(STANDING_ALARM, { periodInMinutes: ALARM_MINUTES });
    if (before !== key) {
      for (const peer of new Set([peerOf(before), peerOf(key)])) {
        if (peer !== undefined) await forgetPeer(peer);
      }
    }
  } finally {
    // Whatever failed above, the setting is written and the link follows it.
    await standing.changed();
    // A program may have got its question in while the link was ending.
    if (before !== null && before !== key && peerOf(before) !== undefined) closeWindowsAsking(peerOf(before));
  }
}

/** Whether what a person allows this peer also reaches its machine's programs. */
async function programsShare(peer) {
  const on = await read("local", STANDING, null);
  return on !== null && (await store.agents()).find((agent) => agent.key === on)?.peer === peer;
}

function secureOrigin(origin) {
  try {
    const url = new URL(origin);
    return url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname));
  } catch {
    return false;
  }
}

const scriptId = (origin, world) => `canoa-acp-${world}-${origin}`;

/** Puts the provider into a site, from now on and into its open tabs. Only after the person allowed that site. */
export async function allowSite(origin, tabId) {
  const matches = [`${origin}/*`];
  const scripts = [
    { id: scriptId(origin, "main"), matches, js: ["acp-page.js"], runAt: "document_start", world: "MAIN" },
    { id: scriptId(origin, "isolated"), matches, js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED" },
  ];
  const existing = await chrome.scripting.getRegisteredContentScripts({ ids: scripts.map((script) => script.id) });
  if (existing.length === 0) await chrome.scripting.registerContentScripts(scripts);
  if (tabId !== undefined) {
    await chrome.scripting.executeScript({ target: { tabId }, world: "MAIN", files: ["acp-page.js"] });
    await chrome.scripting.executeScript({ target: { tabId }, world: "ISOLATED", files: ["acp-content.js"] });
  }
}

export async function forgetSite(origin) {
  await chrome.scripting.unregisterContentScripts({ ids: [scriptId(origin, "main"), scriptId(origin, "isolated")] }).catch(() => {});
  for (const grant of await store.grants(origin)) await gateway.revokeByPerson(grant);
}

export function installProvider() {
  // Without this, a content script could read the agents' secret and turn on what only the person may.
  chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" }).catch(() => {});

  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === STANDING_ALARM) standing.ensure().catch(() => {});
  });
  // A browser that starts wakes this worker for this listener, and any wake-up looks at the setting just below.
  chrome.runtime.onStartup.addListener(() => {});
  // A restart can clear an alarm, and setting one again would put off the next by its whole period.
  read("local", STANDING, null).then(async (on) => {
    if (on !== null && (await chrome.alarms.get(STANDING_ALARM)) === undefined) await chrome.alarms.create(STANDING_ALARM, { periodInMinutes: ALARM_MINUTES });
  }).catch(() => {});
  standing.ensure().catch(() => {});

  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    if ((message?.type !== "canoa.standing.get" && message?.type !== "canoa.standing.set") || !fromPermissionsPage(sender, chrome.runtime)) return false;
    const answer = message.type === "canoa.standing.get"
      ? standingView()
      : setStanding(typeof message.agent === "string" ? message.agent : null).then(standingView);
    answer.then(reply, (error) => reply({ error: String(error.message ?? error) }));
    return true;
  });

  // Closing the window is a refusal, there and then.
  chrome.windows.onRemoved.addListener((windowId) => {
    for (const entry of asking.values()) {
      if (entry.windowId === windowId) entry.done(null);
    }
  });

  chrome.runtime.onConnect.addListener((port) => {
    if (port.name !== PORT_NAME) return;
    const { id, origin, frameId, tab, url } = port.sender ?? {};
    // Canoa's own chat, which a side panel shows in no tab. Only its pages: this origin is what gets the browser's tools.
    if (id === chrome.runtime.id && origin === OWN_ORIGIN && url?.startsWith(chrome.runtime.getURL("ui/"))) {
      gateway.attach(port, { origin, tabId: tab?.id });
      return;
    }
    // A top-level document of a secure origin, as the browser reports it. A cross-origin frame gets nothing in this version.
    if (id !== chrome.runtime.id || frameId !== 0 || tab?.id === undefined || !secureOrigin(origin)) {
      port.disconnect();
      return;
    }
    gateway.attach(port, { origin, tabId: tab.id });
  });

  chrome.tabs.onRemoved.addListener(async (tabId) => {
    const held = await read("session", ONCE, []);
    await write("session", ONCE, held.filter((grant) => grant.tabId !== tabId));
  });

  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    const ours = sender.id === chrome.runtime.id && sender.url?.startsWith(chrome.runtime.getURL("acp-request.html"));
    if (!ours || typeof message?.acp !== "string") return false;
    const entry = asking.get(message.ask);
    if (message.acp === "details") {
      reply(entry?.details ?? null);
    } else if (message.acp === "answer") {
      entry?.done(message.answer ?? null);
      reply(true);
    } else if (message.acp === "nearby") {
      nearby().then(reply);
      return true;
    } else if (message.acp === "manage") {
      Promise.all([store.agents(), store.allGrants()]).then(([agents, grants]) => reply({ agents, grants }));
      return true;
    } else if (message.acp === "add") {
      store.addAgent(message.agent).then((agent) => reply(agent));
      return true;
    } else if (message.acp === "pair") {
      // The person typed the PIN their agent's machine shows: pairing proves that machine's key, which the agent is added with.
      const { label, url, pairUrl, pin } = message.pairing ?? {};
      pairWithPin({ url: pairUrl, pin })
        .then((peer) => store.addAgent({ label, url, peer, method: "noise" }))
        .then((agent) => reply({ agent }), (error) => reply({ error: error.refusal ?? error.message }));
      return true;
    } else if (message.acp === "reachable") {
      Promise.all(METHODS.map(async (method) => [method, await unreachable(method)])).then((pairs) => reply(Object.fromEntries(pairs)));
      return true;
    } else if (message.acp === "remove") {
      read("local", STANDING, null)
        .then((on) => (on === message.key ? setStanding(null) : undefined))
        .then(() => gateway.removeByPerson(message.key))
        .then(() => reply(true));
      return true;
    } else if (message.acp === "revoke") {
      gateway.revokeByPerson(message.grant).then(() => reply(true));
      return true;
    }
    return false;
  });

  chrome.runtime.onInstalled.addListener(() => {
    chrome.contextMenus.create({ id: "canoa-acp-site", title: "Let this site ask for my agents", contexts: ["action"] });
    chrome.contextMenus.create({ id: "canoa-acp-agents", title: "Canoa agents", contexts: ["action"] });
  });
  chrome.contextMenus.onClicked.addListener((info, tab) => {
    if (info.menuItemId === "canoa-acp-agents") {
      chrome.tabs.create({ url: chrome.runtime.getURL("acp-request.html?manage=1") });
    } else if (info.menuItemId === "canoa-acp-site" && tab?.url && secureOrigin(new URL(tab.url).origin)) {
      const origin = new URL(tab.url).origin;
      // The click is the gesture Chrome needs for this, and the prompt is Chrome's own.
      chrome.permissions.request({ origins: [`${origin}/*`] }).then((granted) => {
        if (granted) allowSite(origin, tab.id);
      });
    }
  });
}
