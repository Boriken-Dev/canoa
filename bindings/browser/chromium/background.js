import {
  DIRECTION, CAPABILITY, isAllowed, makeGrant, revokePeer, revokeTab
} from "./permissions.js";
import { captureContext, addContext, takeContext } from "./context-transfer.js";

// Browser-owned, deny-by-default policy. No page-origin messages are accepted.
const KEY = "canoa_grants";
const CONTEXT_KEY = "canoa_pending_contexts";

async function readGrants() {
  const value = (await chrome.storage.session.get(KEY))[KEY];
  return Array.isArray(value) ? value : [];
}
async function storeGrants(grants) {
  await chrome.storage.session.set({ [KEY]: grants });
}

chrome.tabs.onRemoved.addListener(async (tabId) => {
  await storeGrants(revokeTab(await readGrants(), tabId));
});
chrome.tabs.onUpdated.addListener(async (tabId, changes) => {
  if (changes.status === "loading") {
    await storeGrants(revokeTab(await readGrants(), tabId));
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
  localAuthorize, readGrants, makeGrant, storeGrants, revokePeer,
  direction: DIRECTION, capability: CAPABILITY
});
