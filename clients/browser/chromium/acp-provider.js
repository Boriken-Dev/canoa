// The gateway on Chrome's APIs: storage, the windows that ask the person, and getting the two scripts into a site the person allowed.
import { METHODS, dialAgent, ownPeerId, pairWithPin, unreachable } from "./acp-dial.js";
import { createLocalHost, fromExtensionPage, fromOwnPage, localAgents, localGrants, rememberedEntries, stoppedAgents, viewOf } from "./acp-local.js";
import { addedMethod, lookNearby, mcpConfigCommand, offersOf, readCandidates, sameKey } from "./acp-discovery.js";
import { createGateway, dialRecord, sameGrant, supersedes } from "./acp-gateway.js";
import { createStanding, fromPermissionsPage } from "./acp-standing.js";
import { createTools, OFFER } from "./acp-tools.js";
import { createOfficeToolsBridge, secureOrigin, senderEndpoint } from "./office-tools-bridge.js";
import { createToolProvider } from "./tool-provider.js";
import { dispatchBrowserTool } from "./browser-tools.js";
import { markActing } from "./tab-marks.js";
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
const LOCAL_PIN = "canoa_local_host";
const LOCAL_STATE = "canoa_local_state";
const LOCAL_ENTRIES = "canoa_local_entries";
const CONTROL_MS = 30_000;
const STANDING = "canoa_standing";
const STANDING_STATUS = "canoa_standing_status";
const STANDING_ALARM = "canoa-standing";
const LOCAL_TOOLS_STATUS = "canoa_local_tools_status";
// While this browser is paired with Canoa on this computer: finds it again when it was stopped or started later.
const LOCAL_TOOLS_ALARM = "canoa-local-tools";
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

/** What saving a grant replaces (`replaces`), or what a revoke ends (`sameGrant`: the one grant, in its own tab if it is a `once`). */
async function dropGrants(drops) {
  for (const [area, name] of [["local", GRANTS], ["session", ONCE]]) {
    await write(area, name, (await read(area, name, [])).filter((held) => !drops(held)));
  }
}

const localHost = createLocalHost({
  connectNative: (name) => chrome.runtime.connectNative(name),
  lastError: () => chrome.runtime.lastError?.message,
});

const pinnedHost = () => read("local", LOCAL_PIN, null);
const lastLocal = () => read("session", LOCAL_STATE, { state: "unknown" });

/** Asks the native host what Canoa on this computer is doing, and remembers it for the lists that cannot wait for it. */
async function refreshLocal() {
  const state = await localHost.hello();
  const noted = { state: await noteLocal(state), pinned: await pinnedHost() };
  // A host that runs and holds this browser's key is offered the browser's tools, for every session it serves.
  if (state.state === "running" && viewOf(state, noted.pinned).paired) offerLocalTools();
  return noted;
}

/** Remembers a state the host was seen in, and what a paired host served, for a grant on one of its agents to outlast its being stopped. */
async function noteLocal(state) {
  await write("session", LOCAL_STATE, state);
  const kept = rememberedEntries(state, await pinnedHost(), null);
  if (kept !== null) await write("local", LOCAL_ENTRIES, kept);
  return state;
}

/** What the provider's window shows of Canoa on this computer: its state, and the agents a page could be given of it. */
async function localForWindow(state) {
  const pinned = await pinnedHost();
  const view = viewOf(state, pinned);
  return {
    view: { state: view.state, why: view.why, paired: view.paired, changed: view.changed, forbidden: view.forbidden },
    agents: localAgents(state, pinned).map(({ key, label, source, peer, proven }) => ({ key, label, source, peer, proven, local: true })),
  };
}

