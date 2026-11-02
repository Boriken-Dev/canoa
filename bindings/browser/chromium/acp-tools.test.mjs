// An agent reaching the browser's tools over its link: a real mux both ways, standing in for the bridge at the far end, and the real
// gateway and MCP handler.
import assert from "node:assert/strict";
import { test } from "node:test";

import { createMux } from "./vendor/coqui-agentic-peer/mux.mjs";
import { createGateway } from "./acp-gateway.js";
import { createTools, OFFER, SERVER_NAME } from "./acp-tools.js";
import { handleMcpRequest } from "./mcp.js";
import { askableCapabilities, createAccess } from "./mcp-access.js";
import { CAPABILITY, DIRECTION, makeGrant } from "./permissions.js";

const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

function linkedMuxes() {
  const pair = (other) => ({
    send: async (bytes) => other.deliver?.(new Uint8Array(bytes)),
    onMessage(take) {
      this.deliver = take;
    },
    onClose() {},
  });
  const near = {};
  const far = {};
  Object.assign(near, pair(far));
  Object.assign(far, pair(near));
  return { near: createMux(near, { side: "dialed" }), far: createMux(far, { side: "answered" }) };
}

const ENTRY = { name: SERVER_NAME, command: "/abs/bridge", args: ["mcp-shim"], env: [{ name: "A", value: "127.0.0.1:1" }] };

/** The bridge's half: answers an offer with `entry`, and hands back each offer's stream. */
function fakeBridge(far, entry = ENTRY) {
  const offered = [];
  (async () => {
    for (;;) {
      const stream = await far.accept();
      if (stream === null) return;
      assert.equal(stream.protocol, OFFER);
      assert.equal(new TextDecoder().decode(stream.metadata), SERVER_NAME);
      await stream.send(JSON.stringify(entry));
      offered.push(stream);
    }
  })();
  return offered;
}

const sessionNew = (extra = []) => JSON.stringify({ jsonrpc: "2.0", id: 1, method: "session/new", params: { cwd: "/w", mcpServers: extra } });
const rpc = (id, method, params = {}) => ({ jsonrpc: "2.0", id, method, params });

async function receiveJson(stream) {
  return JSON.parse(new TextDecoder().decode(await stream.receive()));
}

test("a session gets the server, beside what it already named; anything else passes unchanged", async () => {
  const { near, far } = linkedMuxes();
  fakeBridge(far);
  const tools = createTools({ mux: near, handle: async () => null });
  const declared = JSON.parse(await tools.declared(sessionNew([{ name: "other" }])));
  assert.deepEqual(declared.params.mcpServers, [{ name: "other" }, ENTRY]);
  const prompt = JSON.stringify(rpc(2, "session/prompt", { sessionId: "s" }));
  assert.equal(await tools.declared(prompt), prompt);
});

test("for each way of opening a session, one offer of its own", async () => {
  const { near, far } = linkedMuxes();
  const offered = fakeBridge(far);
  const tools = createTools({ mux: near, handle: async () => null });
  for (const method of ["session/new", "session/load", "session/resume"]) {
    const declared = JSON.parse(await tools.declared(JSON.stringify(rpc(1, method, { mcpServers: [] }))));
    assert.equal(declared.params.mcpServers.length, 1);
  }
  assert.equal(offered.length, 3);
});

test("a bridge answering anything but a server entry gets no entry declared, and the session still opens", async () => {
  for (const entry of [
    { ...ENTRY, name: "other" },
    { ...ENTRY, command: 7 },
    { ...ENTRY, args: "mcp-shim" },
    { ...ENTRY, env: [{ name: "A" }] },
    { ...ENTRY, command: "x".repeat(5000) },
    "not an object",
  ]) {
    const { near, far } = linkedMuxes();
    fakeBridge(far, entry);
    const tools = createTools({ mux: near, handle: async () => null });
    assert.deepEqual(JSON.parse(await tools.declared(sessionNew())).params.mcpServers, []);
  }
});

test("a link that serves no offer leaves a session as it was", async () => {
  const { near, far } = linkedMuxes();
  (async () => {
    const stream = await far.accept();
    stream.close("refused", "nobody serves that here");
  })();
  const tools = createTools({ mux: near, handle: async () => null });
  assert.equal(await tools.declared(sessionNew()), sessionNew());
});

