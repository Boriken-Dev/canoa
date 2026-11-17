import test from "node:test";
import assert from "node:assert/strict";
import { createToolProvider, PROVIDER, whereOf } from "./tool-provider.js";
import { handleMcpRequest } from "./mcp.js";
import { createAccess } from "./mcp-access.js";
import { revokePeer } from "./permissions.js";

const A = "a".repeat(32);
const B = "b".repeat(32);
const settle = () => new Promise((resolve) => setImmediate(resolve));

/** The link to `@tools` as the host sees it: what the provider sent, and a way to say something to it. */
function fakeLink() {
  const link = {
    sent: [],
    closed: false,
    handlers: {},
    send: async (text) => {
      link.sent.push(JSON.parse(text));
    },
    onMessage(fn) {
      link.handlers.message = fn;
    },
    onClose(fn) {
      link.handlers.close = fn;
    },
    close() {
      link.closed = true;
    },
    hear: (message) => link.handlers.message(JSON.stringify(message)),
    end: () => link.handlers.close(),
    mcp: (stream) => link.sent.filter((message) => message.type === "mcp" && message.stream === stream).map((message) => message.message),
  };
  return link;
}

/** A provider whose serve is the extension's own MCP handler over in-memory grants, with a person who allows the first tab. */
function harness({ answer = { tabId: 7, minutes: 15 } } = {}) {
  const link = fakeLink();
  const state = { grants: [], asked: [], dispatched: [], forgotten: [], status: [] };
  const access = createAccess({
    tabs: async () => [{ id: 7, title: "Docs", url: "https://example.org/a", active: true }],
    ask: async (details) => {
      state.asked.push(details);
      return answer;
    },
    updateGrants: async (change) => {
      state.grants = await change(state.grants);
    },
  });
  const provider = createToolProvider({
    dial: async () => link,
    serve: ({ principal, label, where, request, still }) => handleMcpRequest({
      request,
      peerId: principal,
      trustedPeers: new Set([principal]),
      grants: state.grants,
      dispatch: async (call) => {
        state.dispatched.push({ peerId: call.peerId, name: call.name });
        return { text: "the page" };
      },
      access: {
        capabilities: ["browser.read"],
        logs: false,
        request: ({ capability, reason }) => access({ peer: principal, label, capability, reason, via: "local", where, still }),
      },
    }),
    forget: (principal) => {
      state.forgotten.push(principal);
      state.grants = revokePeer(state.grants, principal);
    },
    onStatus: (status) => state.status.push(status),
  });
  return { link, provider, state };
}

async function connected(options) {
  const made = harness(options);
  const ready = made.provider.ensure();
  await settle();
  made.link.hear({ type: "registered", provider: PROVIDER });
  await ready;
  return made;
}

const open = (link, stream, session = { agent: "Claude", via: "app" }) => link.hear({ type: "open", stream, session });
const ask = (link, stream, id, method, params) => link.hear({ type: "mcp", stream, message: { jsonrpc: "2.0", id, method, params } });
const names = (reply) => reply.result.tools.map((tool) => tool.name);

test("it registers as the browser and is connected once the host says so", async () => {
  const { link, provider, state } = await connected();
  assert.deepEqual(link.sent[0], { type: "register", provider: "browser" });
  assert.equal(provider.connected, true);
  assert.deepEqual(state.status, ["connected"]);
});

test("a host that refuses the registration is no connection, and says why", async () => {
  const made = harness();
  const ready = made.provider.ensure();
  await settle();
  made.link.hear({ type: "refused", why: "this key may not offer tools to this host" });
  await assert.rejects(ready, /may not offer tools/);
  assert.equal(made.provider.connected, false);
  assert.equal(made.link.closed, true);
});

test("one connection at a time: ensure while connecting or connected is the same connection", async () => {
  const made = harness();
  const first = made.provider.ensure();
  const second = made.provider.ensure();
  await settle();
  made.link.hear({ type: "registered" });
  await Promise.all([first, second]);
  assert.equal(made.link.sent.filter((message) => message.type === "register").length, 1);
  await made.provider.ensure();
  assert.equal(made.link.sent.filter((message) => message.type === "register").length, 1);
});

test("a grant the person gives in the window reaches that session only", async () => {
  const { link, state } = await connected();
  open(link, A);
  open(link, B, { agent: "Codex", via: "paired", peer: "sha256:" + "1".repeat(64) });
  ask(link, A, 1, "tools/list");
  ask(link, B, 1, "tools/list");
  await settle();
  assert.deepEqual(names(link.mcp(A)[0]), ["browser_request_access"]);
  assert.deepEqual(names(link.mcp(B)[0]), ["browser_request_access"]);

  ask(link, A, 2, "tools/call", { name: "browser_request_access", arguments: { capability: "browser.read", reason: "look at the docs" } });
  await settle();
  await settle();
  const allowed = link.mcp(A)[1];
  assert.equal(allowed.result.isError, false, JSON.stringify(allowed));
  // The window said who asks and where it runs, in the host's words, and the agent's reason as its own.
  assert.equal(state.asked.length, 1);
  assert.equal(state.asked[0].label, "Claude");
  assert.equal(state.asked[0].via, "local");
  assert.equal(state.asked[0].where, "the Canoa app on this computer");
  assert.equal(state.asked[0].reason, "look at the docs");
  assert.equal(state.asked[0].peer, `local:${A}`);

  ask(link, A, 3, "tools/list");
  ask(link, B, 2, "tools/list");
  ask(link, A, 4, "tools/call", { name: "browser_read_page", arguments: { tabId: 7 } });
  ask(link, B, 3, "tools/call", { name: "browser_read_page", arguments: { tabId: 7 } });
  await settle();
  await settle();
  assert.ok(names(link.mcp(A)[2]).includes("browser_read_page"), "the session that asked has the read tools");
  assert.deepEqual(names(link.mcp(B)[1]), ["browser_request_access"], "the other session was given nothing");
  assert.equal(link.mcp(A)[3].result.isError, false);
  assert.equal(link.mcp(B)[2].result.isError, true, "and cannot use the tab");
  assert.deepEqual(state.dispatched, [{ peerId: `local:${A}`, name: "browser_read_page" }]);
});

