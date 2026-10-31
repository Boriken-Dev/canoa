import { DIRECTION, CAPABILITY, isAllowed, makeGrant, revokePeer, revokeTab } from "./permissions.js";

// Browser-local, deny-by-default grants. Session storage is visible only to
// the extension service worker by default. Do not expose this via web messaging.
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

// The authenticated ACP/peer transport, native broker, and user-consent UI are
// not yet wired. No external messages are accepted and no peer is implicitly
// trusted. A future transport may call localAuthorize only after verifying its
// peer and the browser's current document origin.
async function localAuthorize(request, verifiedPeerIds) {
  return isAllowed({
    grants: await readGrants(),
    request,
    trustedPeers: verifiedPeerIds
  });
}

// Keep the local policy entry points reachable for future transport adapters,
// without advertising them to webpages.
export const permissions = Object.freeze({
  localAuthorize,
  readGrants,
  makeGrant,
  storeGrants,
  revokePeer,
  direction: DIRECTION,
  capability: CAPABILITY
});