const store = {
  stored: () => read("local", AGENTS, []),
  // What this computer's Canoa serves is listed beside the agents the person added, once this browser's key is paired with it.
  agents: async ({ stale = false } = {}) => {
    const state = await lastLocal();
    const pinned = await pinnedHost();
    const live = localAgents(state, pinned);
    // A page's grant on one of its agents outlasts the host being stopped: it is listed, and connecting says it cannot be reached.
    const local = live.length > 0 || !stale ? live : stoppedAgents(state, pinned, await read("local", LOCAL_ENTRIES, null));
    return [...(await read("local", AGENTS, [])), ...local];
  },
  /** The names the running host says this computer has: what an agent there is held to, as one on a loopback address is. */
  thisComputer: async () => {
    const state = await lastLocal();
    const pinned = await pinnedHost();
    // The key the person paired with is this computer's Canoa, whether or not it runs now, and wherever its agents are reached.
    return { names: state.host?.thisComputer ?? state.thisComputer ?? [], peers: typeof pinned?.peer === "string" ? [pinned.peer] : [] };
  },
  addAgent: (details) => oneAtATime(() => addAgent(details)),
  removeAgent: (key) => oneAtATime(async () => {
    await write("local", AGENTS, (await store.stored()).filter((agent) => agent.key !== key));
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
  async grants(origin) {
    const held = (await store.allGrants()).filter((grant) => grant.origin === origin);
    if (origin !== OWN_ORIGIN) return held;
    // Canoa's own pages hold the local host's agents without asking, but keep their sessions in a grant like any other.
    const local = localGrants(origin, await lastLocal(), await pinnedHost());
    const kept = held.map((grant) => (local.find((item) => item.agent === grant.agent) === undefined ? grant : { ...grant, cwd: local[0].cwd, site: local[0].site, permissions: true }));
    return [...kept, ...local.filter((grant) => !held.some((item) => item.agent === grant.agent))];
  },
  saveGrant: (grant) => oneAtATime(async () => {
    await dropGrants((held) => supersedes(grant, held));
    const [area, name] = areaOf(grant);
    await write(area, name, [...(await read(area, name, [])), grant]);
  }),
  deleteGrant: (grant) => oneAtATime(() => dropGrants((held) => sameGrant(held, grant))),
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
    const agents = await store.stored();
    // The same key at a new address is the same agent, whose grants carry over and whose name stays the person's. Anyone can
    // name a public key, so nothing counts as shown until a proof there, and the old address is kept until then.
    const chosen = addedMethod(method);
    const held = peer ? sameKey(agents, peer, chosen) : undefined;
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
      agent.method = chosen;
    }
    if (webtransport) agent.webtransport = { url: String(webtransport.url), hash: String(webtransport.hash) };
    agent.offers = offersOf({ offers });
    await write("local", AGENTS, [...agents, agent]);
    return agent;
}

/** Runs one control method of Canoa on this computer, over a proven link of its own that lasts for the call. */
async function controlCall(method, params) {
  if (typeof method !== "string" || !/^[a-z_]{1,32}\/[a-z_]{1,32}$/.test(method)) throw new Error("not a control method");
  const { state, pinned } = await refreshLocal();
  const view = viewOf(state, pinned);
  if (!view.paired) throw new Error("Canoa on this computer is not paired with this browser");
  const link = await dialAgent({ native: { entry: "@control" }, peer: pinned.peer, method: pinned.method, label: "Canoa" }, {});
  try {
    const answered = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Canoa did not answer in time")), CONTROL_MS);
      link.onMessage((text) => {
        const message = JSON.parse(text);
        if (message.id !== 1) return;
        clearTimeout(timer);
        if (message.error) reject(Object.assign(new Error(String(message.error.message ?? "refused")), { code: message.error.code }));
        else resolve(message.result);
      });
      link.onClose(() => {
        clearTimeout(timer);
        reject(new Error("the link to Canoa ended"));
      });
    });
    await link.send(JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: params ?? {} }));
    return await answered;
  } finally {
    link.close();
  }
}

