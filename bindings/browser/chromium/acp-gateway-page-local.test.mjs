// A web page using the agents of the Canoa on this computer: the host makes the folder from the site the browser reported, the page
// names none and sees none, and a session it opened can be reopened in the same folder.
import assert from "node:assert/strict";
import { test } from "node:test";

import { createGateway } from "./acp-gateway.js";

const OWN = "chrome-extension://me";
const SITE = "dev.boriken.canoa/site";
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));
const FOLDER = "/docs/Canoa/scratch/localhost/0000-1111";

function pageWith({ origin = "http://localhost:8080", grant = {}, agents, dialFails = false, onDial = () => {}, askMode = async () => false } = {}) {
  const local = { key: "local:claude", label: "Claude", source: "local", native: { entry: "claude" }, peer: "sha256:c", proven: true, local: true };
  const held = [{ origin, agent: local.key, lifetime: "always", cwd: null, folder: "provider", peer: "sha256:c", permissions: false, mcpOrigins: [], sessions: [], folders: {}, ...grant }];
  const saved = [];
  const sent = [];
  let fromAgent = () => {};
  const store = {
    agents: async () => agents ?? [local],
    addAgent: async (agent) => agent,
    markProven: async () => {},
    grants: async (who) => held.filter((item) => item.origin === who),
    saveGrant: async (item) => {
      saved.push(structuredClone(item));
    },
    deleteGrant: async () => {},
    secret: async () => "secret",
  };
  const gateway = createGateway({
    store,
    dial: async () => {
      onDial();
      if (dialFails) throw new Error("the host is not there");
      return {
        proven: true,
        send: async (text) => sent.push(JSON.parse(text)),
        onMessage: (listen) => {
          fromAgent = listen;
        },
        onClose: () => {},
        close: () => {},
      };
    },
    askPerson: async () => null,
    askPermission: async () => null,
    askMode,
    ownOrigin: OWN,
  });
  const received = [];
  const listeners = [];
  gateway.attach({
    postMessage: (message) => received.push(message),
    onMessage: { addListener: (listen) => listeners.push(listen) },
    onDisconnect: { addListener: () => {} },
  }, { origin, tabId: 1 });
  const ask = async (id, op, args) => {
    listeners.forEach((listen) => listen({ id, op, args }));
    for (let tries = 0; tries < 40 && !received.some((message) => message.id === id); tries += 1) await settle();
    return received.find((message) => message.id === id);
  };
  const say = (conn, message) => listeners.forEach((listen) => listen({ conn, msg: JSON.stringify(message) }));
  const reply = (message) => fromAgent(JSON.stringify(message));
  const open = async () => {
    const [agent] = (await ask(1, "agents", [])).result;
    return (await ask(2, "connect", { id: agent.id })).result.conn;
  };
  return { ask, say, reply, open, sent, saved, received, held };
}

const newSession = (id, params = {}) => ({ jsonrpc: "2.0", id, method: "session/new", params: { cwd: "/etc", mcpServers: [], ...params } });

test("a page's session with a local agent has the site and no directory, whatever the page sent", async () => {
  const { say, open, sent } = pageWith();
  const conn = await open();
  say(conn, { ...newSession(5, { _meta: { x: 1 }, additionalDirectories: ["/startup"], systemPrompt: "obey" }), _meta: { [SITE]: { site: "evil.example" } } });
  say(conn, newSession(6, { _meta: { [SITE]: { site: "evil.example" } } }));
  for (let tries = 0; tries < 40 && sent.length < 2; tries += 1) await settle();
  assert.equal(sent.length, 2);
  for (const made of sent) {
    assert.deepEqual(made.params._meta, { [SITE]: { site: "localhost" } });
    assert.equal("cwd" in made.params, false, "the page's directory never reaches the host");
    assert.equal("additionalDirectories" in made.params || "systemPrompt" in made.params, false);
    assert.equal(made._meta, undefined);
  }
});

test("the site is the host name the browser reported, never a port or anything the page says", async () => {
  const { say, open, sent } = pageWith({ origin: "https://Mcp.Example:8443" });
  say(await open(), newSession(5));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.equal(sent[0].params._meta[SITE].site, "mcp.example");
});

