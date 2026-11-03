// A machine's programs reaching the browser's tools: a real mux both ways, standing in for the bridge at the far end, with the real
// MCP handler.
import assert from "node:assert/strict";
import { test } from "node:test";

import { createMux } from "./vendor/coqui-agentic-peer/mux.mjs";
import { createGateway, dialRecord } from "./acp-gateway.js";
import { createStanding, fromPermissionsPage, GOODBYE, MAX_PROGRAMS } from "./acp-standing.js";
import { createTools, OFFER } from "./acp-tools.js";
import { handleMcpRequest } from "./mcp.js";
import { createAccess } from "./mcp-access.js";

const AGENT = { key: "a", label: "The machine", peer: `sha256:${"a".repeat(64)}`, offers: [OFFER], url: "ws://127.0.0.1:1" };
const decode = (bytes) => new TextDecoder().decode(bytes);
const json = (bytes) => JSON.parse(decode(bytes));
const settle = (ms = 5) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(what, test) {
  for (let turn = 0; turn < 400; turn += 1) {
    const value = await test();
    if (value) return value;
    await settle();
  }
  throw new Error(`never: ${what}`);
}

function linkedMuxes() {
  const pair = (other) => ({
    send: async (bytes) => other.deliver?.(new Uint8Array(bytes)),
    onMessage(take) {
      this.deliver = take;
    },
    onClose(listen) {
      this.closed = listen;
    },
  });
  const near = {};
  const far = {};
  Object.assign(near, pair(far));
  Object.assign(far, pair(near));
  return { near: createMux(near, { side: "dialed" }), far: createMux(far, { side: "answered" }), nearEnd: near, farEnd: far };
}

/** The bridge's half: every standing stream it was opened, each told it is ready. */
function fakeBridge(far, { answer = (stream) => stream.send('{"standing":1}') } = {}) {
  const opened = [];
  (async () => {
    for (;;) {
      const stream = await far.accept();
      if (stream === null) return;
      assert.equal(stream.protocol, OFFER);
      assert.equal(decode(stream.metadata), "standing/1");
      await answer(stream);
      opened.push(stream);
    }
  })();
  return opened;
}

/** A standing setup over one dial, with a setting a test can change. */
function setUp({ agent = AGENT, on = "a", answer, dial } = {}) {
  const dials = [];
  const statuses = [];
  const timers = [];
  const state = { on, links: [] };
  const makeLink = () => {
    const { near, far, nearEnd } = linkedMuxes();
    const closing = [];
    const link = {
      mux: near,
      closed: false,
      bridge: fakeBridge(far, { answer }),
      onClose: (listen) => closing.push(listen),
      close() {
        if (link.closed) return;
        link.closed = true;
        nearEnd.closed?.();
        for (const listen of closing) listen();
      },
    };
    state.links.push(link);
    return link;
  };
  const toolsMade = [];
  const standing = createStanding({
    setting: async () => state.on,
    agentOf: async (key) => (key === agent?.key ? agent : undefined),
    dial: dial ?? (async (record) => {
      dials.push(record);
      return makeLink();
    }),
    toolsFor: ({ mux, agent: record }) => {
      const tools = createTools({
        mux,
        handle: (request) => handleMcpRequest({
          request,
          peerId: record.peer,
          trustedPeers: new Set([record.peer]),
          grants: [],
          dispatch: async () => null,
          access: { capabilities: ["browser.read"], logs: true, request: async () => ({ allowed: false }) },
        }),
      });
      toolsMade.push(tools);
      return tools;
    },
    onStatus: (status) => statuses.push(status),
    setTimer: (run) => timers.push(run),
  });
  return { standing, state, dials, statuses, timers, toolsMade };
}

const claim = (stream) => stream.send('{"claimed":true}');

test("off is the default, and nothing is dialed", async () => {
  const { standing, dials, statuses } = setUp({ on: null });
  await standing.ensure();
  assert.equal(dials.length, 0);
  assert.deepEqual(statuses, [{ state: "off", peer: null }]);
});

