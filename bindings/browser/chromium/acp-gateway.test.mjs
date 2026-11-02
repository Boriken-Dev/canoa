// What the person decides outside any page, and what a hostile agent or page can do about it. The real gateway, with a store in
// memory and a port standing in for a page's.
import assert from "node:assert/strict";
import { test } from "node:test";

import { createGateway } from "./acp-gateway.js";

const ORIGIN = "https://site.example";
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

function harness({ dial, askPermission = async () => null } = {}) {
  const agents = [{ key: "home", label: "Home", url: "wss://home/acp", peer: "sha256:home" }];
  const grants = [{ origin: ORIGIN, agent: "home", tabId: 1, lifetime: "always", cwd: "/w", permissions: false, sessions: [] }];
  const store = {
    agents: async () => agents,
    addAgent: async (agent) => agent,
    markProven: async () => {},
    grants: async (origin) => grants.filter((grant) => grant.origin === origin),
    async saveGrant(grant) {
      const at = grants.findIndex((held) => held.origin === grant.origin && held.agent === grant.agent);
      if (at >= 0) grants[at] = grant;
      else grants.push(grant);
    },
    async deleteGrant(grant) {
      const at = grants.findIndex((held) => held.origin === grant.origin && held.agent === grant.agent);
      if (at >= 0) grants.splice(at, 1);
    },
    async removeAgent(key) {
      agents.splice(agents.findIndex((agent) => agent.key === key), 1);
      for (let at = grants.length - 1; at >= 0; at -= 1) if (grants[at].agent === key) grants.splice(at, 1);
    },
    settleMove: async (key) => {
      delete agents.find((agent) => agent.key === key).previous;
    },
    revertMove: async (key) => {
      const agent = agents.find((each) => each.key === key);
      Object.assign(agent, agent.previous);
      delete agent.previous;
    },
    secret: async () => "secret",
  };
  const channels = [];
  const dialed = [];
  const gateway = createGateway({
    store,
    dial: dial ?? (async (agent) => {
      dialed.push(agent.url);
      const channel = {
        proven: true,
        sent: [],
        closed: false,
        send: async (text) => channel.sent.push(JSON.parse(text)),
        onMessage: (handler) => {
          channel.fromAgent = (message) => handler(JSON.stringify(message));
        },
        onClose: () => {},
        close: () => {
          channel.closed = true;
        },
      };
      channels.push(channel);
      return channel;
    }),
    askPerson: async () => null,
    askPermission,
  });
  const received = [];
  const listeners = { message: [], disconnect: [] };
  gateway.attach({
    postMessage: (message) => received.push(message),
    onMessage: { addListener: (listen) => listeners.message.push(listen) },
    onDisconnect: { addListener: (listen) => listeners.disconnect.push(listen) },
  }, { origin: ORIGIN, tabId: 1 });
  let next = 1;
  const call = async (op, args) => {
    const id = next++;
    listeners.message.forEach((listen) => listen({ id, op, args }));
    for (let tries = 0; tries < 50; tries += 1) {
      await settle();
      const answer = received.find((message) => message.id === id);
      if (answer) {
        if (answer.error) throw Object.assign(new Error(answer.error.message), { name: answer.error.name });
        return answer.result;
      }
    }
    throw new Error("no answer");
  };
  const toAgent = (conn, message) => listeners.message.forEach((listen) => listen({ conn, msg: JSON.stringify(message) }));
  return { gateway, store, agents, grants, channels, dialed, received, call, toAgent };
}

async function connected(h) {
  const [agent] = await h.call("agents");
  const { conn } = await h.call("connect", { id: agent.id });
  return conn;
}

test("a person revoking outside the page closes its live link at once, and the page cannot bring the grant back", async () => {
  const h = harness();
  const conn = await connected(h);
  await h.gateway.revokeByPerson({ origin: ORIGIN, agent: "home" });
  assert.equal(h.channels[0].closed, true);
  assert.ok(h.received.some((message) => message.conn === conn && message.closed === "revoked"));
  assert.equal(h.grants.length, 0);
  h.toAgent(conn, { jsonrpc: "2.0", id: 9, method: "session/new", params: { cwd: "/x", mcpServers: [] } });
  await settle();
  h.channels[0].fromAgent?.({ jsonrpc: "2.0", id: 9, result: { sessionId: "after" } });
  await settle();
  assert.equal(h.channels[0].sent.length, 0, "nothing more reaches the agent");
  assert.equal(h.grants.length, 0, "and the grant stays gone");
  assert.deepEqual(await h.call("agents"), []);
});