test("a slow answer holds up nothing else on the stream, and what is not JSON is answered, not thrown", async () => {
  const { near, far } = linkedMuxes();
  const offered = fakeBridge(far);
  let release;
  const tools = createTools({
    mux: near,
    handle: async (request) => {
      if (request.method === "slow") await new Promise((resolve) => (release = resolve));
      return { jsonrpc: "2.0", id: request.id, result: request.method };
    },
  });
  await tools.declared(sessionNew());
  const stream = offered[0];
  await stream.send(JSON.stringify(rpc(1, "slow")));
  await stream.send(JSON.stringify(rpc(2, "quick")));
  assert.equal((await receiveJson(stream)).id, 2);
  await stream.send("not json");
  assert.equal((await receiveJson(stream)).error.code, -32700);
  release();
  assert.equal((await receiveJson(stream)).id, 1);
});

test("so many calls at once are refused past a limit, with the rest answered", async () => {
  const { near, far } = linkedMuxes();
  const offered = fakeBridge(far);
  const tools = createTools({ mux: near, handle: () => new Promise(() => {}) });
  await tools.declared(sessionNew());
  for (let id = 1; id <= 17; id += 1) await offered[0].send(JSON.stringify(rpc(id, "tools/call")));
  const refused = await receiveJson(offered[0]);
  assert.equal(refused.id, 17);
  assert.equal(refused.error.code, -32000);
});

test("a change reaches every session's stream, and an ended one is left alone", async () => {
  const { near, far } = linkedMuxes();
  const offered = fakeBridge(far);
  const tools = createTools({ mux: near, handle: async () => null });
  await tools.declared(sessionNew());
  await tools.declared(sessionNew());
  tools.changed();
  for (const stream of offered) assert.equal((await receiveJson(stream)).method, "notifications/tools/list_changed");
  tools.end();
  tools.changed();
});

// The whole of what the plan asks of a grant, without a browser: the real handler over a real mux, with the person answering.
function browserHost({ answer, tabs = [{ id: 5, title: "A page", url: "https://a.example/x", active: true }], debuggerBuild = false }) {
  let grants = [];
  let clock = 1_000_000;
  const peer = "sha256:agent";
  const asked = [];
  const called = [];
  const access = createAccess({
    tabs: async () => tabs,
    ask: async (details) => {
      asked.push(details);
      return answer(details);
    },
    updateGrants: async (change) => {
      grants = await change(grants);
      return grants;
    },
    debuggerBuild,
    now: () => clock,
  });
  const call = (request) => handleMcpRequest({
    request,
    peerId: peer,
    trustedPeers: new Set([peer]),
    grants,
    now: clock,
    dispatch: async (what) => {
      called.push(what.name);
      return "ran";
    },
    access: {
      capabilities: askableCapabilities(debuggerBuild),
      request: ({ capability, reason }) => access({ peer, label: "Agent", capability, reason }),
    },
  });
  return {
    call,
    asked,
    called,
    revoke: () => {
      grants = [];
    },
    tick: (ms) => {
      clock += ms;
    },
    names: async () => (await call(rpc(1, "tools/list"))).result.tools.map((tool) => tool.name),
  };
}

const askFor = (capability, reason = "to read it") => rpc(2, "tools/call", { name: "browser_request_access", arguments: { capability, reason } });

test("an agent sees only the asking tool until a person grants more, and loses what is taken away", async () => {
  const host = browserHost({ answer: () => ({ tabId: 5, minutes: 15 }) });
  assert.deepEqual(await host.names(), ["browser_request_access"]);
  const denied = await host.call(rpc(3, "tools/call", { name: "browser_read_page", arguments: { tabId: 5 } }));
  assert.equal(denied.result.isError, true);
  assert.deepEqual(host.called, []);

  const granted = await host.call(askFor(CAPABILITY.READ));
  assert.equal(granted.result.isError, false);
  assert.equal(JSON.parse(granted.result.content[0].text).tabId, 5);
  assert.deepEqual(await host.names(), ["browser_request_access", "browser_list_tabs", "browser_read_page", "browser_list_webmcp"]);
  assert.equal((await host.call(rpc(4, "tools/call", { name: "browser_read_page", arguments: { tabId: 5 } }))).result.isError, false);
  assert.deepEqual(host.called, ["browser_read_page"]);

  // Read gives no click.
  assert.equal((await host.call(rpc(5, "tools/call", { name: "browser_click", arguments: { tabId: 5, selector: "a" } }))).result.isError, true);

  host.revoke();
  assert.deepEqual(await host.names(), ["browser_request_access"]);
});