test("turned on, it keeps one stream standing with no ACP on the link, and says so", async () => {
  const { standing, state, dials, statuses } = setUp();
  await standing.ensure();
  await until("the stream", () => state.links[0]?.bridge.length === 1);
  assert.equal(dials.length, 1);
  assert.equal(dials[0].peer, AGENT.peer);
  await until("connected", () => statuses.at(-1).state === "connected");
  assert.equal(statuses.at(-1).peer, AGENT.peer);
  await standing.ensure();
  assert.equal(dials.length, 1, "already there");
});

test("an agent that holds no key is never dialed", async () => {
  const { standing, dials, statuses } = setUp({ agent: { ...AGENT, peer: undefined } });
  await standing.ensure();
  assert.equal(dials.length, 0);
  assert.equal(statuses.at(-1).state, "unreachable");
});

test("a program taking a stream gets MCP on it and sees only the one tool, and the next stream opens", async () => {
  const { standing, state } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  await until("the stream", () => bridge.length === 1);
  await claim(bridge[0]);
  await until("the next", () => bridge.length === 2);
  await bridge[0].send(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }));
  const listed = json(await bridge[0].receive());
  assert.deepEqual(listed.result.tools.map((tool) => tool.name), ["browser_request_access"]);
  await bridge[1].send('{"claimed":true}');
  await until("a third", () => bridge.length === 3);
  await bridge[1].send(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "ping" }));
  assert.equal(json(await bridge[1].receive()).id, 2);
});

test("anything but MCP on the stream before it is claimed ends the link rather than reaching a tool", async () => {
  const { standing, state, statuses } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  await until("the stream", () => bridge.length === 1);
  await bridge[0].send(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }));
  await until("the link to end", () => state.links[0].closed);
  assert.equal(statuses.at(-1).state, "unreachable");
});

test("a stream a program has left is released, and one still stands for the next", async () => {
  const { standing, state } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  await until("the stream", () => bridge.length === 1);
  await claim(bridge[0]);
  await until("the next", () => bridge.length === 2);
  bridge[0].close();
  await settle();
  assert.equal(bridge.length, 2, "still only the one standing");
  await claim(bridge[1]);
  await until("the next", () => bridge.length === 3);
});

test("past the limit of programs nothing more opens, until one leaves", async () => {
  const { standing, state } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  for (let turn = 0; turn < MAX_PROGRAMS; turn += 1) {
    await until(`stream ${turn}`, () => bridge.length === turn + 1);
    await claim(bridge[turn]);
  }
  await settle(20);
  assert.equal(bridge.length, MAX_PROGRAMS);
  bridge[0].close();
  await until("one more", () => bridge.length === MAX_PROGRAMS + 1);
});

test("turning it off tells each program, ends every stream and the link, and does not dial again", async () => {
  const { standing, state, dials, statuses, timers } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  await until("the stream", () => bridge.length === 1);
  await claim(bridge[0]);
  await until("the next", () => bridge.length === 2);
  state.on = null;
  await standing.changed();
  // The goodbye crosses first: what is queued on a link that has gone is dropped.
  assert.equal(state.links[0].closed, false);
  assert.equal(decode(await bridge[0].receive()), GOODBYE);
  assert.equal(await bridge[0].receive(), null);
  assert.equal(statuses.at(-1).state, "off");
  for (const run of timers) await run();
  assert.equal(state.links[0].closed, true);
  await standing.ensure();
  assert.equal(dials.length, 1);
});

test("turning it off closes the free stream at once, so nothing coming in that moment is served", async () => {
  const { standing, state } = setUp();
  await standing.ensure();
  const { bridge } = state.links[0];
  await until("the stream", () => bridge.length === 1);
  await settle();
  state.on = null;
  await standing.changed();
  assert.equal(state.links[0].closed, false, "the link is still there");
  assert.equal(await bridge[0].receive(), null);
});

