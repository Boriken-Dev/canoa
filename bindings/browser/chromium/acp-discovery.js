// What the companion reports, as the provider's window may show it. Everything here is somebody's claim: a person decides whether
// to add one, and a proof at connect decides whether it holds the key it named. No chrome.* here, so it can be tested alone.
export const COMPANION = "dev.boriken.coqui_acp_bridge";

const PEER = /^sha256:[0-9a-f]{64}$/;
const LOOPBACK = ["localhost", "127.0.0.1", "[::1]"];
const MAX_CANDIDATES = 64;
const MAX_LABEL = 80;
const MAX_URL = 512;
const SOURCES = ["dns-sd", "local"];

/** Where WebTransport is, if the candidate gave both a port and a certificate hash to pin. */
function webTransportOf(item) {
  if (!Number.isInteger(item.webtransport) || item.webtransport < 1 || item.webtransport > 65535) return undefined;
  if (typeof item.webtransportHash !== "string" || !PEER.test(item.webtransportHash)) return undefined;
  const { hostname, pathname } = new URL(item.url);
  return { url: `https://${hostname}:${item.webtransport}${pathname}`, hash: item.webtransportHash };
}

function parsed(text) {
  try {
    const url = new URL(text);
    // A user name in a URL shows one host and connects to another.
    if (text.length > MAX_URL || url.username !== "" || url.password !== "") return null;
    return url.protocol === "wss:" || (url.protocol === "ws:" && LOOPBACK.includes(url.hostname)) ? url : null;
  } catch {
    return null;
  }
}

/** Addresses a record must not name: this machine, and what is only reachable from it. URL has already turned every spelling of an IPv4 address into the dotted one. */
function pointsHome(hostname) {
  if (LOOPBACK.includes(hostname) || hostname.endsWith(".localhost")) return true;
  if (hostname.startsWith("[")) return /^\[(::|::ffff:|64:ff9b::|fe[89ab])/.test(hostname);
  if (!hostname.includes(".")) return true;
  return /^(127|0|169\.254)\./.test(hostname);
}

/** The usable candidates in a reply, without agents the person already has at that address with that key. */
export function readCandidates(reply, known = []) {
  const list = Array.isArray(reply?.candidates) ? reply.candidates.slice(0, MAX_CANDIDATES) : [];
  const usable = [];
  for (const item of list) {
    if (typeof item?.label !== "string" || typeof item.url !== "string") continue;
    // Format characters can reorder what is shown around them, or hide part of a name.
    const label = item.label.replace(/\p{Cf}|\p{Cc}/gu, "").trim();
    const url = parsed(item.url);
    if (label === "" || url === null) continue;
    const peer = typeof item.peer === "string" && PEER.test(item.peer) ? item.peer : undefined;
    const source = SOURCES.includes(item.source) ? item.source : "dns-sd";
    if (source === "local" ? !LOOPBACK.includes(url.hostname) : pointsHome(url.hostname) || peer === undefined) continue;
    const webtransport = webTransportOf(item);
    if (known.some((agent) => agent.url === item.url && agent.peer === peer && agent.webtransport?.hash === webtransport?.hash)) continue;
    const same = peer === undefined ? undefined : known.find((agent) => agent.peer === peer);
    usable.push({
      label: label.slice(0, MAX_LABEL),
      url: item.url,
      peer,
      source,
      webtransport,
      // The same key somewhere else, which choosing moves that agent to: it still has to prove the key.
      sameKeyAs: same?.label,
      // Two names alike with different keys is what a stranger imitating an agent looks like.
      sameNameAs: same === undefined ? known.find((agent) => agent.label === label)?.label : undefined,
    });
  }
  return usable;
}
