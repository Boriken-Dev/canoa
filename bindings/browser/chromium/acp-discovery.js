// What the companion reports, as the provider's window may show it. Everything here is somebody's claim: a person decides whether
// to add one, and a proof at connect decides whether it holds the key it named. No chrome.* here, so it can be tested alone.
export const COMPANION = "dev.boriken.coqui_acp_bridge";

const PEER = /^sha256:[0-9a-f]{64}$/;
const LOOPBACK = ["localhost", "127.0.0.1", "[::1]"];
const MAX_CANDIDATES = 64;
const MAX_LABEL = 80;
const SOURCES = ["mdns", "local"];

function reachable(text) {
  try {
    const url = new URL(text);
    return url.protocol === "wss:" || (url.protocol === "ws:" && LOOPBACK.includes(url.hostname));
  } catch {
    return false;
  }
}

/** The usable candidates in a reply, without agents the person already has at that address with that key. */
export function readCandidates(reply, known = []) {
  const list = Array.isArray(reply?.candidates) ? reply.candidates.slice(0, MAX_CANDIDATES) : [];
  const usable = [];
  for (const item of list) {
    if (typeof item?.label !== "string" || !item.label.trim() || typeof item.url !== "string" || !reachable(item.url)) continue;
    const peer = typeof item.peer === "string" && PEER.test(item.peer) ? item.peer : undefined;
    if (known.some((agent) => agent.url === item.url && agent.peer === peer)) continue;
    const same = peer === undefined ? undefined : known.find((agent) => agent.peer === peer);
    usable.push({
      label: item.label.trim().slice(0, MAX_LABEL),
      url: item.url,
      peer,
      source: SOURCES.includes(item.source) ? item.source : "mdns",
      // The same key somewhere else: worth saying, and no reason to treat it as that agent until it proves the key.
      sameKeyAs: same?.label,
    });
  }
  return usable;
}
