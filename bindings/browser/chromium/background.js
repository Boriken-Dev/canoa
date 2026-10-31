import {
  DIRECTION, CAPABILITY, isAllowed, makeGrant, revokePeer, revokeTab
} from "./permissions.js";

// Browser-owned, deny-by-default policy. No page-origin messages are accepted.
const KEY = "canoa_grants";

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
});
chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "canoa-full-page") {
    chrome.tabs.create({ url: chrome.runtime.getURL("ui/index.html?view=full") });
  }
});

async function localAuthorize(request, verifiedPeerIds) {
  return isAllowed({ grants: await readGrants(), request, trustedPeers: verifiedPeerIds });
}
export const permissions = Object.freeze({
  localAuthorize, readGrants, makeGrant, storeGrants, revokePeer,
  direction: DIRECTION, capability: CAPABILITY
});