/** The messages the panel and the options page send about Canoa on this computer. Only the extension's own pages are heard. */
function installLocalHost() {
  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    const type = message?.type;
    if (typeof type !== "string" || !type.startsWith("canoa.host.")) return false;
    const heard = type === "canoa.host.status" ? fromExtensionPage(sender, chrome.runtime) : fromOwnPage(sender, chrome.runtime);
    if (!heard) return false;
    const view = async () => {
      const { state, pinned } = await refreshLocal();
      return viewOf(state, pinned);
    };
    let answer;
    if (type === "canoa.host.status") {
      answer = view();
    } else if (type === "canoa.host.start") {
      answer = localHost.start().then(async (state) => {
        await noteLocal(state);
        return viewOf(state, await pinnedHost());
      });
    } else if (type === "canoa.host.pair") {
      // The PIN the person read on their own computer, typed here: the key that proves itself through it is the one that is kept.
      answer = pairWithPin({ native: true, pin: String(message.pin ?? "") })
        .then(async (peer) => {
          await write("local", LOCAL_PIN, { peer, method: "noise" });
          await syncLocalAlarm().catch(() => {});
          return view();
        });
    } else if (type === "canoa.host.unpair") {
      // The host is asked to forget this key first, which is the real revoke; where it cannot be reached the pairing is forgotten
      // here all the same, and the person is told the host still lists the key.
      answer = (async () => {
        let notice;
        if ((await pinnedHost()) !== null) {
          try {
            await controlCall("peer/forget", {});
          } catch (error) {
            notice = `This browser forgot Canoa, but Canoa on this computer still lists its key (${String(error?.message ?? error)}). Remove it there with canoa revoke.`;
          }
        }
        await write("local", LOCAL_PIN, null);
        // Nothing of this browser stays offered to a Canoa it no longer trusts.
        localTools.stop();
        await syncLocalAlarm().catch(() => {});
        const after = await view();
        return notice === undefined ? after : { ...after, notice };
      })();
    } else if (type === "canoa.host.control") {
      answer = controlCall(message.method, message.params).then((result) => ({ result }));
    } else {
      return false;
    }
    answer.then(reply, (error) => reply({ error: String(error?.refusal ?? error?.message ?? error) }));
    return true;
  });
  // A worker that starts looks once, so the first list a panel asks for already knows.
  refreshLocal().catch(() => {});
}

/**
 * Asks Canoa on this computer who is around: its running host over this browser's paired link, else the native host itself, which
 * searches the same way and needs Canoa installed, not running. Not installed is an answer too: the person can still add an agent
 * by hand. Whatever comes back is a claim: nothing is ticked and the proof at connect decides.
 */
async function nearby() {
  const known = await store.stored();
  let paired = false;
  try {
    const { state, pinned } = await refreshLocal();
    paired = viewOf(state, pinned).paired;
  } catch {
    // Not reachable: the native host can still look.
  }
  return lookNearby({
    viaHost: paired ? () => controlCall("discover/nearby", { seconds: SEARCH_SECONDS }) : undefined,
    viaRelay: () => localHost.discover(SEARCH_SECONDS),
    known,
  });
}

const asking = new Map();

