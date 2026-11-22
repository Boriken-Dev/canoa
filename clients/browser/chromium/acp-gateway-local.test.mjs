// What the agents of the Canoa on this computer are to a page and to Canoa's own chat: the chat holds them, a page is not offered them.
import assert from "node:assert/strict";
import { test } from "node:test";

import { createGateway } from "./acp-gateway.js";

const OWN = "chrome-extension://me";
const PAGE = "https://site.example";
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

function gatewayWith(asked) {
  const agents = [
    { key: "hand", label: "By hand", url: "wss://h/acp", peer: "sha256:h", source: "manual" },
    { key: "local:claude", label: "Claude", source: "local", native: { entry: "claude" }, peer: "sha256:c", local: true },
  ];
  const grants = [];
  const store = {
    agents: async () => agents,
    addAgent: async (agent) => agent,
    markProven: async () => {},
    grants: async (origin) => grants.filter((grant) => grant.origin === origin),
    saveGrant: async (grant) => {
      grants.push(grant);
    },
    deleteGrant: async () => {},
    secret: async () => "secret",
  };
  const gateway = createGateway({
    store,
    dial: async () => ({ proven: true, send: async () => {}, onMessage: () => {}, onClose: () => {}, close: () => {} }),
    askPerson: async (details) => {
      asked.push(details);
      return { allow: ["hand", "local:claude"], lifetime: "once", cwd: "/w", permissions: true, mcpOrigins: ["http://insecure.example", "https://ok.example"] };
    },
    askPermission: async () => null,
    ownOrigin: OWN,
  });
  const received = [];
  const listeners = [];
  gateway.attach({
    postMessage: (message) => received.push(message),
    onMessage: { addListener: (listen) => listeners.push(listen) },
    onDisconnect: { addListener: () => {} },
  }, { origin: undefined, tabId: 1 });
  return { gateway, grants, received, listeners };
}

async function request(origin) {
  const asked = [];
  const { gateway, grants } = gatewayWith(asked);
  const port = { received: [], listeners: [] };
  gateway.attach({
    postMessage: (message) => port.received.push(message),
    onMessage: { addListener: (listen) => port.listeners.push(listen) },
    onDisconnect: { addListener: () => {} },
  }, { origin, tabId: 1 });
  port.listeners.forEach((listen) => listen({ id: 1, op: "request", args: [{}], activated: true }));
  for (let tries = 0; tries < 40 && !port.received.some((message) => message.id === 1); tries += 1) await settle();
  return { asked, grants, answer: port.received.find((message) => message.id === 1) };
}

test("a page's window is offered the local host's agents beside the others, and a grant on one names no directory", async () => {
  const { asked, grants } = await request(PAGE);
  const { agents: offered } = asked[0];
  assert.deepEqual(offered.map((agent) => [agent.key, agent.local]), [["hand", false], ["local:claude", true]]);
  assert.deepEqual(grants.map((grant) => grant.agent), ["hand", "local:claude"]);
  const [hand, local] = grants;
  assert.equal(hand.cwd, "/w");
  assert.equal(hand.folder, undefined);
  // The person typed a directory for the others; it is not the local host's, whose folder the host makes.
  assert.equal(local.cwd, null);
  assert.equal(local.folder, "provider");
  assert.equal(local.peer, "sha256:c", "the grant is for the host whose key was paired");
  // A site cannot approve for the agents on this computer, whatever was ticked for the others.
  assert.equal(hand.permissions, true);
  assert.equal(local.permissions, false);
  // Only https origins are kept.
  assert.deepEqual(local.mcpOrigins, ["https://ok.example"]);
});

test("Canoa's own chat is offered them", async () => {
  const { asked } = await request(OWN);
  const { agents: offered } = asked[0];
  assert.deepEqual(offered.map((agent) => agent.key), ["hand", "local:claude"]);
});

