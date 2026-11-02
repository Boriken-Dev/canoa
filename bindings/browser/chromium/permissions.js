/**
 * Browser-owned authorization decisions. Transport adapters must provide the
 * identity of a cryptographically verified peer in trustedPeers; a webpage's
 * claimed peer id must never be added to that set.
 *
 * This module has no chrome.* dependencies and can be independently tested.
 */

export const DIRECTION = Object.freeze({
  PAGE_TO_AGENT: "page-to-agent",
  AGENT_TO_PAGE: "agent-to-page"
});

export const CAPABILITY = Object.freeze({
  CONNECT: "agent.connect",
  PROMPT: "agent.prompt",
  READ: "browser.read",
  INTERACT: "browser.interact",
  WEBMCP: "webmcp.invoke",
  DISCOVER: "mcp.discover",
  DEBUG: "browser.debug"
});

const directions = new Set(Object.values(DIRECTION));
const capabilities = new Set(Object.values(CAPABILITY));

function validOrigin(value) {
  if (typeof value !== "string") return false;
  try {
    const parsed = new URL(value);
    return ["https:", "http:"].includes(parsed.protocol) &&
      parsed.origin === value &&
      parsed.hostname.length > 0 &&
      !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

function validGrant(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  if (!directions.has(value.direction) || !capabilities.has(value.capability)) return false;
  if (typeof value.peerId !== "string" || value.peerId.length < 1 || value.peerId.length > 256) return false;
  if (!validOrigin(value.origin)) return false;
  if (!Number.isSafeInteger(value.expiresAt) || value.expiresAt <= 0) return false;

  if (value.direction === DIRECTION.AGENT_TO_PAGE) {
    if (!Number.isSafeInteger(value.tabId) || value.tabId < 0) return false;
    if (![CAPABILITY.READ, CAPABILITY.INTERACT, CAPABILITY.WEBMCP, CAPABILITY.DISCOVER, CAPABILITY.DEBUG]
      .includes(value.capability)) return false;
  } else {
    if (value.tabId !== undefined) return false;
    if (![CAPABILITY.CONNECT, CAPABILITY.PROMPT].includes(value.capability)) return false;
  }
  return true;
}

/**
 * Create a grant after an explicit local user decision.
 * A grant by itself never authorizes an unverified peer.
 */
export function makeGrant({ direction, peerId, origin, tabId, capability, expiresAt }) {
  const grant = {
    direction, peerId, origin, capability, expiresAt,
    ...(tabId === undefined ? {} : { tabId })
  };
  if (!validGrant(grant)) throw new TypeError("Invalid Canoa permission grant");
  return Object.freeze(grant);
}

/** No implicit inheritance from read to write, or from connect to prompt. */
export function isAllowed({ grants, request, trustedPeers, now = Date.now() }) {
  if (!Array.isArray(grants) || !(trustedPeers instanceof Set)) return false;
  if (!validGrant(request) || !Number.isFinite(now)) return false;
  if (!trustedPeers.has(request.peerId)) return false;

  return grants.some((grant) =>
    validGrant(grant) &&
    grant.expiresAt > now &&
    grant.peerId === request.peerId &&
    grant.direction === request.direction &&
    grant.origin === request.origin &&
    grant.capability === request.capability &&
    (grant.direction !== DIRECTION.AGENT_TO_PAGE || grant.tabId === request.tabId)
  );
}

/**
 * Drop all permissions on tab close / navigation.
 * Revocation is independent of agent-side session permissions.
 */
export function revokeTab(grants, tabId) {
  if (!Array.isArray(grants)) return [];
  return grants.filter((g) => g.tabId !== tabId);
}

/** Whether two grants are the same one: a position in the list says nothing, since grants come and go behind whoever is looking. */
export function sameGrant(a, b) {
  return a?.direction === b?.direction && a?.peerId === b?.peerId && a?.tabId === b?.tabId
    && a?.origin === b?.origin && a?.capability === b?.capability && a?.expiresAt === b?.expiresAt;
}

export function revokePeer(grants, peerId) {
  if (!Array.isArray(grants)) return [];
  return grants.filter((g) => g.peerId !== peerId);
}

export function pruneExpired(grants, now = Date.now()) {
  if (!Array.isArray(grants)) return [];
  return grants.filter((g) => validGrant(g) && g.expiresAt > now);
}
