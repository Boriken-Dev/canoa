// What docs/browser.md settles beyond the conformance checks, against the real page script, content script and gateway.
import assert from "node:assert/strict";
import { test } from "node:test";

import { makeDriver } from "./acp-driver.mjs";

const RPC = (id, method, params) => JSON.stringify({ jsonrpc: "2.0", id, method, params });
const later = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
// Waits for a condition instead of a fixed time, which a loaded machine can overrun.
async function until(condition, message) {
  for (let waited = 0; !condition(); waited += 5) {
    if (waited > 3000) assert.fail(message);
    await later(5);
  }
}

async function granted(driver, origin, { label = "agent", lifetime = "always", tabId = 1, cwd = "/granted", permissions } = {}) {
  driver.discover(label);
  const page = driver.page(origin, { tabId });
  page.activate();
  driver.person.answerNext({ allow: label, cwd, lifetime, permissions });
  const [agent] = await page.acp.request();
  return { page, agent, label };
}

async function connected(driver, origin, options) {
  const made = await granted(driver, origin, options);
  const connection = await made.page.acp.connect(made.agent);
  const writer = connection.writable.getWriter();
  const reader = connection.readable.getReader();
  const read = async (id) => {
    for (;;) {
      const message = JSON.parse((await reader.read()).value);
      if (message.id === id) return message;
    }
  };
  return {
    ...made,
    connection,
    writer,
    reader,
    seen: driver.agent(made.label),
    open: async (id) => {
      await writer.write(RPC(id, "session/new", { cwd: "/x", mcpServers: [] }));
      return (await read(id)).result.sessionId;
    },
  };
}

const rejection = async (promise) => {
  try {
    await promise;
  } catch (error) {
    return error?.name;
  }
  return "resolved";
};

test("connect() with a signal that is already aborted rejects with AbortError and dials nothing", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { page, agent } = await granted(driver, "https://a.example");
  const control = new AbortController();
  control.abort(new Error("any reason at all"));
  assert.equal(await rejection(page.acp.connect(agent, { signal: control.signal })), "AbortError");
  await later();
  assert.equal(driver.dials, 0);
});

test("aborting connect() while it waits rejects with AbortError and ends what was being opened", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { page, agent } = await granted(driver, "https://a.example");
  const release = driver.holdDials();
  const control = new AbortController();
  const waiting = page.acp.connect(agent, { signal: control.signal });
  await later();
  control.abort();
  assert.equal(await rejection(waiting), "AbortError");
  release();
  await later();
  assert.equal(driver.channels.length, 1);
  assert.equal(driver.channels[0].closedByGateway, true, "the link that finished opening was kept");
});

test("a signal does nothing once connect() has resolved", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { page, agent } = await granted(driver, "https://a.example");
  const control = new AbortController();
  const connection = await page.acp.connect(agent, { signal: control.signal });
  control.abort();
  const state = await Promise.race([connection.closed.then(() => "closed"), later(30).then(() => "open")]);
  assert.equal(state, "open");
});

test("readable.cancel() ends the connection as page, and what the agent says after it is dropped without a throw", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { connection, reader, seen, open } = await connected(driver, "https://a.example");
  const sessionId = await open(1);
  const failures = [];
  const catcher = (error) => failures.push(error);
  process.on("uncaughtException", catcher);
  process.on("unhandledRejection", catcher);
  try {
    await reader.cancel();
    assert.equal((await connection.closed).reason, "page");
    seen.send({ jsonrpc: "2.0", method: "session/update", params: { sessionId, update: { sessionUpdate: "agent_message_chunk" } } });
    await later(40);
  } finally {
    process.off("uncaughtException", catcher);
    process.off("unhandledRejection", catcher);
  }
  assert.deepEqual(failures, []);
  assert.equal(driver.channels[0].closedByGateway, true, "the link stayed open");
});

test("writable.close() and writable.abort() end the connection as page, and the gateway closes the link", { timeout: 4000 }, async () => {
  for (const end of [(grant) => grant.writer.close(), (grant) => grant.writer.abort(new Error("gave up"))]) {
    const driver = makeDriver();
    const grant = await connected(driver, "https://a.example");
    await end(grant);
    assert.equal((await grant.connection.closed).reason, "page");
    await later();
    assert.equal(driver.channels[0].closedByGateway, true);
  }
});

test("a write after the connection ended rejects with InvalidStateError and sends nothing", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { connection, writer, seen } = await connected(driver, "https://a.example");
  connection.close();
  await connection.closed;
  const received = seen.received().length;
  assert.equal(await rejection(writer.write(RPC(1, "initialize", { protocolVersion: 1 }))), "InvalidStateError");
  await later();
  assert.equal(seen.received().length, received);
});