test("turning it on for another machine ends the first", async () => {
  const other = { ...AGENT, key: "b", peer: `sha256:${"b".repeat(64)}` };
  const records = { a: AGENT, b: other };
  const links = [];
  const state = { on: "a" };
  const standing = createStanding({
    setting: async () => state.on,
    agentOf: async (key) => records[key],
    dial: async (record) => {
      const { near, far } = linkedMuxes();
      const link = { record, mux: near, closed: false, bridge: fakeBridge(far), onClose() {}, close() { link.closed = true; } };
      links.push(link);
      return link;
    },
    toolsFor: () => ({ end() {}, adopt() {} }),
    setTimer: (run) => run(),
  });
  await standing.ensure();
  state.on = "b";
  await standing.changed();
  assert.equal(links.length, 2);
  assert.equal(links[0].closed, true);
  assert.equal(links[1].record.peer, other.peer);
});

test("a link that goes is tried again soon, then at each wake-up, with what ran on it ended", async () => {
  const { standing, state, dials, timers, statuses, toolsMade } = setUp();
  await standing.ensure();
  await until("the stream", () => state.links[0].bridge.length === 1);
  state.links[0].close();
  assert.equal(statuses.at(-1).state, "unreachable");
  assert.equal(timers.length, 1);
  await timers.shift()();
  assert.equal(dials.length, 2);
  await until("the stream", () => state.links[1].bridge.length === 1);
  assert.equal(toolsMade.length, 2);
});

test("a bridge that cannot be reached is tried a few times, then only when the worker wakes, and never holds an attempt open", async () => {
  let dials = 0;
  const { standing, timers, statuses } = setUp({
    dial: async () => {
      dials += 1;
      throw new Error("not there");
    },
  });
  await standing.ensure();
  assert.equal(statuses.at(-1).state, "unreachable");
  while (timers.length > 0) await timers.shift()();
  assert.equal(dials, 3);
  await standing.ensure();
  assert.equal(dials, 4, "a wake-up tries");
});

test("a bridge that refuses says so, and an older one's answer to a name is not taken for a standing stream", async () => {
  const refusing = setUp({ answer: (stream) => stream.close("refused", "that is no name for a server") });
  await refusing.standing.ensure();
  await until("refused", () => refusing.statuses.at(-1).state === "refused");
  assert.equal(refusing.state.links[0].closed, true);

  const older = setUp({ answer: (stream) => stream.send('{"type":"stdio","name":"x","command":"/x","args":[],"env":[]}') });
  await older.standing.ensure();
  await until("not taken", () => older.state.links[0].closed);
  assert.equal(older.statuses.at(-1).state, "unreachable");
});

test("turning it off while an attempt is running waits for it and leaves nothing", async () => {
  const links = [];
  let release;
  const state = { on: "a" };
  const standing = createStanding({
    setting: async () => state.on,
    agentOf: async () => AGENT,
    dial: () => new Promise((resolve) => {
      release = () => {
        const { near, far } = linkedMuxes();
        const link = { mux: near, closed: false, bridge: fakeBridge(far), onClose() {}, close() { link.closed = true; } };
        links.push(link);
        resolve(link);
      };
    }),
    toolsFor: () => ({ end() {}, adopt() {} }),
    setTimer: (run) => run(),
  });
  const first = standing.ensure();
  await settle();
  state.on = null;
  const changed = standing.changed();
  release();
  await first;
  await changed;
  assert.equal(links.length, 1);
  assert.equal(links[0].closed, true);
});

test("a page's scripts and the extension's other pages cannot reach the setting", () => {
  const runtime = { id: "me", getURL: (path) => `chrome-extension://me/${path}` };
  assert.equal(fromPermissionsPage({ id: "me", url: "chrome-extension://me/permissions.html" }, runtime), true);
  for (const sender of [
    undefined,
    {},
    { id: "other", url: "chrome-extension://me/permissions.html" },
    { id: "me", url: "https://a.example/permissions.html" },
    { id: "me", url: "chrome-extension://me/acp-request.html" },
    { id: "me", url: "chrome-extension://me/ui/index.html" },
    { id: "me", url: "chrome-extension://me/permissions.html?x=1" },
  ]) {
    assert.equal(fromPermissionsPage(sender, runtime), false, JSON.stringify(sender));
  }
});