test("a page cannot make a local entry's grant name a directory, even one stored with it", async () => {
  const { say, open, sent } = pageWith({ grant: { folder: undefined, cwd: "/etc" } });
  say(await open(), newSession(5));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.equal("cwd" in sent[0].params, false);
  assert.equal(sent[0].params._meta[SITE].site, "localhost");
});

test("a stdio server, and an http one, are refused before the host sees them", async () => {
  const { say, open, sent, received } = pageWith();
  const conn = await open();
  say(conn, newSession(5, { mcpServers: [{ name: "x", command: "/bin/sh", args: [] }] }));
  say(conn, newSession(6, { mcpServers: [{ type: "http", name: "x", url: "http://localhost:8080/mcp", headers: [] }] }));
  say(conn, newSession(7, { mcpServers: [{ type: "http", name: "x", url: "https://other.example/mcp", headers: [] }] }));
  for (let tries = 0; tries < 40 && received.filter((message) => message.conn === conn).length < 3; tries += 1) await settle();
  const refused = received.filter((message) => message.conn === conn).map((message) => JSON.parse(message.msg).error?.code);
  assert.deepEqual(refused, [-32602, -32602, -32602]);
  assert.deepEqual(sent, []);
});

test("a server of the page's own origin over https, with its bearer, goes through; the host's folder answer does not reach the page", async () => {
  const { say, reply, open, sent, received, saved } = pageWith({ origin: "https://mcp.example" });
  const conn = await open();
  const server = { type: "http", name: "office", url: "https://mcp.example/v1/mcp", headers: [{ name: "Authorization", value: "Bearer canary" }] };
  say(conn, newSession(5, { mcpServers: [server] }));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.deepEqual(sent[0].params.mcpServers, [server]);
  reply({ jsonrpc: "2.0", id: 5, result: { sessionId: "s1", _meta: { [SITE]: { site: "mcp.example", id: "u", folder: FOLDER }, kept: 1 } } });
  for (let tries = 0; tries < 40 && !received.some((message) => message.conn === conn); tries += 1) await settle();
  const told = JSON.parse(received.find((message) => message.conn === conn).msg);
  assert.equal(told.result.sessionId, "s1");
  assert.deepEqual(told.result._meta, { kept: 1 }, "no path reaches the page");
  assert.ok(!JSON.stringify(received).includes("scratch"));
  // The grant keeps the session and its folder for a later resume, and nothing of what the page sent in it.
  await settle();
  const last = saved.at(-1);
  assert.deepEqual(last.sessions, ["s1"]);
  assert.deepEqual(last.folders, { s1: FOLDER });
  assert.ok(!JSON.stringify(saved).includes("canary"));
});

test("a session the page opened is reopened in the folder the host made, and one it did not open is refused", async () => {
  const { say, open, sent, received } = pageWith({ grant: { sessions: ["s1"], folders: { s1: FOLDER } } });
  const conn = await open();
  say(conn, { jsonrpc: "2.0", id: 5, method: "session/resume", params: { sessionId: "s1", cwd: "/etc", mcpServers: [] } });
  say(conn, { jsonrpc: "2.0", id: 6, method: "session/load", params: { sessionId: "someone-elses", cwd: FOLDER, mcpServers: [] } });
  for (let tries = 0; tries < 40 && (sent.length === 0 || !received.some((message) => message.conn === conn)); tries += 1) await settle();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].params.cwd, FOLDER);
  assert.deepEqual(sent[0].params._meta, { [SITE]: { site: "localhost" } });
  assert.equal(JSON.parse(received.find((message) => message.conn === conn).msg).error.code, -32601);
});

test("a reopened session whose folder is not known carries no directory at all", async () => {
  const { say, open, sent } = pageWith({ grant: { sessions: ["s1"], folders: {} } });
  say(await open(), { jsonrpc: "2.0", id: 5, method: "session/resume", params: { sessionId: "s1", cwd: "/etc", mcpServers: [] } });
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.equal("cwd" in sent[0].params, false);
});

