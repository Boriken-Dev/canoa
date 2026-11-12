// Sessions attached to tabs and tabs shared with them, in a real browser: what the panel's messages do to the extension's grants,
// the tab group around a tab an agent acts on, the toolbar badge, and who may send those messages. The tool call itself is
// `dispatchBrowserTool` as the worker runs it, called from an extension page with the grants the worker holds.
//
//   CANOA_E2E_BROWSER=path/to/edge [CANOA_E2E_SHOTS=dir] node e2e/panel.mjs
//
// Needs no agent program: the paired agent is a record, since nothing here dials it.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { findCanoaWorker, lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const browser = process.env.CANOA_E2E_BROWSER;
if (!browser) throw new Error("set CANOA_E2E_BROWSER");

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-panel-"));
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
manifest.host_permissions = ["http://127.0.0.1/*"];
delete manifest.side_panel;
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));
// A page of the packaged UI, as the panel is, without building Flutter for it.
mkdirSync(path.join(unpacked, "ui"));
writeFileSync(path.join(unpacked, "ui", "own.html"), "<!doctype html><title>own</title><p>own</p>");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const site = createServer((request, response) => {
  response.writeHead(200, { "content-type": "text/html" });
  response.end(`<!doctype html><title>page ${request.url}</title><p>page</p>`);
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${site.address().port}`;

const chrome = spawn(browser, [
  "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
  "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
], { stdio: "ignore" });

let debug;
const targets = async () => (await fetch(`http://127.0.0.1:${debug}/json/list`)).json();
async function waitFor(what, test, ms = 15_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}
const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 500)}`);
  }
  console.log(results.at(-1));
}

let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome.kill();
  process.exit(1);
}, 240_000);
try {
  debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const worker = await waitFor("the extension's service worker", async () => findCanoaWorker(await targets()), 20_000);
  const sw = new Session(worker.webSocketDebuggerUrl);
  await waitFor("the extension's APIs", async () => (await sw.run("typeof chrome")) === "object");
  const id = new URL(worker.url).hostname;
  const open = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
    const session = new Session(made.webSocketDebuggerUrl);
    await session.call("Runtime.enable");
    return session;
  };
  const shoot = async (session, name) => {
    if (!process.env.CANOA_E2E_SHOTS) return;
    mkdirSync(process.env.CANOA_E2E_SHOTS, { recursive: true });
    const { data } = await session.call("Page.captureScreenshot", { format: "png" });
    writeFileSync(path.join(process.env.CANOA_E2E_SHOTS, `${name}.png`), Buffer.from(data, "base64"));
  };

  // The paired agent: a record, added the way the agents window adds one.
  const PEER = `sha256:${"ab".repeat(32)}`;
  const agents = await open(`chrome-extension://${id}/acp-request.html?manage=1`);
  await waitFor("the agents window", () => agents.run("document.readyState === 'complete' && typeof chrome?.runtime?.sendMessage === 'function'"));
  const added = await agents.run(`chrome.runtime.sendMessage({ acp: "add", agent: { label: "Panel agent", url: "ws://127.0.0.1:9/x", peer: "${PEER}", method: "noise" } })`);
  assert.ok(!added?.error, JSON.stringify(added));

  const panel = await open(`chrome-extension://${id}/ui/own.html`);
  await waitFor("the UI page", () => panel.run("document.readyState === 'complete'"));
  const send = (message) => panel.run(`chrome.runtime.sendMessage(${JSON.stringify(message)})`);
  const tabs = async (...paths) => {
    const made = [];
    for (const suffix of paths) {
      const session = await open(`${base}${suffix}`);
      await waitFor("the page", () => session.run("document.readyState === 'complete'"));
      made.push(session);
    }
    const lookup = async () => {
      const all = await sw.run(`chrome.tabs.query({}).then((tabs) => tabs.filter((tab) => (tab.url || tab.pendingUrl || "").startsWith(${JSON.stringify(base)})).map((tab) => [tab.url || tab.pendingUrl, tab.id]))`);
      const ids = paths.map((suffix) => all.find(([url]) => url === `${base}${suffix}`)?.[1]);
      return ids.every((tabId) => tabId !== undefined) ? ids : null;
    };
    const ids = await waitFor("the tabs", lookup);
    return ids.map((tabId, index) => ({ tabId, session: made[index] }));
  };
  // The tools as the worker runs them, called from the UI page with the grants the worker holds, then the group the provider makes.
  const act = (tabId, name = "browser_read_page") => panel.run(`(async () => {
    const tools = await import(chrome.runtime.getURL("browser-tools.js"));
    const { readGrants } = await import(chrome.runtime.getURL("browser-grants.js"));
    const { markActing } = await import(chrome.runtime.getURL("tab-marks.js"));
    try {
      const result = await tools.dispatchBrowserTool({ name: ${JSON.stringify(name)}, args: { tabId: ${tabId} }, peerId: "${PEER}", trustedPeers: new Set(["${PEER}"]), grants: await readGrants() });
      await markActing({ tabId: ${tabId}, peer: "${PEER}", label: "Panel agent" });
      return { ok: true, title: result?.title };
    } catch (error) {
      return { ok: false, why: String(error.message) };
    }
  })()`);
  const groupTitleOf = (tabId) => sw.run(`chrome.tabs.get(${tabId}).then((tab) => tab.groupId === chrome.tabGroups.TAB_GROUP_ID_NONE ? null : chrome.tabGroups.get(tab.groupId).then((group) => [group.title, group.color]))`);
  const badge = (tabId) => sw.run(`chrome.action.getBadgeText({ tabId: ${tabId} })`);
  const shared = async (sessionId) => (await send({ type: "canoa.tabs.list" })).sessions.find((item) => item.sessionId === sessionId)?.shared ?? [];

  const [a, b] = await tabs("/a", "/b");
  await check("attaching a session to a tab grants the agent nothing", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.attach", sessionId: "s1", tabId: a.tabId }), {});
    assert.equal((await act(a.tabId)).ok, false);
    assert.equal(await groupTitleOf(a.tabId), null);
  });
  await check("sharing a tab lets the paired agent read it, until it is taken away, and groups and marks the tab", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.share", sessionId: "s1", tabId: a.tabId, peerId: PEER }), {});
    const grants = await sw.run("chrome.storage.session.get('canoa_grants').then((held) => held.canoa_grants)");
    assert.equal(grants.length, 1);
    assert.equal(grants[0].expiresAt, Number.MAX_SAFE_INTEGER);
    assert.equal(grants[0].capability, "browser.read");
    const read = await act(a.tabId);
    assert.equal(read.ok, true, JSON.stringify(read));
    assert.equal(read.title, "page /a");
    assert.deepEqual(await groupTitleOf(a.tabId), ["Panel agent", "cyan"]);
    await waitFor("the badge", async () => (await badge(a.tabId)) === "•");
    assert.deepEqual(await shared("s1"), [a.tabId]);
    assert.equal((await act(b.tabId)).ok, false);
  });
  await check("the agent cannot interact with a shared tab: sharing is read only", async () => {
    const clicked = await act(a.tabId, "browser_click");
    assert.equal(clicked.ok, false);
    assert.ok(clicked.why.includes("denied"), clicked.why);
  });
  await check("the sessions of an origin are listed, and another origin's are not", async () => {
    assert.deepEqual((await send({ type: "canoa.tabs.sessionsFor", origin: base })).sessions, ["s1"]);
    assert.deepEqual((await send({ type: "canoa.tabs.sessionsFor", origin: "https://elsewhere.example" })).sessions, []);
  });
  await check("a share needs a paired agent and a web tab", async () => {
    assert.ok((await send({ type: "canoa.tabs.share", sessionId: "s1", tabId: b.tabId, peerId: `sha256:${"00".repeat(32)}` })).error);
    const blank = await open("about:blank");
    const blankId = (await sw.run("chrome.tabs.query({}).then((tabs) => tabs.filter((tab) => tab.url === 'about:blank').map((tab) => tab.id))"))[0];
    assert.ok((await send({ type: "canoa.tabs.share", sessionId: "s1", tabId: blankId, peerId: PEER })).error);
    await blank.call("Page.close").catch(() => {});
  });
  await check("messages from any page of the extension but the UI are not answered", async () => {
    const options = await open(`chrome-extension://${id}/permissions.html`);
    try {
      await waitFor("the options page", () => options.run("document.readyState === 'complete'"));
      const answer = await options.run(`chrome.runtime.sendMessage({ type: "canoa.tabs.share", sessionId: "s9", tabId: ${b.tabId}, peerId: "${PEER}" }).then((reply) => JSON.stringify(reply ?? null), (error) => "rejected")`);
      assert.ok(answer === "null" || answer === "rejected", answer);
      assert.deepEqual(await shared("s9"), []);
    } finally {
      await options.call("Page.close").catch(() => {});
    }
  });
  await check("unsharing takes the read away, ungroups the tab and clears its badge", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.unshare", sessionId: "s1", tabId: a.tabId }), {});
    assert.equal((await act(a.tabId)).ok, false);
    await waitFor("the tab to leave its group", async () => (await groupTitleOf(a.tabId)) === null);
    await waitFor("the badge to clear", async () => (await badge(a.tabId)) === "");
  });
  await check("a session may share any number of tabs, and a closed tab's share goes with it", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.attach", sessionId: "s2", tabId: a.tabId }), {});
    const more = await tabs(...Array.from({ length: 11 }, (_, index) => `/m${index}`));
    // A tab still loading has no address yet, and is refused until it has one.
    for (const tab of [a, b, ...more]) {
      await waitFor("the share", async () => Object.keys(await send({ type: "canoa.tabs.share", sessionId: "s2", tabId: tab.tabId, peerId: PEER })).length === 0);
    }
    assert.equal((await shared("s2")).length, 13);
    for (const tab of more) await tab.session.call("Page.close").catch(() => {});
    await waitFor("the closed tabs' shares to go", async () => (await shared("s2")).length === 2);
  });
  await check("navigating a shared tab ends the share, and so does its session ending", async () => {
    assert.equal((await act(a.tabId)).ok, true);
    await a.session.run("location.href = '/a2'; true");
    await waitFor("the share to end", async () => !(await shared("s2")).includes(a.tabId));
    assert.equal((await act(a.tabId)).ok, false);
    assert.equal((await act(b.tabId)).ok, true);
    assert.deepEqual(await send({ type: "canoa.tabs.detach", sessionId: "s2" }), {});
    assert.equal((await act(b.tabId)).ok, false);
    await waitFor("the group to go", async () => (await groupTitleOf(b.tabId)) === null);
  });
  await check("a tab the person already put in a group of their own is not moved", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.attach", sessionId: "s3", tabId: b.tabId }), {});
    assert.deepEqual(await send({ type: "canoa.tabs.share", sessionId: "s3", tabId: b.tabId, peerId: PEER }), {});
    await sw.run(`chrome.tabs.group({ tabIds: [${b.tabId}] }).then((groupId) => chrome.tabGroups.update(groupId, { title: "Mine" })).then(() => true)`);
    assert.equal((await act(b.tabId)).ok, true);
    assert.deepEqual((await groupTitleOf(b.tabId))?.[0], "Mine");
    await send({ type: "canoa.tabs.detach", sessionId: "s3" });
  });
  await check("a detach with no session ends no one's grants", async () => {
    const ordinary = { direction: "agent-to-page", peerId: PEER, tabId: b.tabId, origin: base, capability: "browser.read", expiresAt: Date.now() + 600_000 };
    await sw.run(`chrome.storage.session.get("canoa_grants").then((held) => chrome.storage.session.set({ canoa_grants: [...(held.canoa_grants ?? []), ${JSON.stringify(ordinary)}] })).then(() => true)`);
    for (const message of [{ type: "canoa.tabs.detach" }, { type: "canoa.tabs.detach", sessionId: "" }, { type: "canoa.tabs.detach", sessionId: 7 }]) {
      assert.ok((await send(message)).error, JSON.stringify(message));
    }
    assert.equal((await act(b.tabId)).ok, true);
    await sw.run(`chrome.storage.session.set({ canoa_grants: [] }).then(() => true)`);
  });
  await check("a tab the person moved to a group of their own stays there when the share ends", async () => {
    await sw.run(`chrome.tabs.ungroup(${b.tabId}).then(() => true)`);
    await send({ type: "canoa.tabs.attach", sessionId: "s4", tabId: b.tabId });
    await send({ type: "canoa.tabs.share", sessionId: "s4", tabId: b.tabId, peerId: PEER });
    assert.equal((await act(b.tabId)).ok, true);
    assert.equal((await groupTitleOf(b.tabId))?.[0], "Panel agent");
    await sw.run(`chrome.tabs.group({ tabIds: [${b.tabId}] }).then((groupId) => chrome.tabGroups.update(groupId, { title: "Moved" })).then(() => true)`);
    await send({ type: "canoa.tabs.unshare", sessionId: "s4", tabId: b.tabId });
    await sleep(1000);
    assert.equal((await groupTitleOf(b.tabId))?.[0], "Moved");
    await sw.run(`chrome.tabs.ungroup(${b.tabId}).then(() => true)`);
    await send({ type: "canoa.tabs.detach", sessionId: "s4" });
  });
  await check("when the agent's group has been closed, the next tool call makes a new one", async () => {
    await send({ type: "canoa.tabs.attach", sessionId: "s5", tabId: b.tabId });
    await send({ type: "canoa.tabs.share", sessionId: "s5", tabId: b.tabId, peerId: PEER });
    assert.equal((await act(b.tabId)).ok, true);
    await sw.run(`chrome.tabs.ungroup(${b.tabId}).then(() => true)`);
    assert.equal((await act(b.tabId)).ok, true);
    assert.deepEqual((await groupTitleOf(b.tabId))?.[0], "Panel agent");
    await send({ type: "canoa.tabs.detach", sessionId: "s5" });
  });
  await shoot(panel, "panel_page");
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`ABORTED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  chrome.kill();
  site.close();
  await sleep(500);
  rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
console.log(exitCode === 0 ? "PASS" : "FAILED");
process.exit(exitCode);