test("an answer already on its way when the person revokes cannot bring the grant back", async () => {
  const h = harness();
  const conn = await connected(h);
  h.toAgent(conn, { jsonrpc: "2.0", id: 9, method: "session/new", params: { cwd: "/x", mcpServers: [] } });
  await settle();
  const late = h.channels[0].fromAgent;
  await h.gateway.revokeByPerson({ origin: ORIGIN, agent: "home" });
  late({ jsonrpc: "2.0", id: 9, result: { sessionId: "late" } });
  await settle();
  assert.equal(h.grants.length, 0);
});

test("a person removing an agent closes every link to it", async () => {
  const h = harness();
  await connected(h);
  await h.gateway.removeByPerson("home");
  assert.equal(h.channels[0].closed, true);
  assert.equal(h.agents.length, 0);
  assert.equal(h.grants.length, 0);
});

test("an agent moved to an address that does not prove the key goes back, and works", async () => {
  const dialed = [];
  const h = harness({
    dial: async (agent) => {
      dialed.push(agent.url);
      if (agent.url === "wss://stranger/acp") throw new Error("not the key");
      return { proven: true, send: async () => {}, onMessage: () => {}, onClose: () => {}, close: () => {} };
    },
  });
  Object.assign(h.agents[0], { url: "wss://stranger/acp", proven: false, previous: { url: "wss://home/acp", webtransport: undefined, proven: true } });
  await connected(h);
  assert.deepEqual(dialed, ["wss://stranger/acp", "wss://home/acp"]);
  assert.equal(h.agents[0].url, "wss://home/acp");
  assert.equal(h.agents[0].previous, undefined);
});

test("a move that proves the key sticks", async () => {
  const h = harness();
  Object.assign(h.agents[0], { url: "wss://new/acp", proven: false, previous: { url: "wss://home/acp", proven: true } });
  await connected(h);
  assert.deepEqual(h.dialed, ["wss://new/acp"]);
  assert.equal(h.agents[0].previous, undefined);
  assert.equal(h.agents[0].url, "wss://new/acp");
});

test("an agent with no move to undo still reports that it could not be reached", async () => {
  const h = harness({ dial: async () => { throw new Error("gone"); } });
  const [agent] = await h.call("agents");
  await assert.rejects(h.call("connect", { id: agent.id }), { name: "NetworkError" });
});

function permissionFrom(sessionId, id) {
  return {
    jsonrpc: "2.0", id, method: "session/request_permission",
    params: { sessionId, toolCall: { toolCallId: "t" }, options: [{ optionId: "no", name: "No", kind: "reject_once" }] },
  };
}

async function withSession(h) {
  const conn = await connected(h);
  h.toAgent(conn, { jsonrpc: "2.0", id: 1, method: "session/new", params: { cwd: "/x", mcpServers: [] } });
  await settle();
  h.channels[0].fromAgent({ jsonrpc: "2.0", id: 1, result: { sessionId: "s1" } });
  await settle();
  return conn;
}

test("an agent cannot open a window per message: one at a time, and no window for a session that is not this page's", async () => {
  let open = 0;
  let most = 0;
  let asked = 0;
  const answers = [];
  const h = harness({
    askPermission: () => new Promise((resolve) => {
      asked += 1;
      open += 1;
      most = Math.max(most, open);
      answers.push(() => {
        open -= 1;
        resolve(null);
      });
    }),
  });
  await withSession(h);
  const agent = h.channels[0];
  agent.fromAgent(permissionFrom(undefined, 100));
  agent.fromAgent(permissionFrom("someone-elses", 101));
  await settle();
  assert.equal(asked, 0, "nothing to ask about");
  assert.equal(agent.sent.filter((message) => message.id === 100 || message.id === 101).length, 2, "both answered, as refused");

  for (let id = 200; id < 240; id += 1) agent.fromAgent(permissionFrom("s1", id));
  await settle();
  assert.equal(asked, 1, "one window at a time");
  while (answers.length > 0) {
    answers.shift()();
    await settle();
  }
  assert.equal(most, 1);
  assert.ok(asked <= 9, `at most a few wait, got ${asked}`);
  assert.equal(agent.sent.filter((message) => message.id >= 200).length, 40, "every request gets an answer");
});
