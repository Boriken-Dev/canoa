import assert from "node:assert/strict";
import { test } from "node:test";

import { createLocalHost, fromOwnPage, localAgents, localGrants, readHello, rememberedEntries, stoppedAgents, viewOf } from "./acp-local.js";

const PEER = `sha256:${"ab".repeat(32)}`;
const runningReply = (extra = {}) => ({
  reply: "hello", running: true, peer: PEER, method: "noise", name: "Canoa", version: "0.0.1", control: true,
  entries: [{ name: "claude", label: "Claude", kind: "agent", local: true }, { name: "scripted", label: "Scripted", kind: "agent", local: false }],
  ...extra,
});

/** `connectNative` whose host answers each question by `script(request)`; `null` is no answer, a string a disconnect with that error. */
function scripted(script, log = []) {
  let lastError;
  const connectNative = () => {
    const listeners = { message: [], disconnect: [] };
    return {
      onMessage: { addListener: (listen) => listeners.message.push(listen) },
      onDisconnect: { addListener: (listen) => listeners.disconnect.push(listen) },
      disconnect() {},
      postMessage(request) {
        log.push(request.op);
        const answer = script(request);
        queueMicrotask(() => {
          if (answer === null) return;
          if (typeof answer === "string") {
            lastError = answer;
            listeners.disconnect.forEach((listen) => listen());
          } else {
            listeners.message.forEach((listen) => listen(answer));
          }
        });
      },
    };
  };
  return { connectNative, lastError: () => lastError };
}

test("a running host's entries are what a panel lists, as a claim cut down to size", () => {
  const claim = readHello(runningReply({
    entries: [
      { name: "ok", label: "A‮evil", kind: "agent" },
      { name: "@control", label: "no", kind: "agent" },
      { name: "tool", label: "A tool", kind: "tool" },
      { name: "ok", label: "twice", kind: "agent" },
      ...Array.from({ length: 200 }, (_, n) => ({ name: `e${n}`, label: "x".repeat(500), kind: "provider" })),
    ],
  }));
  assert.equal(claim.entries.length, 64);
  assert.deepEqual(claim.entries[0], { name: "ok", label: "Aevil", kind: "agent", local: false });
  assert.ok(claim.entries.every((entry) => entry.label.length <= 80 && !entry.name.startsWith("@") && entry.kind !== "tool"));
});

test("the names a host gives this computer are kept as names and addresses, bounded", () => {
  const claim = readHello(runningReply({ hostname: "Desk-One", addresses: ["192.0.2.2", "2001:db8::2", "not an address", "192.0.2.2.5", 7, "x".repeat(60), ...Array.from({ length: 40 }, (_, n) => `10.0.0.${n}`)] }));
  assert.equal(claim.hostname, "desk-one");
  assert.deepEqual(claim.thisComputer.slice(0, 4), ["desk-one", "desk-one.local", "192.0.2.2", "2001:db8::2"]);
  assert.ok(claim.thisComputer.length <= 18);
  assert.equal(claim.thisComputer.includes("not an address"), false);
  for (const hostname of ["", "a b", "evil.example/x", "x".repeat(300), 5, "-bad", "bad-"]) {
    const kept = readHello(runningReply({ hostname }));
    assert.deepEqual(kept.thisComputer, [], String(hostname));
  }
  assert.deepEqual(readHello(runningReply({})).thisComputer, []);
});

test("a reply that is not a host's is no claim", () => {
  assert.equal(readHello(undefined), null);
  assert.equal(readHello(runningReply({ peer: "me" })), null);
  assert.equal(readHello(runningReply({ method: "rot13" })), null);
  assert.equal(readHello(runningReply({ reply: "start" })), null);
  assert.equal(readHello({ reply: "hello", running: false }), null);
});

test("three states: no native host, no Canoa running, and Canoa running with what it serves", async () => {
  const missing = scripted(() => "Specified native messaging host not found.");
  assert.deepEqual(await createLocalHost(missing).hello(), { state: "not-installed" });

  const forbidden = scripted(() => "Access to the specified native messaging host is forbidden.");
  assert.deepEqual(await createLocalHost(forbidden).hello(), { state: "not-installed", forbidden: true });

  const stopped = scripted(() => ({ reply: "hello", running: false, door: false }));
  assert.deepEqual(await createLocalHost(stopped).hello(), { state: "stopped" });

  const running = scripted(() => runningReply());
  const state = await createLocalHost(running).hello();
  assert.equal(state.state, "running");
  assert.equal(state.host.peer, PEER);
  assert.deepEqual(state.host.entries.map((entry) => entry.name), ["claude", "scripted"]);

  const doorOff = scripted(() => ({ reply: "hello", running: true, door: false }));
  assert.equal((await createLocalHost(doorOff).hello()).state, "unavailable");
});

