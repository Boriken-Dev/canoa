// The Canoa on this computer, reached through the browser's native messaging host: whether it is installed, whether it runs, what it
// says it serves, and starting it. Everything it says in `hello` is a claim: it only prefills, and the proof at connect decides.
// No chrome.* here, so it can be tested alone; the worker hands in `connectNative` and `lastError`.
import { NATIVE_HOST } from "./acp-dial.js";

/** How long a question to the native host may take. */
export const HELLO_MS = 5_000;
/** How long Start waits for the host to come up, asking every `POLL_MS`. */
export const START_WAIT_MS = 30_000;
export const POLL_MS = 500;
/** What a search takes beyond its wait: reading the descriptor folder is bounded at 5 s, and each agent's check at about 3 s. */
export const SEARCH_SLACK_MS = 8_000 + 2_000;
/** The longest a search nearby may take; the host clamps to the same. */
export const MAX_SEARCH_SECONDS = 10;

const PEER = /^sha256:[0-9a-f]{64}$/;
const METHODS = ["noise", "webcrypto"];
const KINDS = ["agent", "provider"];
const MAX_ENTRIES = 64;
const MAX_LABEL = 80;
const MAX_NAME = 128;

const clean = (value, max) => (typeof value === "string" ? value.replace(/\p{Cf}|\p{Cc}/gu, "").trim().slice(0, max) : "");

const HOSTNAME = /^[a-z0-9](?:[a-z0-9.-]{0,251}[a-z0-9])?$/;
const IPV4 = /^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)$/;
const IPV6 = /^[0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7}$/;
const MAX_ADDRESSES = 16;

/**
 * The names the host says this computer has, as text a URL's host can be compared with: its host name (and that name in the local
 * network's `.local`) and its network addresses. A claim, and one that only narrows: an agent at one of these names is held to what
 * an agent on this computer is.
 */
function computerNames(reply) {
  const names = [];
  const hostname = typeof reply.hostname === "string" ? reply.hostname.trim().toLowerCase() : "";
  if (HOSTNAME.test(hostname)) names.push(hostname, ...(hostname.includes(".") ? [] : [`${hostname}.local`]));
  let addresses = 0;
  for (const item of Array.isArray(reply.addresses) ? reply.addresses.slice(0, MAX_ADDRESSES * 2) : []) {
    const address = typeof item === "string" ? item.trim().toLowerCase() : "";
    if (!(IPV4.test(address) || (address.length <= 45 && IPV6.test(address))) || names.includes(address)) continue;
    names.push(address);
    addresses += 1;
    if (addresses === MAX_ADDRESSES) break;
  }
  return names;
}

/**
 * What the host's `hello` claims, kept to what a person can be shown and what can be dialed: entries that speak ACP, bounded,
 * with names and labels stripped of anything that reorders or hides text. A reply that is not a host's gives null.
 */
export function readHello(reply) {
  if (reply?.reply !== "hello" || reply.running !== true) return null;
  if (typeof reply.peer !== "string" || !PEER.test(reply.peer) || !METHODS.includes(reply.method)) return null;
  const entries = [];
  for (const item of Array.isArray(reply.entries) ? reply.entries.slice(0, MAX_ENTRIES * 2) : []) {
    const name = clean(item?.name, MAX_NAME);
    if (name === "" || name.startsWith("@") || !KINDS.includes(item?.kind)) continue;
    if (entries.some((held) => held.name === name)) continue;
    entries.push({ name, label: clean(item.label, MAX_LABEL) || name, kind: item.kind, local: item.local === true });
    if (entries.length === MAX_ENTRIES) break;
  }
  return {
    peer: reply.peer,
    method: reply.method,
    name: clean(reply.name, MAX_LABEL) || "Canoa",
    version: clean(reply.version, 32),
    entries,
    control: reply.control === true,
    hostname: HOSTNAME.test(typeof reply.hostname === "string" ? reply.hostname.trim().toLowerCase() : "") ? reply.hostname.trim().toLowerCase() : undefined,
    thisComputer: computerNames(reply),
  };
}

