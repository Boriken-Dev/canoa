import {
  DIRECTION, CAPABILITY, isAllowed, makeGrant, revokePeer, revokeTab, sameGrant
} from "./permissions.js";
import { captureContext, addContext, takeContext } from "./context-transfer.js";
import { detachOnTabClosed } from "./devtools.js";
import { installProvider } from "./acp-provider.js";
import { readGrants, updateGrants } from "./browser-grants.js";

installProvider();

// Browser-owned, deny-by-default policy. A page reaches this extension only through the provider, which checks its sender.
const CONTEXT_KEY = "canoa_pending_contexts";

chrome.tabs.onRemoved.addListener(async (tabId) => {
  await detachOnTabClosed(tabId);
  await updateGrants((grants) => revokeTab(grants, tabId));
});
chrome.tabs.onUpdated.addListener(async (tabId, changes) => {
  if (changes.status === "loading") {
    await detachOnTabClosed(tabId);
    await updateGrants((grants) => revokeTab(grants, tabId));
  }
});

chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true }).catch(() => {});
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "canoa-full-page",
    title: "Open Canoa in a tab",
    contexts: ["action"]
  });
  chrome.contextMenus.create({
    id: "canoa-permissions",
    title: "Canoa browser permissions",
    contexts: ["action"]
  });
  chrome.contextMenus.create({
    id: "canoa-ask",
    title: "Ask Canoa",
    contexts: ["page", "selection"]
  });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === "canoa-full-page") {
    chrome.tabs.create({ url: chrome.runtime.getURL("ui/index.html?view=full") });
    return;
  }
  if (info.menuItemId === "canoa-permissions") {
    chrome.runtime.openOptionsPage();
    return;
  }
  if (info.menuItemId !== "canoa-ask" || !tab) return;
  const context = captureContext(info, tab);
  if (!context) return;
  // Open from the user-initiated browser action, without injecting code into
  // the page or granting any connected agent browser permissions.
  chrome.sidePanel.open({ windowId: tab.windowId }).catch(() => {
    chrome.tabs.create({ url: chrome.runtime.getURL("ui/index.html?view=full") });
  });
  void (async () => {
    const stored = await chrome.storage.session.get(CONTEXT_KEY);
    const queue = addContext(stored[CONTEXT_KEY], context);
    await chrome.storage.session.set({ [CONTEXT_KEY]: queue });
  })();
});

// Privileged message boundary: accept context claims only from a packaged
// extension UI page. Content scripts and websites cannot consume the queue.
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  const ui = chrome.runtime.getURL("ui/");
  if (message?.type !== "canoa.context.claim" ||
      sender.id !== chrome.runtime.id ||
      !sender.url?.startsWith(ui)) return false;
  void (async () => {
    const stored = await chrome.storage.session.get(CONTEXT_KEY);
    const { context, remaining } = takeContext(stored[CONTEXT_KEY]);
    await chrome.storage.session.set({ [CONTEXT_KEY]: remaining });
    sendResponse({ context });
  })().catch(() => sendResponse({ context: null }));
  return true;
});

async function localAuthorize(request, verifiedPeerIds) {
  return isAllowed({ grants: await readGrants(), request, trustedPeers: verifiedPeerIds });
}
export const permissions = Object.freeze({
  localAuthorize, readGrants, makeGrant, updateGrants, revokePeer,
  direction: DIRECTION, capability: CAPABILITY
});

// Only the installed extension's own options page can create grants.
// A trusted peer identity is *still* required at each tool invocation.
// Websites and injected content scripts cannot send these messages.
chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (sender.id !== chrome.runtime.id ||
      sender.url !== chrome.runtime.getURL('permissions.html')) return false;
  if (!msg || typeof msg !== 'object' ||
      !['canoa.grants.list','canoa.grants.add','canoa.grants.remove'].includes(msg.type)) {
    return false;
  }
  (async () => {
    if (msg.type === 'canoa.grants.list') {
      reply({grants:await readGrants()});
      return;
    }
    if (msg.type === 'canoa.grants.remove') {
      reply({grants:await updateGrants((current) => current.filter((grant) => !sameGrant(grant,msg.grant)))});
      return;
    }
    if (!/^sha256:[a-f0-9]{64}$/.test(msg.peerId || '') ||
        !Number.isSafeInteger(msg.tabId) || msg.tabId < 0 ||
        ![CAPABILITY.READ,CAPABILITY.INTERACT,CAPABILITY.WEBMCP,
          CAPABILITY.DISCOVER,CAPABILITY.DEBUG].includes(msg.capability) ||
        !Number.isInteger(msg.minutes) || msg.minutes < 1 || msg.minutes > 120) {
      throw Error('Invalid browser permission');
    }
    if (msg.capability === CAPABILITY.DEBUG &&
        !chrome.runtime.getManifest().permissions.includes('debugger')) {
      throw Error('Install the elevated debugger build first');
    }
    const tab = await chrome.tabs.get(msg.tabId);
    if (!tab.url || !/^https?:\/\//.test(tab.url)) throw Error('Protected tab');
    const grant = makeGrant({
      direction:DIRECTION.AGENT_TO_PAGE,
      peerId:msg.peerId,tabId:tab.id,
      origin:new URL(tab.url).origin,capability:msg.capability,
      expiresAt:Date.now() + msg.minutes*60*1000,
    });
    reply({grants:await updateGrants((current) => [...current,grant].slice(-250))});
  })().catch(error=>reply({error:String(error.message || error)}));
  return true;
});