test("a host that is stopped is an unreachable agent to a page holding a grant on it, not a stranger", async () => {
  const { ask } = pageWith({ dialFails: true });
  const [agent] = (await ask(1, "agents", [])).result;
  assert.equal(agent.source, "local");
  assert.equal(agent.identity.verified, true);
  const refused = await ask(2, "connect", { id: agent.id });
  assert.equal(refused.error.name, "NetworkError");
});

test("a page with no grant on a local agent sees nothing of it", async () => {
  const { ask } = pageWith({ origin: "https://other.example", grant: { origin: "https://mcp.example" } });
  assert.deepEqual((await ask(1, "agents", [])).result, []);
});

test("a page is told a name of its own for the host's key, the same each time and different for each site", async () => {
  const peers = [];
  for (const origin of ["http://localhost:8080", "https://mcp.example", "https://mcp.example"]) {
    const { ask } = pageWith({ origin });
    const [agent] = (await ask(1, "agents", [])).result;
    peers.push(agent.identity.peer);
  }
  assert.ok(peers.every((peer) => /^sha256:[0-9a-f]{64}$/.test(peer) && peer !== "sha256:c"));
  assert.notEqual(peers[0], peers[1]);
});

test("a grant made for another host's key holds nothing of this one", async () => {
  const { ask } = pageWith({ grant: { peer: "sha256:other" } });
  assert.deepEqual((await ask(1, "agents", [])).result, []);
  const { ask: without } = pageWith({ grant: { peer: undefined } });
  assert.deepEqual((await without(1, "agents", [])).result, []);
});

test("nothing the host says about itself in an answer reaches the page", async () => {
  const { say, reply, open, received } = pageWith();
  const conn = await open();
  say(conn, newSession(5));
  await settle();
  reply({ jsonrpc: "2.0", id: 5, result: { sessionId: "s1", _meta: { "dev.boriken.canoa/profile": { items: ["secret-skill"] }, "dev.boriken.canoa/site": { folder: FOLDER }, other: 1 } } });
  for (let tries = 0; tries < 40 && !received.some((message) => message.conn === conn); tries += 1) await settle();
  const told = received.find((message) => message.conn === conn).msg;
  assert.ok(!told.includes("secret-skill") && !told.includes("scratch") && !told.includes("dev.boriken"));
  assert.deepEqual(JSON.parse(told).result._meta, { other: 1 });
});

test("one origin holds only so many connections", async () => {
  const many = pageWith();
  const [one] = (await many.ask(1, "agents", [])).result;
  const results = [];
  for (let count = 0; count < 10; count += 1) results.push((await many.ask(10 + count, "connect", { id: one.id })).error?.name ?? "ok");
  assert.deepEqual(results.filter((name) => name === "ok").length, 8);
  assert.deepEqual(results.slice(8), ["NotAllowedError", "NotAllowedError"]);
});

test("a site that cannot be named is refused before anything is dialled", async () => {
  let dials = 0;
  const { ask } = pageWith({ origin: "null", grant: { origin: "null" }, onDial: () => { dials += 1; } });
  const [agent] = (await ask(1, "agents", [])).result;
  assert.equal((await ask(2, "connect", { id: agent.id })).error.name, "NotAllowedError");
  assert.equal(dials, 0);
});

const APPROVAL = "dev.boriken.canoa/approval";
const TOKEN = "connection-token";

/** A connection with a session of the page's, after the host's answer to initialize gave the token. */
async function withSession(options) {
  const world = pageWith(options);
  const conn = await world.open();
  world.say(conn, { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: 1 } });
  await settle();
  world.reply({ jsonrpc: "2.0", id: 1, result: { protocolVersion: 1, _meta: { [APPROVAL]: TOKEN } } });
  world.say(conn, newSession(2));
  await settle();
  world.reply({ jsonrpc: "2.0", id: 2, result: { sessionId: "s1", configOptions: [
    { id: "mode", category: "mode", type: "select" }, { id: "model", category: "model", type: "select" }] } });
  await settle();
  world.sent.length = 0;
  world.told = () => world.received.filter((message) => message.conn === conn).map((message) => JSON.parse(message.msg));
  return { ...world, conn };
}