/** The site the extension's own pages are to the host: its sessions work in a scratch folder the host makes for it. */
export const LOCAL_SITE = "canoa-extension";

const NOT_FOUND = /not found/i;
const FORBIDDEN = /forbidden/i;

/**
 * @param connectNative chrome.runtime.connectNative
 * @param lastError () => the text of chrome.runtime.lastError, read when a port disconnects
 * @param sleep (ms) => Promise, replaced in tests
 */
export function createLocalHost({
  connectNative,
  lastError = () => undefined,
  host = NATIVE_HOST,
  helloMs = HELLO_MS,
  startWaitMs = START_WAIT_MS,
  pollMs = POLL_MS,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  /** One question on a port of its own, which is closed after the answer. Resolves with the reply, or { gone } with why there is none. */
  function ask(request, ms) {
    return new Promise((resolve) => {
      let port;
      let done = false;
      let timer;
      const finish = (value) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        try {
          port?.disconnect();
        } catch {
          // Already over.
        }
        resolve(value);
      };
      try {
        port = connectNative(host);
      } catch (error) {
        finish({ gone: String(error?.message ?? error) });
        return;
      }
      timer = setTimeout(() => finish({ gone: "the native host did not answer in time", late: true }), ms);
      port.onMessage.addListener((message) => finish({ reply: message }));
      port.onDisconnect.addListener(() => finish({ gone: lastError() ?? "the native host went away" }));
      port.postMessage(request);
    });
  }

  /** The three states a panel shows, and what the host claims when it runs. */
  async function hello() {
    const answer = await ask({ op: "hello" }, helloMs);
    if (answer.gone !== undefined) {
      if (NOT_FOUND.test(answer.gone)) return { state: "not-installed" };
      if (FORBIDDEN.test(answer.gone)) return { state: "not-installed", forbidden: true };
      return { state: "unavailable", why: clean(answer.gone, 200) };
    }
    const claim = readHello(answer.reply);
    if (claim !== null) return { state: "running", host: claim };
    if (answer.reply?.reply === "hello" && answer.reply.running === false) return { state: "stopped" };
    // The host holds the home with its door off, or something that is no host answered.
    if (answer.reply?.reply === "hello" && answer.reply.running === true) return { state: "unavailable", why: "Canoa runs, but not for browsers" };
    return { state: "unavailable", why: "the native host answered something else" };
  }

  /**
   * Asks the native host to look for agents nearby itself, for when the Canoa that holds the home is not reachable over a paired
   * link: it needs Canoa installed, not running. Resolves { state: "ok", reply } or { state, why } as `hello` does.
   */
  async function discover(seconds) {
    const wait = Math.min(Math.max(Math.trunc(seconds), 1), MAX_SEARCH_SECONDS);
    const answer = await ask({ op: "discover", seconds: wait }, wait * 1000 + SEARCH_SLACK_MS);
    if (answer.gone !== undefined) {
      if (NOT_FOUND.test(answer.gone)) return { state: "not-installed" };
      if (FORBIDDEN.test(answer.gone)) return { state: "not-installed", forbidden: true };
      return { state: "unavailable", why: clean(answer.gone, 200) };
    }
    if (answer.reply?.reply === "discover" && Array.isArray(answer.reply.candidates)) return { state: "ok", reply: answer.reply };
    return { state: "unavailable", why: clean(answer.reply?.error, 200) || "the native host answered something else" };
  }

  let starting;

  /** Asks the host program to start Canoa, then waits for it to answer. A second call while one runs is that one. */
  function start() {
    starting ??= (async () => {
      try {
        const asked = await ask({ op: "start" }, helloMs);
        if (asked.gone !== undefined) return NOT_FOUND.test(asked.gone) ? { state: "not-installed" } : { state: "unavailable", why: clean(asked.gone, 200) };
        if (typeof asked.reply?.started !== "number") return { state: "unavailable", why: clean(asked.reply?.error, 200) || "Canoa could not be started" };
        const until = Date.now() + startWaitMs;
        for (;;) {
          const state = await hello();
          if (state.state === "running" || Date.now() >= until) return state.state === "running" ? state : { state: "unavailable", why: "Canoa did not come up in time" };
          await sleep(pollMs);
        }
      } finally {
        starting = undefined;
      }
    })();
    return starting;
  }

  return { hello, start, discover, get starting() { return starting !== undefined; } };
}

