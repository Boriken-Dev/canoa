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
      return { allow: ["hand", "local:claude"], lifetime: "once", cwd: "/w" };
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

test("a page's window is not offered the local host's agents, and a decision naming one grants nothing", async () => {
  const { asked, grants } = await request(PAGE);
  const { agents: offered } = asked[0];
  assert.deepEqual(offered.map((agent) => agent.key), ["hand"]);
  assert.deepEqual(grants.map((grant) => grant.agent), ["hand"]);
});

test("Canoa's own chat is offered them", async () => {
  const { asked } = await request(OWN);
  const { agents: offered } = asked[0];
  assert.deepEqual(offered.map((agent) => agent.key), ["hand", "local:claude"]);
});