test("a grant ends at its time, without anyone asking again", async () => {
  const host = browserHost({ answer: () => ({ tabId: 5, minutes: 15 }) });
  await host.call(askFor(CAPABILITY.READ));
  host.tick(15 * 60_000 + 1);
  assert.deepEqual(await host.names(), ["browser_request_access"]);
});

test("a person refusing is told to the agent, and it cannot ask again at once", async () => {
  const host = browserHost({ answer: () => null });
  const refused = await host.call(askFor(CAPABILITY.READ));
  assert.equal(refused.result.isError, true);
  assert.equal(host.asked.length, 1);
  await host.call(askFor(CAPABILITY.READ));
  assert.equal(host.asked.length, 1);
  host.tick(30_001);
  await host.call(askFor(CAPABILITY.READ));
  assert.equal(host.asked.length, 2);
});

test("one question at a time: an agent cannot stack windows on a person", async () => {
  let answer;
  const host = browserHost({ answer: () => new Promise((resolve) => (answer = resolve)) });
  const first = host.call(askFor(CAPABILITY.READ));
  await settle();
  const second = await host.call(askFor(CAPABILITY.INTERACT));
  assert.equal(second.result.isError, true);
  assert.equal(host.asked.length, 1);
  answer({ tabId: 5, minutes: 15 });
  assert.equal((await first).result.isError, false);
});

test("the person chooses the tab: an answer naming another, or an odd time, grants nothing", async () => {
  for (const answer of [{ tabId: 6, minutes: 15 }, { tabId: 5, minutes: 100000 }, { tabId: "5", minutes: 15 }]) {
    const host = browserHost({ answer: () => answer });
    assert.equal((await host.call(askFor(CAPABILITY.READ))).result.isError, true);
    assert.deepEqual(await host.names(), ["browser_request_access"]);
  }
});

test("only ordinary web pages can be offered, so not the provider's window or the browser's own pages", async () => {
  const host = browserHost({
    answer: () => null,
    tabs: [
      { id: 1, url: "chrome-extension://abc/acp-request.html" },
      { id: 2, url: "chrome://settings" },
      { id: 3, url: "edge://settings" },
      { id: 4, url: "file:///c/secret" },
      { id: 5, url: "https://a.example/", title: "Fine" },
    ],
  });
  await host.call(askFor(CAPABILITY.READ));
  assert.deepEqual(host.asked[0].tabs.map((tab) => tab.id), [5]);
});

test("the debugger cannot be asked for in the normal build, and is its own grant in the other", async () => {
  const normal = browserHost({ answer: () => ({ tabId: 5, minutes: 15 }) });
  assert.equal((await normal.call(askFor(CAPABILITY.DEBUG))).result.isError, true);
  assert.equal(normal.asked.length, 0);
  const schema = (await normal.call(rpc(1, "tools/list"))).result.tools[0].inputSchema;
  assert.ok(!schema.properties.capability.enum.includes(CAPABILITY.DEBUG));
  assert.ok(!(await normal.names()).some((name) => name.startsWith("browser_debug")));

  const debug = browserHost({ answer: () => ({ tabId: 5, minutes: 15 }), debuggerBuild: true });
  assert.equal((await debug.call(askFor(CAPABILITY.DEBUG))).result.isError, false);
  const names = await debug.names();
  assert.ok(names.includes("browser_debug_command"));
  assert.ok(!names.includes("browser_read_page"), "the debugger is no grant to read");
});

test("what an agent says goes to the person without what could disguise it, and bounded", async () => {
  const host = browserHost({ answer: () => null });
  await host.call(askFor(CAPABILITY.READ, `harmless\u202egnp.exe\u0000 ${"x".repeat(1000)}`));
  const { reason } = host.asked[0];
  assert.ok(!/\u202e|\u0000/.test(reason));
  assert.ok(reason.length <= 400);
});