/**
 * Only a page of the extension itself may start Canoa: its panel, its options page and the provider's own windows. A web page's
 * scripts, and the extension's content scripts that run in one, report that page's address and are refused.
 */
export function fromOwnPage(sender, runtime) {
  if (sender?.id !== runtime.id || typeof sender.url !== "string") return false;
  return sender.url === runtime.getURL("permissions.html") || sender.url.startsWith(runtime.getURL("ui/"));
}

/** Any page of the extension, the provider's windows included: enough to show what Canoa on this computer is doing. */
export function fromExtensionPage(sender, runtime) {
  return sender?.id === runtime.id && typeof sender.url === "string" && sender.url.startsWith(runtime.getURL(""));
}

/** What the panel shows: the state, whether this browser's key is paired with the host that answered, and what it serves. */
export function viewOf(state, pinned) {
  const host = state.state === "running" ? state.host : undefined;
  const paired = host !== undefined && pinned !== null && pinned !== undefined && pinned.peer === host.peer;
  return {
    state: state.state,
    forbidden: state.forbidden === true,
    why: state.why,
    host: host === undefined ? undefined : { name: host.name, version: host.version, peer: host.peer, method: host.method, entries: host.entries, control: host.control },
    paired,
    // A key paired with another Canoa than the one that answers: nothing is trusted until the person pairs again.
    changed: host !== undefined && pinned !== null && pinned !== undefined && pinned.peer !== host.peer,
    canPin: host === undefined ? undefined : host.method === "noise",
  };
}

/** The agents the local host serves, once this browser's key is paired with it. The key is the pinned one, never the claimed one. */
export function localAgents(state, pinned) {
  const view = viewOf(state, pinned);
  if (!view.paired) return [];
  return view.host.entries.map((entry) => ({
    key: `local:${entry.name}`,
    label: entry.label,
    source: "local",
    native: { entry: entry.name },
    peer: pinned.peer,
    method: pinned.method,
    // The key was bound by a PIN the person typed, and every connection proves it again: this is the host they paired.
    proven: true,
    offers: [],
    local: true,
  }));
}

/** What a browser that paired once keeps of the host's entries, so a grant on one outlasts the host being stopped. */
export function rememberedEntries(state, pinned, earlier) {
  const view = viewOf(state, pinned);
  if (view.paired) return { peer: pinned.peer, entries: view.host.entries };
  return earlier;
}

/**
 * The agents a page holds a grant on while the host is not answering: the entries it last served, under the key that was paired.
 * Connecting to one fails as an unreachable agent does, not as one the page was never given. Never offered in a window.
 */
export function stoppedAgents(state, pinned, remembered) {
  if (!["stopped", "unavailable"].includes(state.state) || pinned === null || pinned === undefined) return [];
  if (remembered?.peer !== pinned.peer || !Array.isArray(remembered.entries)) return [];
  return localAgents({ state: "running", host: { ...remembered, peer: pinned.peer, method: pinned.method, name: "Canoa", version: "", control: false } }, pinned);
}

/** The grants Canoa's own pages hold on those agents with nothing to ask: the person's own chat, on the person's own computer. */
export function localGrants(origin, state, pinned) {
  const view = viewOf(state, pinned);
  if (!view.paired) return [];
  return localAgents(state, pinned).map((agent) => ({
    origin,
    agent: agent.key,
    lifetime: "always",
    // The host makes the folder, as it does for a site (the gateway names the site); this is only what the scope needs to see.
    cwd: "/",
    site: LOCAL_SITE,
    permissions: true,
    mcpOrigins: [],
    sessions: [],
  }));
}