test("a host that never answers is unavailable after the wait, not for ever", async () => {
  const silent = scripted(() => null);
  const state = await createLocalHost({ ...silent, helloMs: 20 }).hello();
  assert.equal(state.state, "unavailable");
});

test("connectNative throwing is a state too", async () => {
  const state = await createLocalHost({ connectNative: () => { throw new Error("Specified native messaging host not found."); } }).hello();
  assert.deepEqual(state, { state: "not-installed" });
});

test("Start asks once, waits for the host to answer, and a second click while it starts does nothing", async () => {
  const log = [];
  let asks = 0;
  const host = scripted((request) => {
    if (request.op === "start") return { reply: "start", started: 4242 };
    asks += 1;
    return asks < 3 ? { reply: "hello", running: false } : runningReply();
  }, log);
  const local = createLocalHost({ ...host, pollMs: 1, sleep: () => new Promise((resolve) => setTimeout(resolve, 1)) });
  const first = local.start();
  assert.equal(local.starting, true);
  const second = local.start();
  assert.equal(first, second);
  const state = await first;
  assert.equal(state.state, "running");
  assert.equal(log.filter((op) => op === "start").length, 1, log.join());
  assert.equal(local.starting, false);
});

test("Start that the host program refuses, or that never brings Canoa up, says so", async () => {
  const refused = scripted(() => ({ reply: "start", error: "Canoa is already running" }));
  const state = await createLocalHost(refused).start();
  assert.equal(state.state, "unavailable");
  assert.match(state.why, /already running/);

  const never = scripted((request) => (request.op === "start" ? { reply: "start", started: 1 } : { reply: "hello", running: false }));
  const slow = await createLocalHost({ ...never, startWaitMs: 30, pollMs: 5, sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)) }).start();
  assert.equal(slow.state, "unavailable");
  assert.match(slow.why, /did not come up/);

  const missing = scripted(() => "Specified native messaging host not found.");
  assert.deepEqual(await createLocalHost(missing).start(), { state: "not-installed" });
});

test("only the extension's own pages may start Canoa, never a web page or a content script in one", () => {
  const runtime = { id: "abc", getURL: (path) => `chrome-extension://abc/${path}` };
  assert.equal(fromOwnPage({ id: "abc", url: "chrome-extension://abc/ui/index.html?view=panel" }, runtime), true);
  assert.equal(fromOwnPage({ id: "abc", url: "chrome-extension://abc/permissions.html" }, runtime), true);
  // A content script reports the page it runs in.
  assert.equal(fromOwnPage({ id: "abc", url: "https://evil.example/", tab: { id: 1 } }, runtime), false);
  assert.equal(fromOwnPage({ id: "abc", url: "chrome-extension://abc/acp-request.html?ask=1" }, runtime), false);
  assert.equal(fromOwnPage({ id: "other", url: "chrome-extension://abc/ui/index.html" }, runtime), false);
  assert.equal(fromOwnPage({ id: "abc" }, runtime), false);
  assert.equal(fromOwnPage(undefined, runtime), false);
});

const running = (host = {}) => ({ state: "running", host: { ...readHello(runningReply({ home: "/home/p", ...host })) } });
const pinned = { peer: PEER, method: "noise" };

test("the host's agents are listed only once this browser's key is paired with the host that answers", () => {
  const state = running();
  assert.deepEqual(localAgents(state, null), []);
  assert.deepEqual(localAgents({ state: "stopped" }, pinned), []);
  // A claim of another key than the one paired is not the same Canoa, and nothing is listed from it.
  const other = running({ peer: `sha256:${"cd".repeat(32)}` });
  assert.deepEqual(localAgents(other, pinned), []);
  assert.equal(viewOf(other, pinned).changed, true);
  const agents = localAgents(state, pinned);
  assert.deepEqual(agents.map((agent) => agent.key), ["local:claude", "local:scripted"]);
  assert.deepEqual(agents[0].native, { entry: "claude" });
  assert.equal(agents[0].peer, PEER, "the pinned key, which the proof checks");
  assert.equal(agents[0].method, "noise");
  assert.equal(agents[0].source, "local");
});