const bypass = (id) => ({ jsonrpc: "2.0", id, method: "session/set_config_option", params: { sessionId: "s1", configId: "mode", value: "bypassPermissions" } });

test("the token the host gives is never told to the page", async () => {
  const { told } = await withSession();
  assert.ok(!JSON.stringify(told()).includes(TOKEN) && !JSON.stringify(told()).includes("approval"));
});

test("a page's bypass request asks the person, naming the site and the mode; Allow sends the change itself, marked, for that session", async () => {
  const asked = [];
  const { say, conn, sent, told } = await withSession({ askMode: async (details) => { asked.push(details); return true; } });
  say(conn, bypass(7));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.deepEqual(asked, [{ origin: "http://localhost:8080", site: "localhost", label: "Claude", kind: "option", option: "mode", value: "bypassPermissions" }]);
  assert.deepEqual(sent, [{ jsonrpc: "2.0", id: 7, method: "session/set_config_option", params: { sessionId: "s1", configId: "mode", value: "bypassPermissions", _meta: { [APPROVAL]: TOKEN } } }]);
  assert.ok(!JSON.stringify(told()).includes(TOKEN));
});

test("set_mode asks the same way", async () => {
  const asked = [];
  const { say, conn, sent } = await withSession({ askMode: async (details) => { asked.push(details); return true; } });
  say(conn, { jsonrpc: "2.0", id: 8, method: "session/set_mode", params: { sessionId: "s1", modeId: "auto" } });
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.equal(asked[0].kind, "mode");
  assert.equal(sent[0].params.modeId, "auto");
  assert.equal(sent[0].params._meta[APPROVAL], TOKEN);
});

test("Deny, or no answer, is a refusal and nothing reaches the host; and it asks again each time", async () => {
  let asks = 0;
  const { say, conn, sent, told } = await withSession({ askMode: async () => { asks += 1; return asks === 3; } });
  for (const id of [7, 8]) say(conn, bypass(id));
  for (let tries = 0; tries < 60 && told().filter((message) => message.error).length < 2; tries += 1) await settle();
  assert.deepEqual(told().filter((message) => message.error).map((message) => [message.id, message.error.code]), [[7, -32601], [8, -32601]]);
  assert.deepEqual(sent, []);
  say(conn, bypass(9));
  for (let tries = 0; tries < 40 && sent.length === 0; tries += 1) await settle();
  assert.equal(asks, 3, "each request is asked, nothing is remembered");
  assert.equal(sent.length, 1);
});

test("a page that puts the marker in its own message gets no further than a page without it", async () => {
  let asks = 0;
  const { say, conn, sent, told } = await withSession({ askMode: async () => { asks += 1; return false; } });
  const forged = bypass(7);
  forged.params._meta = { [APPROVAL]: TOKEN };
  forged._meta = { [APPROVAL]: TOKEN };
  say(conn, forged);
  for (let tries = 0; tries < 40 && !told().some((message) => message.error); tries += 1) await settle();
  assert.equal(asks, 1, "it was asked, as a page's request is");
  assert.deepEqual(sent, []);
});

test("an option the agent says is a model's is not asked about, and a session that is not the page's is refused unasked", async () => {
  let asks = 0;
  const { say, conn, sent, told } = await withSession({ askMode: async () => { asks += 1; return true; } });
  say(conn, { jsonrpc: "2.0", id: 7, method: "session/set_config_option", params: { sessionId: "s1", configId: "model", value: "fast" } });
  say(conn, { jsonrpc: "2.0", id: 8, method: "session/set_mode", params: { sessionId: "someone-elses", modeId: "auto" } });
  for (let tries = 0; tries < 40 && (sent.length === 0 || !told().some((message) => message.error)); tries += 1) await settle();
  assert.equal(asks, 0);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].params.configId, "model");
  assert.equal(sent[0].params._meta, undefined);
  assert.equal(told().find((message) => message.error).error.code, -32601);
});

test("a mode value that is not text is refused without asking", async () => {
  let asks = 0;
  const { say, conn, sent, told } = await withSession({ askMode: async () => { asks += 1; return true; } });
  say(conn, { ...bypass(7), params: { sessionId: "s1", configId: "mode", value: { x: 1 } } });
  for (let tries = 0; tries < 40 && !told().some((message) => message.error); tries += 1) await settle();
  assert.equal(asks, 0);
  assert.deepEqual(sent, []);
});

