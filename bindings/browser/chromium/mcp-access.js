// An agent asking for a browser tab. Only the person can grant it, in the provider's window: the agent names what it wants and why,
// and the person picks the tab. Nothing here runs anything on a tab. No chrome.* here, so it can be tested alone.
import { CAPABILITY, DIRECTION, makeGrant } from "./permissions.js";

const MAX_REASON = 400;
const MINUTES = [15, 60];
// However an agent is persuaded to keep asking, a person is not to be worn down by windows.
const AFTER_NO_MS = 30_000;

const ASKABLE = [CAPABILITY.READ, CAPABILITY.INTERACT, CAPABILITY.WEBMCP, CAPABILITY.DISCOVER];

/** The capabilities an agent may ask for here. The debugger exists only in the debugger build. */
export function askableCapabilities(debuggerBuild) {
  return debuggerBuild ? [...ASKABLE, CAPABILITY.DEBUG] : ASKABLE;
}

/** The agent's own words, without what could reorder or hide what is shown around them. */
function cleanReason(reason) {
  return typeof reason === "string" ? reason.replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, MAX_REASON) : "";
}

/**
 * Who asks is told only what they may know: a refusal does not say what tabs exist.
 *
 * @param tabs () => every open tab, as { id, title, url, active }
 * @param ask (details) => { tabId, minutes } from the person, or null
 * @param updateGrants (change) => stores what `change` makes of the grants
 */
export function createAccess({ tabs, ask, updateGrants, debuggerBuild = false, now = Date.now }) {
  const held = new Map();

  /** `via` says who is asking, for the window. `still` is asked as the grant is stored: whether what asked is still allowed to. */
  return async function requestAccess({ peer, label, capability, reason, via, still }) {
    if (!askableCapabilities(debuggerBuild).includes(capability)) return { allowed: false, why: "That cannot be asked for here." };
    const state = held.get(peer) ?? { asking: false, notBefore: 0 };
    held.set(peer, state);
    if (state.asking) return { allowed: false, why: "The person is already being asked." };
    if (now() < state.notBefore) return { allowed: false, why: "The person said no just now. Do not ask again yet." };
    state.asking = true;
    try {
      const open = (await tabs()).filter((tab) => Number.isSafeInteger(tab.id) && /^https?:\/\//.test(tab.url ?? ""));
      const answer = await ask({
        label,
        peer,
        via,
        capability,
        reason: cleanReason(reason),
        tabs: open.map(({ id, title, url, active }) => ({ id, title: cleanReason(title).slice(0, 120), origin: new URL(url).origin, active: active === true })),
        minutes: MINUTES,
      });
      const tab = open.find((candidate) => candidate.id === answer?.tabId);
      if (tab === undefined || !MINUTES.includes(answer.minutes)) {
        state.notBefore = now() + AFTER_NO_MS;
        return { allowed: false, why: "The person did not allow that." };
      }
      const grant = makeGrant({
        direction: DIRECTION.AGENT_TO_PAGE,
        peerId: peer,
        tabId: tab.id,
        origin: new URL(tab.url).origin,
        capability,
        expiresAt: now() + answer.minutes * 60_000,
      });
      let stored = false;
      // Inside the change, which runs in turn with whatever takes grants away, so a person turning this off cannot be overtaken.
      await updateGrants(async (grants) => {
        stored = still === undefined || (await still());
        return stored ? [...grants, grant].slice(-250) : grants;
      });
      if (!stored) return { allowed: false, why: "This was turned off meanwhile." };
      return { allowed: true, tabId: tab.id, origin: grant.origin, capability, expiresAt: grant.expiresAt };
    } finally {
      state.asking = false;
    }
  };
}