/** Opens the provider's own window, where a page cannot draw over it. Closing it, or no answer in time, is a refusal. */
async function ask(details, width, height, signal) {
  if (signal?.aborted) return null;
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
    // The page went, or answered the question another way: the window closes with no answer.
    signal?.addEventListener("abort", () => done(null), { once: true });
    chrome.windows.create({ url: `acp-request.html?ask=${id}`, type: "popup", width, height }).then(
      (window) => {
        const entry = asking.get(id);
        if (entry !== undefined) entry.windowId = window.id;
        // Answered or cancelled while it was opening.
        else chrome.windows.remove(window.id).catch(() => {});
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
  localTools.changed();
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
 * What answers one agent's MCP messages with the browser's tools: `peer` is the principal the grants are for (a key, or one session of
 * Canoa on this computer), and every call is decided from the grants as they are then. `via` says who asks, for the window, `where`
 * where it runs, and `still` whether what asked is still allowed to.
 */
function browserHandler({ peer, label, via, where, logs, still }) {
  const capabilities = askableCapabilities(debuggerBuild());
  return async (request) => handleMcpRequest({
    request,
    peerId: peer,
    trustedPeers: new Set([peer]),
    grants: await readGrants(),
    dispatch: within(async (call) => {
      const result = await dispatchBrowserTool(call);
      // A tool that ran on a tab puts it in the agent's group; a failure to do that never fails the tool.
      if (call.name !== "browser_list_tabs" && Number.isSafeInteger(call.args?.tabId)) await markActing({ tabId: call.args.tabId, peer, label }).catch(() => {});
      return result;
    }, TOOL_MS),
    access: {
      capabilities,
      logs,
      request: ({ capability, reason }) => requestAccess({ peer, label, capability, reason, via, where, still }),
    },
  });
}

/**
 * The browser's tools for one proven link. Each call is decided from the grants as they are then. `via` says a program with no session
 * is behind it, and `still` whether that is still turned on.
 */
function offerTools({ mux, peer, label, via, still }) {
  const tools = createTools({
    mux,
    handle: browserHandler({ peer, label, via, logs: via !== undefined, still }),
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
  askPerson: ({ signal, ...details }) => ask({ kind: "grant", ...details }, 520, 680, signal),
  askPermission: ({ signal, ...details }) => ask({ kind: "permission", ...details }, 480, 420, signal),
  askMode: async (details) => {
    const answer = await ask({ ...details, kind: "mode" }, 480, 460);
    return answer === true ? true : answer === "never" ? "never" : false;
  },
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
  agentOf: async (key) => (await store.stored()).find((agent) => agent.key === key),
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
  const agents = await store.stored();
  const ids = Object.fromEntries(await Promise.all(METHODS.map(async (method) => [method, await ownPeerId(method)])));
  return {
    ids,
    on: await read("local", STANDING, null),
    status: await read("session", STANDING_STATUS, { state: "off" }),
    machines: agents.filter((agent) => typeof agent.peer === "string").map(({ key, label, peer: id, method, offers }) => ({
      key, label, peer: id, method: method ?? "webcrypto", serves: offers?.includes(OFFER) === true, command: mcpConfigCommand(ids, method),
    })),
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

/**
 * The browser as a tool provider of Canoa on this computer: one proven link to its `@tools` door, over which the host sends each of its
 * sessions' MCP messages. Each session is a principal of its own, so a grant the person gives in the extension's window reaches that
 * session only and ends with it. A web page never reaches this: it is the host that adds the server to a session, and it adds none to
 * a session a page started.
 */
const localTools = createToolProvider({
  dial: async () => {
    const pinned = await pinnedHost();
    if (pinned === null) throw new Error("this browser is not paired with Canoa on this computer");
    return dialAgent({ native: { entry: "@tools" }, peer: pinned.peer, method: pinned.method, label: "Canoa" }, {});
  },
  serve: ({ principal, label, where, request, still }) => browserHandler({ peer: principal, label, via: "local", where, logs: false, still })(request),
  // A session ended, or the link did: what the person gave it goes, and a window still asking for it closes.
  forget: (principal) => forgetPeer(principal).catch(() => {}),
  onStatus: (state) => {
    write("session", LOCAL_TOOLS_STATUS, { state }).catch(() => {});
  },
});

/** Connects the provider if this browser is paired and it is not connected; a failure is for the next look, an alarm away. */
function offerLocalTools() {
  localTools.ensure().catch(() => {});
}

/** The alarm that looks again exists only while this browser is paired with Canoa on this computer. */
async function syncLocalAlarm() {
  if ((await pinnedHost()) === null) {
    await chrome.alarms.clear(LOCAL_TOOLS_ALARM);
  } else if ((await chrome.alarms.get(LOCAL_TOOLS_ALARM)) === undefined) {
    await chrome.alarms.create(LOCAL_TOOLS_ALARM, { periodInMinutes: ALARM_MINUTES });
  }
}

/** Turns it on for one agent, or off with null. */
async function setStanding(key) {
  const agents = await store.stored();
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
  return on !== null && (await store.stored()).find((agent) => agent.key === on)?.peer === peer;
}

const scriptId = (origin, world) => `canoa-acp-${world}-${origin}`;

/** Puts the provider into a site (top frame and matching subframes), from now on and into its open tabs. Only after the person allowed that site. */
export async function allowSite(origin, tabId, frameId) {
  const matches = [`${origin}/*`];
  const scripts = [
    { id: scriptId(origin, "main"), matches, js: ["acp-page.js"], runAt: "document_start", world: "MAIN", allFrames: true, matchOriginAsFallback: true },
    { id: scriptId(origin, "isolated"), matches, js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED", allFrames: true, matchOriginAsFallback: true },
  ];
  const existing = await chrome.scripting.getRegisteredContentScripts({ ids: scripts.map((script) => script.id) });
  if (existing.length === 0) await chrome.scripting.registerContentScripts(scripts);
  if (tabId !== undefined) {
    const target = frameId !== undefined ? { tabId, frameIds: [frameId] } : { tabId };
    await chrome.scripting.executeScript({ target, world: "MAIN", files: ["acp-page.js"] }).catch(() => {});
    await chrome.scripting.executeScript({ target, world: "ISOLATED", files: ["acp-content.js"] }).catch(() => {});
  }
}

export async function forgetSite(origin) {
  await chrome.scripting.unregisterContentScripts({ ids: [scriptId(origin, "main"), scriptId(origin, "isolated")] }).catch(() => {});
  for (const grant of await store.grants(origin)) await gateway.revokeByPerson(grant);
}

/** The keys of the agents this browser holds, for a grant to check its peer against. */
export async function pairedPeers() {
  return (await store.stored()).map((agent) => agent.peer).filter((peer) => typeof peer === "string");
}

export function installProvider() {
  installLocalHost();
  // Without this, a content script could read the agents' secret and turn on what only the person may.
  chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" }).catch(() => {});

  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === STANDING_ALARM) standing.ensure().catch(() => {});
    // Looks at the host, which connects the provider if it runs and holds this browser's key.
    if (alarm.name === LOCAL_TOOLS_ALARM && !localTools.connected) refreshLocal().catch(() => {});
  });
  syncLocalAlarm().catch(() => {});
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

  const officeToolsBridge = createOfficeToolsBridge({
    dialLocal: async () => {
      const pinned = await pinnedHost();
      if (pinned === null) throw new Error("this browser is not paired with Canoa on this computer");
      return dialAgent({ native: { entry: "@tools" }, peer: pinned.peer, method: pinned.method, label: "Canoa" }, {});
    },
    gateway,
    localWeb: async () => (await localHost.hello()).host?.web ?? null,
  });

  chrome.runtime.onConnect.addListener((port) => {
    if (port.name !== PORT_NAME) return;
    const endpoint = senderEndpoint(port.sender, {
      runtimeId: chrome.runtime.id,
      ownOrigin: OWN_ORIGIN,
      uiPrefix: chrome.runtime.getURL("ui/"),
    });
    if (endpoint === null) {
      port.disconnect();
      return;
    }
    if (endpoint.kind === "own") {
      gateway.attach(port, { origin: endpoint.origin, tabId: endpoint.tabId });
      return;
    }
    officeToolsBridge.attach(port, { origin: endpoint.origin, tabId: endpoint.tabId });
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
      Promise.all([store.stored(), store.allGrants()]).then(([agents, grants]) => reply({ agents, grants }));
      return true;
    } else if (message.acp === "add") {
      // A refusal, such as a known key named under another method, says why rather than leaving the window waiting.
      store.addAgent(message.agent).then((agent) => reply(agent), (error) => reply({ error: error.message }));
      return true;
    } else if (message.acp === "pair") {
      // The person typed the PIN their agent's machine shows: pairing proves that machine's key, which the agent is added with.
      const { label, url, pairUrl, pin } = message.pairing ?? {};
      pairWithPin({ url: pairUrl, pin })
        .then((peer) => store.addAgent({ label, url, peer, method: "noise" }))
        .then((agent) => reply({ agent }), (error) => reply({ error: error.refusal ?? error.message }));
      return true;
    } else if (message.acp === "host") {
      // What the window says about Canoa on this computer, asked fresh: the person may have started it since the page asked.
      refreshLocal().then(({ state }) => localForWindow(state)).then(reply, () => reply({ view: { state: "unavailable" }, agents: [] }));
      return true;
    } else if (message.acp === "start") {
      // The person's click in this window, which only the extension's own pages can send: a page cannot open it with its own script.
      localHost.start().then(noteLocal).then(localForWindow).then(reply, () => reply({ view: { state: "unavailable" }, agents: [] }));
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
    chrome.contextMenus.create({ id: "canoa-acp-site", title: "Let this site ask for my agents", contexts: ["action", "frame"] });
    chrome.contextMenus.create({ id: "canoa-acp-agents", title: "Canoa agents", contexts: ["action"] });
  });
  chrome.contextMenus.onClicked.addListener((info, tab) => {
    if (info.menuItemId === "canoa-acp-agents") {
      chrome.tabs.create({ url: chrome.runtime.getURL("acp-request.html?manage=1") });
    } else if (info.menuItemId === "canoa-acp-site" && tab?.id !== undefined) {
      const targetUrl = info.frameUrl ?? tab.url;
      let origin;
      try {
        origin = targetUrl ? new URL(targetUrl).origin : undefined;
      } catch {
        origin = undefined;
      }
      if (!origin || !secureOrigin(origin)) return;
      // The click is the gesture Chrome needs for this, and the prompt is Chrome's own.
      chrome.permissions.request({ origins: [`${origin}/*`] }).then((granted) => {
        if (granted) allowSite(origin, tab.id, info.frameId);
      });
    }
  });
}
