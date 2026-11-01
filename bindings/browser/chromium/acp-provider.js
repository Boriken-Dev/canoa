// The gateway on Chrome's APIs: storage, the windows that ask the person, and getting the two scripts into a site the person allowed.
import { dialAgent } from "./acp-dial.js";
import { createGateway } from "./acp-gateway.js";

const AGENTS = "canoa_acp_agents";
const GRANTS = "canoa_acp_grants";
const ONCE = "canoa_acp_once";
const SECRET = "canoa_acp_secret";
const PORT_NAME = "canoa-acp";
const ASK_MS = 120_000;

const read = async (area, key, fallback) => (await chrome.storage[area].get(key))[key] ?? fallback;
const write = (area, key, value) => chrome.storage[area].set({ [key]: value });
const sameGrant = (a, b) => a.origin === b.origin && a.agent === b.agent;
const areaOf = (grant) => (grant.lifetime === "always" ? ["local", GRANTS] : ["session", ONCE]);

const store = {
  agents: () => read("local", AGENTS, []),
  async addAgent({ label, url, peer }) {
    const agent = { key: crypto.randomUUID(), label: String(label).slice(0, 80), url: String(url), source: "manual" };
    if (peer) agent.peer = String(peer);
    await write("local", AGENTS, [...(await store.agents()), agent]);
    return agent;
  },
  async removeAgent(key) {
    await write("local", AGENTS, (await store.agents()).filter((agent) => agent.key !== key));
    for (const [area, name] of [["local", GRANTS], ["session", ONCE]]) {
      await write(area, name, (await read(area, name, [])).filter((grant) => grant.agent !== key));
    }
  },
  async markProven(key) {
    const agents = await store.agents();
    await write("local", AGENTS, agents.map((agent) => (agent.key === key ? { ...agent, proven: true } : agent)));
  },
  async allGrants() {
    return [...(await read("local", GRANTS, [])), ...(await read("session", ONCE, []))];
  },
  grants: async (origin) => (await store.allGrants()).filter((grant) => grant.origin === origin),
  async saveGrant(grant) {
    await store.deleteGrant(grant);
    const [area, name] = areaOf(grant);
    await write(area, name, [...(await read(area, name, [])), grant]);
  },
  async deleteGrant(grant) {
    for (const [area, name] of [["local", GRANTS], ["session", ONCE]]) {
      await write(area, name, (await read(area, name, [])).filter((held) => !sameGrant(held, grant)));
    }
  },
  async secret() {
    let secret = await read("local", SECRET);
    if (secret === undefined) {
      secret = crypto.randomUUID() + crypto.randomUUID();
      await write("local", SECRET, secret);
    }
    return secret;
  },
};

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

const gateway = createGateway({
  store,
  dial: (agent) => dialAgent(agent),
  askPerson: (details) => ask({ kind: "grant", ...details }, 520, 680),
  askPermission: (details) => ask({ kind: "permission", ...details }, 480, 420),
});

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
  for (const grant of await store.grants(origin)) await store.deleteGrant(grant);
}

export function installProvider() {
  chrome.runtime.onConnect.addListener((port) => {
    if (port.name !== PORT_NAME) return;
    const { id, origin, frameId, tab } = port.sender ?? {};
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
    } else if (message.acp === "manage") {
      Promise.all([store.agents(), store.allGrants()]).then(([agents, grants]) => reply({ agents, grants }));
      return true;
    } else if (message.acp === "add") {
      store.addAgent(message.agent).then((agent) => reply(agent));
      return true;
    } else if (message.acp === "remove") {
      store.removeAgent(message.key).then(() => reply(true));
      return true;
    } else if (message.acp === "revoke") {
      store.deleteGrant(message.grant).then(() => reply(true));
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