test("a grant of lifetime once is seen only by the tab it was made in, across that tab's reloads", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { agent } = await granted(driver, "https://a.example", { lifetime: "once", tabId: 1 });
  const reloaded = driver.page("https://a.example", { tabId: 1 });
  assert.deepEqual(Array.from(await reloaded.acp.agents(), (held) => held.id), [agent.id]);
  const other = driver.page("https://a.example", { tabId: 2 });
  assert.equal((await other.acp.agents()).length, 0);
  assert.equal(await rejection(other.acp.connect(agent)), "NotAllowedError");
  await other.acp.revoke(agent);
  assert.equal((await reloaded.acp.agents()).length, 1, "another tab's revoke took the grant away");
  // That tab asks for its own, which leaves the first tab's alone.
  other.activate();
  driver.person.answerNext({ allow: "agent", cwd: "/granted", lifetime: "once" });
  assert.equal((await other.acp.request()).length, 1);
  assert.equal((await reloaded.acp.agents()).length, 1);
});

test("an always grant serves every tab of the origin", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  await granted(driver, "https://a.example", { lifetime: "always", tabId: 1 });
  assert.equal((await driver.page("https://a.example", { tabId: 2 }).acp.agents()).length, 1);
});

test("the page's session/cancel closes the permission window for that session and answers the agent cancelled", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  driver.person.holdPermissions();
  const { writer, seen, open } = await connected(driver, "https://a.example");
  const session = await open(1);
  seen.send({ jsonrpc: "2.0", id: 900, method: "session/request_permission", params: { sessionId: session, toolCall: { toolCallId: "t" }, options: [] } });
  seen.send({ jsonrpc: "2.0", id: 901, method: "session/request_permission", params: { sessionId: session, toolCall: { toolCallId: "u" }, options: [] } });
  await later();
  assert.equal(driver.person.permissionWindows.length, 1, "one window at a time");
  await writer.write(JSON.stringify({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: session } }));
  await later(40);
  assert.equal(driver.person.permissionWindows[0].closed, true, "the window stayed open");
  assert.equal(driver.person.permissionWindows.length, 1, "the queued request opened a window after the cancel");
  const answers = seen.received().filter((message) => message.method === undefined);
  assert.deepEqual(answers.map((answer) => [answer.id, answer.result.outcome.outcome]).sort(), [[900, "cancelled"], [901, "cancelled"]]);
  assert.ok(seen.received().some((message) => message.method === "session/cancel"), "the cancel did not reach the agent");
});

test("a session/cancel for another session leaves a permission window open", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  driver.person.holdPermissions();
  const { writer, seen, open } = await connected(driver, "https://a.example");
  const first = await open(1);
  await open(2);
  seen.send({ jsonrpc: "2.0", id: 900, method: "session/request_permission", params: { sessionId: first, toolCall: { toolCallId: "t" }, options: [] } });
  await later();
  await writer.write(JSON.stringify({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: "s2" } }));
  await later(40);
  assert.equal(driver.person.permissionWindows[0].closed, false);
});

test("requesting an agent already granted ends the connections on the old grant, and the old grant is never written back", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  const { page, agent, connection, writer, open } = await connected(driver, "https://a.example", { cwd: "/old" });
  const session = await open(1);
  page.activate();
  driver.person.answerNext({ allow: "agent", cwd: "/new", lifetime: "always" });
  const [again] = await page.acp.request();
  assert.equal(again.id, agent.id);
  assert.equal((await connection.closed).reason, "revoked");
  // Whatever the old connection does now reaches nothing, and the stored grant is the new one with its sessions kept.
  await writer.write(RPC(2, "session/new", { cwd: "/x", mcpServers: [] })).catch(() => {});
  await later(40);
  const held = await driver.store.grants("https://a.example");
  assert.equal(held.length, 1);
  assert.equal(held[0].cwd, "/new");
  assert.deepEqual(held[0].sessions, [session]);
});

test("a request window closes when its page goes, and what the person decides after that grants nothing", { timeout: 4000 }, async () => {
  const driver = makeDriver();
  driver.discover("agent");
  const page = driver.page("https://a.example");
  page.activate();
  driver.person.answerNext({ hold: true });
  page.acp.request().catch(() => {});
  await until(() => driver.person.windows.length === 1, "no request window opened");
  page.navigate();
  await until(() => driver.person.windows[0].closed, "the window stayed open after its page went");
  driver.person.windows[0].answer({ allow: "agent", cwd: "/granted", lifetime: "always" });
  await later(40);
  assert.deepEqual(await driver.store.grants("https://a.example"), []);
});

test("a link ends as agent when the agent ended it in order, and as failed when it was cut", { timeout: 4000 }, async () => {
  for (const [ending, reason] of [["closed", "agent"], ["cut", "failed"], [undefined, "agent"]]) {
    const driver = makeDriver();
    const { connection, seen } = await connected(driver, "https://a.example");
    seen.hangUp(ending);
    assert.equal((await connection.closed).reason, reason, `${ending}`);
  }
});