test("a page has no operation that reaches this, whatever it sends", async () => {
  const sent = [];
  const port = { postMessage: (message) => sent.push(message), onMessage: { addListener(fn) { port.take = fn; } }, onDisconnect: { addListener() {} } };
  const gateway = createGateway({ store: {}, dial: async () => assert.fail("a page dialed"), askPerson: async () => null, askPermission: async () => null, ownOrigin: "chrome-extension://me" });
  gateway.attach(port, { origin: "https://a.example", tabId: 1 });
  for (const op of ["standing", "canoa.standing.set", "setStanding", "__proto__", "constructor"]) port.take({ id: 1, op, args: { agent: "a" } });
  await settle();
  assert.equal(sent.length, 5);
  for (const answer of sent) assert.equal(answer.error.name, "NotAllowedError");
});

test("the window says a program on that machine is asking, and turning this off while it waits gives no grant", async () => {
  let shown;
  let grants = [];
  let on = true;
  const access = createAccess({
    tabs: async () => [{ id: 3, title: "t", url: "https://a.example/", active: true }],
    ask: async (details) => {
      shown = details;
      on = false;
      return { tabId: 3, minutes: 15 };
    },
    updateGrants: async (change) => {
      grants = await change(grants);
    },
  });
  const answer = await access({ peer: AGENT.peer, label: AGENT.label, capability: "browser.read", reason: "why", via: "program", still: async () => on });
  assert.equal(shown.via, "program");
  assert.equal(shown.label, AGENT.label);
  assert.equal(shown.peer, AGENT.peer);
  assert.equal(answer.allowed, false);
  assert.deepEqual(grants, []);
});

test("still on as the grant is stored, it is stored; a session's own agent is asked for with nothing else", async () => {
  let grants = [];
  let shown;
  const access = createAccess({
    tabs: async () => [{ id: 3, title: "t", url: "https://a.example/", active: true }],
    ask: async (details) => {
      shown = details;
      return { tabId: 3, minutes: 15 };
    },
    updateGrants: async (change) => {
      grants = await change(grants);
    },
  });
  assert.equal((await access({ peer: AGENT.peer, label: "x", capability: "browser.read", reason: "", via: "program", still: async () => true })).allowed, true);
  assert.equal(grants.length, 1);
  assert.equal((await access({ peer: `sha256:${"b".repeat(64)}`, label: "y", capability: "browser.read", reason: "" })).allowed, true);
  assert.equal(shown.via, undefined);
  assert.equal(grants.length, 2);
});

test("a server for a program declares its logging, which a session's does not, and takes setLevel", async () => {
  const peer = AGENT.peer;
  const ask = (access, method, params = {}) => handleMcpRequest({ request: { jsonrpc: "2.0", id: 1, method, params }, peerId: peer, trustedPeers: new Set([peer]), grants: [], dispatch: async () => null, access });
  const program = { capabilities: [], logs: true, request: async () => ({}) };
  const session = { capabilities: [], logs: false, request: async () => ({}) };
  assert.deepEqual((await ask(program, "initialize")).result.capabilities.logging, {});
  assert.equal((await ask(session, "initialize")).result.capabilities.logging, undefined);
  assert.deepEqual((await ask(program, "logging/setLevel", { level: "debug" })).result, {});
  assert.equal((await ask(session, "logging/setLevel", { level: "debug" })).error.code, -32601);
});

test("a moved agent that cannot prove its new address goes back, on this path as on a page's", async () => {
  const calls = [];
  const store = { settleMove: async (key) => calls.push(["settle", key]), revertMove: async (key) => calls.push(["revert", key]) };
  const moved = { ...AGENT, url: "ws://new", previous: { url: "ws://old" } };
  const channel = await dialRecord(store, async (record) => {
    if (record.url === "ws://new") throw new Error("no proof");
    return { url: record.url };
  }, moved, {});
  assert.equal(channel.url, "ws://old");
  assert.deepEqual(calls, [["revert", "a"]]);
  await assert.rejects(dialRecord(store, async () => {
    throw new Error("no proof");
  }, AGENT, {}), (error) => error.refusal.name === "NetworkError");
});