test("Canoa's own pages hold those agents with a folder the host makes, never one the host's hello names, and nobody else's page is given any", () => {
  const grants = localGrants("chrome-extension://me", running(), pinned);
  assert.equal(grants.length, 2);
  assert.ok(grants.every((grant) => grant.origin === "chrome-extension://me" && grant.cwd === "/" && grant.site === "canoa-extension" && grant.permissions === true));
  assert.equal(readHello(runningReply({ home: "/home/p" })).home, undefined, "a claim of a folder is not read");
  assert.deepEqual(localGrants("chrome-extension://me", running(), null), []);
});

test("the view says what a panel needs and nothing else of the claim", () => {
  const view = viewOf(running(), pinned);
  assert.equal(view.state, "running");
  assert.equal(view.paired, true);
  assert.equal(view.canPin, true);
  assert.deepEqual(Object.keys(view.host).sort(), ["control", "entries", "method", "name", "peer", "version"]);
  assert.equal(viewOf({ state: "not-installed" }, pinned).paired, false);
  assert.equal(viewOf(running(), null).paired, false);
});

test("discover asks the native host to look itself, bounds the wait, and tells not installed from a reply that is not a search", async () => {
  const log = [];
  const found = { reply: "discover", candidates: [{ label: "A", url: "ws://127.0.0.1:1/acp" }] };
  const asked = [];
  const { connectNative, lastError } = scripted((request) => {
    asked.push(request);
    return found;
  }, log);
  const local = createLocalHost({ connectNative, lastError, helloMs: 50 });
  assert.deepEqual(await local.discover(3), { state: "ok", reply: found });
  assert.deepEqual(asked[0], { op: "discover", seconds: 3 });
  await local.discover(3600);
  assert.equal(asked[1].seconds, 10, "a long wait is cut to what the host takes");

  const missing = scripted(() => "Specified native messaging host not found.");
  assert.deepEqual(await createLocalHost({ ...missing, helloMs: 50 }).discover(3), { state: "not-installed" });

  const odd = scripted(() => ({ error: "unknown op" }));
  const refused = await createLocalHost({ ...odd, helloMs: 50 }).discover(3);
  assert.equal(refused.state, "unavailable");
  assert.equal(refused.why, "unknown op");
  const wrong = scripted(() => ({ reply: "discover", candidates: "many" }));
  assert.equal((await createLocalHost({ ...wrong, helloMs: 50 }).discover(3)).state, "unavailable");
});

test("the host's agents are proven by the key a PIN bound, and are remembered for a grant to outlast the host being stopped", () => {
  const state = running();
  assert.ok(localAgents(state, pinned).every((agent) => agent.proven === true));
  const kept = rememberedEntries(state, pinned, null);
  assert.equal(kept.peer, PEER);
  assert.deepEqual(kept.entries.map((entry) => entry.name), ["claude", "scripted"]);
  // Nothing is remembered from a host this browser is not paired with, and a state that is not a host's keeps what was.
  assert.equal(rememberedEntries(state, null, null), null);
  assert.equal(rememberedEntries({ state: "stopped" }, pinned, kept), kept);
  const stopped = stoppedAgents({ state: "stopped" }, pinned, kept);
  assert.deepEqual(stopped.map((agent) => agent.key), ["local:claude", "local:scripted"]);
  assert.ok(stopped.every((agent) => agent.peer === PEER && agent.local === true));
  assert.equal(stoppedAgents({ state: "unavailable", why: "x" }, pinned, kept).length, 2);
  // Not for an uninstalled host, a running one (its own list is the truth), an unpaired browser or another host's key.
  assert.deepEqual(stoppedAgents({ state: "not-installed" }, pinned, kept), []);
  assert.deepEqual(stoppedAgents(state, pinned, kept), []);
  assert.deepEqual(stoppedAgents({ state: "stopped" }, null, kept), []);
  assert.deepEqual(stoppedAgents({ state: "stopped" }, { ...pinned, peer: `sha256:${"cd".repeat(32)}` }, kept), []);
});
