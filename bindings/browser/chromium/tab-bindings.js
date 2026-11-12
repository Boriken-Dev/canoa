// The extension's record of which session is in which tab, and the messages the panel sends to change it and to share a tab.
// What each change means is in tab-sessions.js; this puts it on Chrome's storage, tabs and permissions.
import { readGrants, updateGrants } from "./browser-grants.js";
import { isPairedPeer } from "./agent-choices.js";
import { addShare, attach, detachSession, detachTab, endSession, fromUiPage, sessionsFor, shareGrant, sharedTabs, unshare } from "./tab-sessions.js";

const KEY = "canoa_tab_sessions";
const TYPES = new Set(["canoa.tabs.attach", "canoa.tabs.detach", "canoa.tabs.list", "canoa.tabs.share", "canoa.tabs.unshare", "canoa.tabs.sessionsFor"]);

async function readBindings() {
  const value = (await chrome.storage.session.get(KEY))[KEY];
  return Array.isArray(value) ? value : [];
}

let changes = Promise.resolve();
function updateBindings(change) {
  const done = changes.then(async () => {
    const updated = await change(await readBindings());
    await chrome.storage.session.set({ [KEY]: updated });
    return updated;
  });
  changes = done.catch(() => {});
  return done;
}

async function webTab(tabId) {
  if (!Number.isSafeInteger(tabId) || tabId < 0) throw new Error("Invalid tab");
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab?.url || !/^https?:\/\//.test(tab.url)) throw new Error("That tab cannot be shared");
  return { tab, origin: new URL(tab.url).origin };
}

async function handle(message, pairedPeers) {
  if (message.type === "canoa.tabs.attach") {
    const { origin } = await webTab(message.tabId);
    await updateBindings((bindings) => attach(bindings, { sessionId: message.sessionId, tabId: message.tabId, origin }));
    return {};
  }
  if (message.type === "canoa.tabs.detach") {
    // Without a session it would end every grant made some other way.
    if (typeof message.sessionId !== "string" || message.sessionId.length === 0 || message.sessionId.length > 128) throw new Error("Invalid session");
    await updateBindings((bindings) => detachSession(bindings, message.sessionId));
    await updateGrants((grants) => endSession(grants, message.sessionId));
    return {};
  }
  if (message.type === "canoa.tabs.list") {
    const grants = await readGrants();
    const bindings = await readBindings();
    return { sessions: bindings.map((binding) => ({ ...binding, shared: sharedTabs(grants, binding.sessionId) })) };
  }
  if (message.type === "canoa.tabs.sessionsFor") return { sessions: sessionsFor(await readBindings(), String(message.origin)) };
  if (message.type === "canoa.tabs.unshare") {
    await updateGrants((grants) => unshare(grants, { sessionId: message.sessionId, tabId: message.tabId }));
    return {};
  }
  // share: the person's click in the panel made the browser's own prompt for this site, so the access is there or this refuses.
  const { origin } = await webTab(message.tabId);
  if (!isPairedPeer(await pairedPeers(), message.peerId)) throw new Error("Invalid browser permission");
  if (!(await chrome.permissions.contains({ origins: [`${origin}/*`] }))) throw new Error("Browser host access was refused");
  const grant = shareGrant({ sessionId: message.sessionId, peerId: message.peerId, tabId: message.tabId, origin });
  await updateGrants((grants) => addShare(grants, grant));
  return {};
}

export function installTabBindings({ pairedPeers }) {
  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    if (!TYPES.has(message?.type) || !fromUiPage(sender, chrome.runtime)) return false;
    handle(message, pairedPeers).then(reply, (error) => reply({ error: String(error.message ?? error) }));
    return true;
  });
  chrome.tabs.onRemoved.addListener((tabId) => {
    updateBindings((bindings) => detachTab(bindings, tabId)).catch(() => {});
  });
  // A tab that goes to another site is no longer the one the session was attached to.
  chrome.tabs.onUpdated.addListener((tabId, changed) => {
    if (changed.status !== "loading" || changed.url === undefined) return;
    let origin;
    try {
      origin = new URL(changed.url).origin;
    } catch {
      origin = null;
    }
    updateBindings((bindings) => bindings.filter((binding) => binding.tabId !== tabId || binding.origin === origin)).catch(() => {});
  });
}