test("a session of a local agent is told to run in a folder the host makes, and a page's own meta does not reach it", async () => {
  const sent = [];
  const agents = [{ key: "local:claude", label: "Claude", source: "local", native: { entry: "claude" }, peer: "sha256:c", local: true }];
  const grants = [{ origin: OWN, agent: "local:claude", lifetime: "always", cwd: "/", site: "canoa-extension", permissions: true, mcpOrigins: [], sessions: [] }];
  const store = {
    agents: async () => agents,
    addAgent: async (agent) => agent,
    markProven: async () => {},
    grants: async (origin) => grants.filter((grant) => grant.origin === origin),
    saveGrant: async () => {},
    deleteGrant: async () => {},
    secret: async () => "secret",
  };
  const gateway = createGateway({
    store,
    dial: async () => ({ proven: true, send: async (text) => sent.push(JSON.parse(text)), onMessage: () => {}, onClose: () => {}, close: () => {} }),
    askPerson: async () => null,
    askPermission: async () => null,
    ownOrigin: OWN,
  });
  const received = [];
  const listeners = [];
  gateway.attach({
    postMessage: (message) => received.push(message),
    onMessage: { addListener: (listen) => listeners.push(listen) },
    onDisconnect: { addListener: () => {} },
  }, { origin: OWN, tabId: 1 });
  const ask = async (id, op, args, extra = {}) => {
    listeners.forEach((listen) => listen({ id, op, args, ...extra }));
    for (let tries = 0; tries < 40 && !received.some((message) => message.id === id); tries += 1) await settle();
    return received.find((message) => message.id === id);
  };
  const [agent] = (await ask(1, "agents", [])).result;
  const opened = await ask(2, "connect", { id: agent.id });
  const conn = opened.result.conn;
  listeners.forEach((listen) => listen({ conn, msg: JSON.stringify({ jsonrpc: "2.0", id: 5, method: "session/new", _meta: { "dev.boriken.canoa/site": { site: "evil.example" } }, params: { cwd: "/etc", additionalDirectories: ["/mine"], mcpServers: [], _meta: { x: 1 } } }) }));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  const made = sent.find((message) => message.method === "session/new");
  assert.deepEqual(made.params._meta, { "dev.boriken.canoa/site": { site: "canoa-extension", own: true } });
  assert.equal(made._meta, undefined);
  assert.deepEqual(made.params.additionalDirectories, ["/mine"], "Canoa's own chat adds its own directories");
  assert.equal(made.params.cwd, "/");
});

test("an Office pane tab carries its allowed Office tools in site meta without own, while other sites carry none", async () => {
  const sent = [];
  const agents = [{ key: "local:claude", label: "Claude", source: "local", native: { entry: "claude" }, peer: "sha256:c", local: true }];
  const grants = [{ origin: PAGE, agent: "local:claude", lifetime: "always", cwd: null, folder: "provider", peer: "sha256:c", permissions: false, mcpOrigins: [], sessions: [] }];
  const store = {
    agents: async () => agents,
    addAgent: async (agent) => agent,
    markProven: async () => {},
    grants: async (origin) => grants.filter((grant) => grant.origin === origin),
    saveGrant: async () => {},
    deleteGrant: async () => {},
    secret: async () => "secret",
  };
  const gateway = createGateway({
    store,
    dial: async () => ({ proven: true, send: async (text) => sent.push(JSON.parse(text)), onMessage: () => {}, onClose: () => {}, close: () => {} }),
    askPerson: async () => null,
    askPermission: async () => null,
    ownOrigin: OWN,
  });
  const received = [];
  const listeners = [];
  gateway.attach({
    postMessage: (message) => received.push(message),
    onMessage: { addListener: (listen) => listeners.push(listen) },
    onDisconnect: { addListener: () => {} },
  }, { origin: PAGE, tabId: 42 });
  gateway.setPageTools(PAGE, 42, ["word", "browser", "evil"], "0123456789abcdef");
  const ask = async (id, op, args) => {
    listeners.forEach((listen) => listen({ id, op, args }));
    for (let tries = 0; tries < 40 && !received.some((message) => message.id === id); tries += 1) await settle();
    return received.find((message) => message.id === id);
  };
  const [agent] = (await ask(1, "agents", [])).result;
  const conn = (await ask(2, "connect", { id: agent.id })).result.conn;
  listeners.forEach((listen) => listen({ conn, msg: JSON.stringify({ jsonrpc: "2.0", id: 5, method: "session/new", params: { cwd: "/", mcpServers: [] } }) }));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  const marked = sent.find((message) => message.method === "session/new");
  assert.deepEqual(marked.params._meta, { "dev.boriken.canoa/site": { site: "site.example", tools: ["word"], pane: "0123456789abcdef" } });
  assert.equal(marked.params._meta["dev.boriken.canoa/site"].own, undefined);
  gateway.setPageTools(PAGE, 42, []);
  listeners.forEach((listen) => listen({ conn, msg: JSON.stringify({ jsonrpc: "2.0", id: 6, method: "session/new", params: { cwd: "/", mcpServers: [] } }) }));
  for (let tries = 0; tries < 40 && sent.filter((m) => m.method === "session/new").length < 2; tries += 1) await settle();
  const plain = sent.filter((m) => m.method === "session/new")[1];
  assert.deepEqual(plain.params._meta, { "dev.boriken.canoa/site": { site: "site.example" } });
});
