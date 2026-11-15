// What Canoa on this computer reports when asked who is nearby, as the provider's window may show it. Everything here is somebody's claim: a person decides whether
// to add one, and a proof at connect decides whether it holds the key it named. No chrome.* here, so it can be tested alone.
const PEER = /^sha256:[0-9a-f]{64}$/;
const LOOPBACK = ["localhost", "127.0.0.1", "[::1]"];
const MAX_CANDIDATES = 64;
const MAX_LABEL = 80;
const MAX_URL = 512;
const SOURCES = ["dns-sd", "local"];
const MAX_OFFERS = 16;
const METHODS = ["noise", "webcrypto"];
const OFFER = /^[\x21-\x7e]{1,64}$/;

/** The method an agent added with a key proves by: the one named, else WebCrypto's, as for a record naming none. */
export function addedMethod(method) {
  if (method === undefined || method === null) return "webcrypto";
  if (!METHODS.includes(method)) throw new Error("the method is noise or webcrypto");
  return method;
}

/**
 * The agent the person already has with `peer`, if any. A peer id names a key of one type, so the same id under another
 * method is a mistake or a lie, and is refused with the reason rather than added beside it.
 */
export function sameKey(agents, peer, method) {
  const held = agents.find((agent) => agent.peer === peer);
  if (held === undefined) return undefined;
  const heldMethod = held.method ?? "webcrypto";
  if (heldMethod !== method) throw new Error(`that key is a ${heldMethod} key here, and cannot be a ${method} agent's`);
  return held;
}

/** What the person runs on a machine for its MCP clients, naming this browser's key for that machine's method; null when this browser has none. */
export function mcpConfigCommand(ids, method) {
  const id = ids[method ?? "webcrypto"];
  return typeof id === "string" ? `coqui-acp-bridge mcp-config --peer ${id}` : null;
}

/** Where WebTransport is, if the candidate gave both a port and a certificate hash to pin. */
function webTransportOf(item) {
  if (!Number.isInteger(item.webtransport) || item.webtransport < 1 || item.webtransport > 65535) return undefined;
  if (typeof item.webtransportHash !== "string" || !PEER.test(item.webtransportHash)) return undefined;
  const { hostname, pathname } = new URL(item.url);
  return { url: `https://${hostname}:${item.webtransport}${pathname}`, hash: item.webtransportHash };
}

/** The protocols a candidate says it serves, as a hint for choosing: what a peer may open is decided on the link itself. */
export function offersOf(item) {
  if (!Array.isArray(item.offers)) return [];
  return item.offers
    .filter((offer) => typeof offer === "string" && OFFER.test(offer) && !/[,=]/.test(offer))
    .slice(0, MAX_OFFERS);
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

/** The code of a host's JSON-RPC answer that says the key may not do this. */
export const REFUSED = -32001;

/**
 * What a search said about itself that the person should read: a descriptor folder that was not read or could not be checked, and
 * why the network could not be searched. Text from the host, so cut down and stripped of anything that reorders it.
 */
export function searchNotes(reply) {
  const notes = [];
  if (reply?.directory === "shared") notes.push("The folder where agents on this computer announce themselves can be changed by another user, so it was not read.");
  if (reply?.directory === "unchecked") notes.push("The folder where agents on this computer announce themselves could not be checked for who can change it.");
  if (typeof reply?.networkError === "string") {
    const why = reply.networkError.replace(/\p{Cf}|\p{Cc}/gu, "").trim().slice(0, 200);
    if (why !== "") notes.push(`The network could not be searched: ${why}`);
  }
  return notes;
}

/**
 * Asks who is nearby: `viaHost` (undefined when this browser is not paired with a running host) first, then `viaRelay`, the native
 * host searching for itself. A host that refuses the key (no discover grant) is the answer: another route would make the grant mean
 * nothing. A host that cannot be reached, or fails, leaves the relay to look.
 */
export async function lookNearby({ viaHost, viaRelay, known }) {
  if (viaHost !== undefined) {
    try {
      const reply = await viaHost();
      return { candidates: readCandidates(reply, known), notes: searchNotes(reply) };
    } catch (error) {
      if (error?.code === REFUSED) return { candidates: [], refused: true };
    }
  }
  const found = await viaRelay();
  if (found.state === "ok") return { candidates: readCandidates(found.reply, known), notes: searchNotes(found.reply) };
  return { candidates: [], needsCanoa: found.state === "not-installed", why: found.why };
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
    // A record naming no method is WebCrypto's, the only method before records named one; one this browser does not know is unusable.
    const method = item.method === undefined || item.method === null ? "webcrypto" : item.method;
    if (!METHODS.includes(method)) continue;
    const source = SOURCES.includes(item.source) ? item.source : "dns-sd";
    if (source === "local" ? !LOOPBACK.includes(url.hostname) : pointsHome(url.hostname) || peer === undefined) continue;
    const webtransport = webTransportOf(item);
    if (known.some((agent) => agent.url === item.url && agent.peer === peer && agent.webtransport?.hash === webtransport?.hash)) continue;
    const same = peer === undefined ? undefined : known.find((agent) => agent.peer === peer);
    usable.push({
      label: label.slice(0, MAX_LABEL),
      url: item.url,
      peer,
      method,
      source,
      webtransport,
      offers: offersOf(item),
      // The same key somewhere else, which choosing moves that agent to: it still has to prove the key.
      sameKeyAs: same?.label,
      // Two names alike with different keys is what a stranger imitating an agent looks like.
      sameNameAs: same === undefined ? known.find((agent) => agent.label === label)?.label : undefined,
    });
  }
  return usable;
}
