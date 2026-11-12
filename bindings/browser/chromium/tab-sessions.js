// Which Canoa session belongs to which tab, and which tabs a session shares with its agent. Attaching a session to a tab grants
// nothing: only `shareGrant`, made on the person's click, lets the session's agent read that tab, and it lasts until it is taken
// away. No chrome.* here, so it can be tested alone.
import { CAPABILITY, DIRECTION, UNTIL_REVOKED, makeGrant } from "./permissions.js";

/** The agent's name on its tab group; what the agent wrote about itself is cut and cleaned. */
export function groupTitle(label) {
  const title = String(label ?? "").replace(/\p{Cf}|\p{Cc}/gu, " ").replace(/\s+/g, " ").trim().slice(0, 40);
  return title === "" ? "Canoa" : title;
}

export const GROUP_COLOR = "cyan";

const validId = (value) => typeof value === "string" && value.length > 0 && value.length <= 128;
const validTab = (value) => Number.isSafeInteger(value) && value >= 0;

/** Binds a session to one tab (a session is in one place at a time): the old binding of that session is replaced. */
export function attach(bindings, { sessionId, tabId, origin }) {
  if (!validId(sessionId) || !validTab(tabId)) throw new TypeError("Invalid attach");
  const site = new URL(origin).origin;
  if (!/^https?:/.test(site)) throw new TypeError("Invalid attach");
  return [...bindings.filter((binding) => binding.sessionId !== sessionId), { sessionId, tabId, origin: site }];
}

export const detachSession = (bindings, sessionId) => bindings.filter((binding) => binding.sessionId !== sessionId);
export const detachTab = (bindings, tabId) => bindings.filter((binding) => binding.tabId !== tabId);

/** The sessions recorded for one origin, and nothing else about them. */
export function sessionsFor(bindings, origin) {
  let site;
  try {
    site = new URL(origin).origin;
  } catch {
    return [];
  }
  return bindings.filter((binding) => binding.origin === site).map((binding) => binding.sessionId);
}

export const tabOf = (bindings, sessionId) => bindings.find((binding) => binding.sessionId === sessionId)?.tabId;

/** The grant a click on "share this tab" makes: read, for that session's agent, on that tab and origin, until it is taken away. */
export function shareGrant({ sessionId, peerId, tabId, origin }) {
  if (!validId(sessionId)) throw new TypeError("Invalid share");
  return makeGrant({
    direction: DIRECTION.AGENT_TO_PAGE, peerId, tabId, origin: new URL(origin).origin,
    capability: CAPABILITY.READ, expiresAt: UNTIL_REVOKED, session: sessionId
  });
}

const isShare = (grant) => typeof grant.session === "string" && grant.capability === CAPABILITY.READ;

/** Adds the share once: sharing a tab again is the same share, not a second one. */
export function addShare(grants, grant) {
  return [...grants.filter((held) => !(isShare(held) && held.session === grant.session && held.tabId === grant.tabId)), grant];
}

export const unshare = (grants, { sessionId, tabId }) => grants.filter((grant) => !(isShare(grant) && grant.session === sessionId && grant.tabId === tabId));
/** Ends a session's shares. Without a session id it ends nothing: an undefined id would match every grant made some other way. */
export const endSession = (grants, sessionId) => (validId(sessionId) ? grants.filter((grant) => grant.session !== sessionId) : grants);
export const sharedTabs = (grants, sessionId) => [...new Set(grants.filter((grant) => isShare(grant) && grant.session === sessionId).map((grant) => grant.tabId))];

/** Tabs that were grouped for an agent and no longer have any grant: they leave the group, and nothing else does. */
export function tabsToRelease(acting, grants) {
  return acting.filter((entry) => !grants.some((grant) => grant.tabId === entry.tabId));
}

/** Notes a tab as acted on by a peer, once, with the group it was put in. The name is only what the group is called. */
export function noteActing(acting, { tabId, peer, label, groupId, windowId }) {
  return acting.some((entry) => entry.tabId === tabId) ? acting : [...acting, { tabId, peer, label: groupTitle(label), groupId, windowId }];
}

/** The group already made for this peer in that window, so its tabs gather in one. Two agents with one name are two peers. */
export function groupOf(acting, peer, windowId) {
  return acting.find((entry) => entry.peer === peer && entry.windowId === windowId)?.groupId;
}

/** Tabs recorded as ours that are no longer in the group we put them in (the person moved them, or they are gone). */
export function leftTheirGroup(acting, groupIdOf) {
  return acting.filter((entry) => groupIdOf(entry.tabId) !== entry.groupId);
}

/** Only the extension's own UI pages (the panel, the full-page view) may bind sessions and share tabs: a web page or a content script is refused. */
export function fromUiPage(sender, runtime) {
  return sender?.id === runtime.id && typeof sender.url === "string" && sender.url.startsWith(runtime.getURL("ui/"));
}