async function requestAs(origin, agents, decision) {
  const grants = [];
  const store = {
    agents: async () => agents, addAgent: async (agent) => agent, markProven: async () => {},
    grants: async () => decision.earlier ?? [], saveGrant: async (grant) => { grants.push(grant); }, deleteGrant: async () => {}, secret: async () => "s",
  };
  const g = createGateway({
    store, dial: async () => ({ proven: true, send: async () => {}, onMessage: () => {}, onClose: () => {}, close: () => {} }),
    askPerson: async () => decision, askPermission: async () => null, ownOrigin: OWN,
  });
  const received = [];
  const listeners = [];
  g.attach({ postMessage: (m) => received.push(m), onMessage: { addListener: (l) => listeners.push(l) }, onDisconnect: { addListener: () => {} } }, { origin, tabId: 1 });
  listeners.forEach((listen) => listen({ id: 1, op: "request", args: [{}], activated: true }));
  for (let tries = 0; tries < 40 && grants.length === 0; tries += 1) await settle();
  return grants;
}

test("agents found on this computer never take the site's say over permissions, as the host's own do not", async () => {
  const found = { key: "found", label: "Found", source: "local", peer: "sha256:f", url: "ws://127.0.0.1:9/acp" };
  const loopback = { key: "loop", label: "Loop", source: "manual", peer: "sha256:l", url: "ws://localhost:9/acp" };
  const hand = { key: "hand", label: "By hand", source: "manual", peer: "sha256:h", url: "wss://h/acp" };
  const grants = await requestAs("https://p.example", [found, loopback, hand], { allow: ["found", "loop", "hand"], lifetime: "once", cwd: "/w", permissions: true });
  assert.deepEqual(grants.map((grant) => [grant.agent, grant.permissions]), [["found", false], ["loop", false], ["hand", true]]);
});

test("a grant made again for another host key starts with no sessions or folders", async () => {
  const local = { key: "local:claude", label: "Claude", source: "local", peer: "sha256:new", proven: true, local: true };
  const earlier = [{ origin: "https://p.example", agent: "local:claude", peer: "sha256:old", sessions: ["s1"], folders: { s1: "/x" } }];
  const grants = await requestAs("https://p.example", [local], { allow: ["local:claude"], lifetime: "once", earlier });
  assert.deepEqual([grants[0].sessions, grants[0].folders, grants[0].peer], [[], {}, "sha256:new"]);
  const same = await requestAs("https://p.example", [local], { allow: ["local:claude"], lifetime: "once", earlier: [{ ...earlier[0], peer: "sha256:new" }] });
  assert.deepEqual([same[0].sessions, same[0].folders], [["s1"], { s1: "/x" }]);
});

test("a failed dial is not counted against an origin for ever", async () => {
  const failing = pageWith({ dialFails: true });
  const [one] = (await failing.ask(1, "agents", [])).result;
  for (let count = 0; count < 12; count += 1) assert.equal((await failing.ask(10 + count, "connect", { id: one.id })).error.name, "NetworkError");
});

test("what the host says of itself is taken out of every page link, one the host places or not", async () => {
  const hand = { key: "hand", label: "Hand", source: "manual", peer: "sha256:h", url: "wss://h/acp", proven: true };
  const world = pageWith({ agents: [hand], grant: { agent: "hand", folder: undefined, cwd: "/w", peer: undefined } });
  const conn = await world.open();
  world.say(conn, newSession(2));
  await settle();
  world.reply({ jsonrpc: "2.0", id: 2, result: { sessionId: "s1", _meta: { "dev.boriken.canoa/profile": { items: ["secret-skill"] }, kept: 1 } } });
  for (let tries = 0; tries < 40 && !world.received.some((message) => message.conn === conn); tries += 1) await settle();
  const told = world.received.find((message) => message.conn === conn).msg;
  assert.ok(!told.includes("secret-skill"));
  assert.deepEqual(JSON.parse(told).result._meta, { kept: 1 });
});