test("a grant is one peer's: another with the same tab has none", async () => {
  const grants = [makeGrant({ direction: DIRECTION.AGENT_TO_PAGE, peerId: "sha256:other", tabId: 5, origin: "https://a.example", capability: CAPABILITY.READ, expiresAt: Date.now() + 60_000 })];
  const answer = await handleMcpRequest({
    request: rpc(1, "tools/list"),
    peerId: "sha256:agent",
    trustedPeers: new Set(["sha256:agent"]),
    grants,
    dispatch: async () => "ran",
  });
  assert.deepEqual(answer.result.tools, []);
});

test("an MCP version this does not know gets the one it does, as MCP says, rather than an error", async () => {
  const host = browserHost({ answer: () => null });
  const { result } = await host.call(rpc(1, "initialize", { protocolVersion: "2099-01-01" }));
  assert.equal(result.protocolVersion, "2025-11-25");
  assert.equal(result.capabilities.tools.listChanged, true);
  assert.equal((await host.call(rpc(1, "initialize", { protocolVersion: "2025-06-18" }))).result.protocolVersion, "2025-06-18");
});

// A gateway with the tools wired, to show who gets them.
function gatewayWith({ origin, offers }) {
  const key = "agent";
  const agents = [{ key, label: "Agent", url: "wss://a/", peer: "sha256:agent", offers }];
  const grants = [{ origin, agent: key, lifetime: "always", cwd: "/w", permissions: true, sessions: [] }];
  const dialed = [];
  const offered = [];
  const sent = [];
  const gateway = createGateway({
    store: {
      agents: async () => agents,
      markProven: async () => {},
      grants: async (asked) => grants.filter((grant) => grant.origin === asked),
      saveGrant: async () => {},
      secret: async () => "secret",
    },
    dial: async (_agent, options) => {
      dialed.push(options);
      return {
        proven: true,
        mux: options.link ? {} : undefined,
        send: async (text) => sent.push(JSON.parse(text)),
        onMessage() {},
        onClose() {},
        close() {},
      };
    },
    ownOrigin: "chrome-extension://canoa",
    offerTools: () => {
      offered.push(true);
      return {
        declared: async (text) => {
          const message = JSON.parse(text);
          return JSON.stringify({ ...message, params: { ...message.params, mcpServers: [ENTRY] } });
        },
        end() {},
      };
    },
  });
  const listeners = [];
  const port = {
    posted: [],
    postMessage(message) {
      this.posted.push(message);
    },
    onMessage: { addListener: (listen) => listeners.push(listen) },
    onDisconnect: { addListener() {} },
  };
  gateway.attach(port, { origin, tabId: 1 });
  return {
    dialed,
    offered,
    sent,
    async open() {
      listeners[0]({ id: 1, op: "agents" });
      await settle();
      const { id } = port.posted.at(-1).result[0];
      listeners[0]({ id: 2, op: "connect", args: { id } });
      await settle();
      const { conn } = port.posted.at(-1).result;
      listeners[0]({ conn, msg: sessionNew() });
      await settle();
    },
  };
}

test("Canoa's own chat gets the tools from an agent offering them", async () => {
  const own = gatewayWith({ origin: "chrome-extension://canoa", offers: [OFFER] });
  await own.open();
  assert.deepEqual(own.dialed, [{ link: true }]);
  assert.deepEqual(own.sent[0].params.mcpServers, [ENTRY]);
});

test("a page's session has no browser server and no link, ever, whatever the agent offers", async () => {
  const page = gatewayWith({ origin: "https://site.example", offers: [OFFER] });
  await page.open();
  assert.deepEqual(page.dialed, [{ link: false }]);
  assert.deepEqual(page.offered, []);
  assert.deepEqual(page.sent[0].params.mcpServers, []);
});

test("an agent that offers no link gets plain ACP, even from Canoa's own chat", async () => {
  const own = gatewayWith({ origin: "chrome-extension://canoa", offers: [] });
  await own.open();
  assert.deepEqual(own.dialed, [{ link: false }]);
  assert.deepEqual(own.sent[0].params.mcpServers, []);
});