test("a session the host names to another agent is told apart by the host's words: paired is not the app", () => {
  assert.equal(whereOf({ via: "app" }), "the Canoa app on this computer");
  assert.match(whereOf({ via: "paired" }), /program paired with Canoa/);
  assert.equal(whereOf(undefined), "the Canoa app on this computer");
});

test("ending a session, or a changed grant, reaches the agent and takes what it held", async () => {
  const { link, provider, state } = await connected();
  open(link, A);
  ask(link, A, 1, "tools/call", { name: "browser_request_access", arguments: { capability: "browser.read", reason: "x" } });
  await settle();
  await settle();
  assert.equal(state.grants.length, 1);

  // Revoking under the permissions page: the grants change, and every session is told its list may have.
  state.grants = [];
  provider.changed();
  await settle();
  assert.deepEqual(link.mcp(A).at(-1), { jsonrpc: "2.0", method: "notifications/tools/list_changed" });
  ask(link, A, 2, "tools/call", { name: "browser_read_page", arguments: { tabId: 7 } });
  await settle();
  assert.equal(link.mcp(A).at(-1).result.isError, true);

  // The session ended: what it was given goes, and a window still asking for it is closed.
  ask(link, A, 3, "tools/call", { name: "browser_request_access", arguments: { capability: "browser.read", reason: "x" } });
  await settle();
  await settle();
  assert.equal(state.grants.length, 1);
  link.hear({ type: "close", stream: A });
  assert.deepEqual(state.forgotten, [`local:${A}`]);
  assert.equal(state.grants.length, 0);
  assert.deepEqual(provider.principals(), []);
});

test("the link ending takes every session's grants and the connection", async () => {
  const { link, provider, state } = await connected();
  open(link, A);
  open(link, B);
  assert.equal(provider.principals().length, 2);
  link.end();
  assert.deepEqual(state.forgotten.sort(), [`local:${A}`, `local:${B}`].sort());
  assert.equal(provider.connected, false);
  assert.deepEqual(state.status, ["connected", "unavailable"]);
  assert.equal(link.closed, true);
});

test("stopping, as when the person unpairs, ends it and says it is off", async () => {
  const { link, provider, state } = await connected();
  open(link, A);
  provider.stop();
  assert.equal(link.closed, true);
  assert.deepEqual(state.forgotten, [`local:${A}`]);
  assert.equal(state.status.at(-1), "off");
});

test("what is no stream of the host's is ignored: bad ids, unknown streams, malformed messages", async () => {
  const { link, provider } = await connected();
  open(link, "not-hex");
  open(link, "a".repeat(31));
  assert.deepEqual(provider.principals(), []);
  ask(link, A, 1, "tools/list");
  link.handlers.message("not json");
  link.handlers.message("null");
  link.hear({ type: "mcp", stream: A });
  await settle();
  assert.equal(link.sent.length, 1, "only the registration was sent");
  open(link, A);
  open(link, A);
  assert.equal(provider.principals().length, 1, "a stream opens once");
  link.hear({ type: "mcp", stream: A, message: ["tools/list"] });
  await settle();
  assert.equal(link.sent.length, 1);
});

test("the agent's name is cleaned of what could hide or reorder text and kept short", async () => {
  const { link, state } = await connected();
  open(link, A, { agent: "Cl‮aude\u0000" + "x".repeat(200), via: "app" });
  ask(link, A, 1, "tools/call", { name: "browser_request_access", arguments: { capability: "browser.read", reason: "x" } });
  await settle();
  await settle();
  const label = state.asked[0].label;
  assert.ok(!/[‮\u0000]/.test(label), label);
  assert.ok(label.length <= 80, label.length);
});

test("a session that asks for more calls than it may at once is refused the rest", async () => {
  const hold = [];
  const link = fakeLink();
  const provider = createToolProvider({
    dial: async () => link,
    serve: () => new Promise((resolve) => hold.push(resolve)),
  });
  const ready = provider.ensure();
  await settle();
  link.hear({ type: "registered" });
  await ready;
  open(link, A);
  for (let id = 1; id <= 17; id += 1) ask(link, A, id, "tools/list");
  await settle();
  assert.equal(hold.length, 16);
  const refused = link.mcp(A);
  assert.equal(refused.length, 1);
  assert.equal(refused[0].id, 17);
  assert.equal(refused[0].error.code, -32000);
});
