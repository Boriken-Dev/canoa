// Sessions attached to tabs and tabs shared with them, in a real browser: what the panel's messages do to the extension's grants,
// the tab group around a tab an agent acts on, the toolbar badge, who may send those messages, and a session popped out to a browser
// window of its own: the window the worker opens, who holds the tab binding, and what a window may not ask. The tool call itself is
// `dispatchBrowserTool` as the worker runs it, called from an extension page with the grants the worker holds.
//
//   CANOA_E2E_BROWSER=path/to/edge [CANOA_E2E_SHOTS=dir] [CANOA_E2E_PANEL_UI=app/build/web] node e2e/panel.mjs
//
// CANOA_E2E_PANEL_UI is a `flutter build web --csp --base-href /ui/` output, which then stands in for the packaged panel: the
// run checks it starts in the extension with the worker's tab bridge beside it.
//
// Needs no agent program: the paired agent is a record, since nothing here dials it.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
// A page of the packaged UI, as the panel is, without building Flutter for it; or the built panel itself.
const builtUi = process.env.CANOA_E2E_PANEL_UI;
if (builtUi) {
  cpSync(builtUi, path.join(unpacked, "ui"), { recursive: true, filter: (from) => !from.endsWith(".map") });
  const index = path.join(unpacked, "ui", "index.html");
  // As app/tool/build.py's package_ui adds the extension's scripts.
  const scripts = ["bridge.js", "acp-page.js", "acp-content.js"].map((name) => `<script src="../${name}"></script>`).join("");
  writeFileSync(index, readFileSync(index, "utf8").replace("<head>", `<head>${scripts}`));
} else {
  mkdirSync(path.join(unpacked, "ui"));
}
writeFileSync(path.join(unpacked, "ui", "own.html"), '<!doctype html><title>own</title><script src="../bridge.js"></script><p>own</p>');
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
  await check("the panel's bridge lists web tabs and the sessions, shares from a click's permission, and unshares", async () => {
    const call = (name, ...args) => panel.run(`window.canoaBridge.tabs.call(${JSON.stringify(name)}, ${JSON.stringify(JSON.stringify(args))}).then((text) => JSON.parse(text), (error) => ({ error: error.message }))`, { gesture: true });
    await sw.run(`chrome.tabs.update(${a.tabId}, { active: true }).then(() => true)`);
    const snapshot = await call("snapshot");
    assert.ok(snapshot.tabs.some((tab) => tab.id === a.tabId && tab.url.startsWith(base)), JSON.stringify(snapshot));
    assert.ok(snapshot.tabs.every((tab) => /^https?:/.test(tab.url)));
    await call("attach", "s6", a.tabId);
    assert.deepEqual(await call("share", "s6", a.tabId, PEER), {});
    assert.equal((await act(a.tabId)).ok, true);
    const listed = (await call("snapshot")).sessions.find((item) => item.sessionId === "s6");
    assert.deepEqual(listed?.shared, [a.tabId]);
    assert.ok((await call("share", "s6", a.tabId, `sha256:${"00".repeat(32)}`)).error);
    assert.deepEqual(await call("unshare", "s6", a.tabId), {});
    assert.equal((await act(a.tabId)).ok, false);
    assert.ok((await call("claimEverything")).error);
    await call("detach", "s6");
  });
  // A session popped out: the worker opens the browser window, a window holds or takes the tab binding, and asks for nothing that grants.
  const LINK = "5f".repeat(16);
  const OWNER = `window:${LINK}`;
  const bound = async (sessionId) => (await send({ type: "canoa.tabs.list" })).sessions.find((item) => item.sessionId === sessionId);
  const asWindow = async (sessionId, link = LINK) => {
    const page = await open(`chrome-extension://${id}/ui/own.html?view=window&session=${sessionId}&popout=${link}`);
    await waitFor("the window page", () => page.run("document.readyState === 'complete' && typeof window.canoaBridge?.client === 'string'"));
    return page;
  };
  const refused = (answer) => answer === "null" || answer === "rejected" || /error/i.test(answer);
  const ask = (page, message) => page.run(`chrome.runtime.sendMessage(${JSON.stringify(message)}).then((reply) => JSON.stringify(reply ?? null), () => "rejected")`);
  let windowId;
  await check("popping a session out opens a browser window at the UI as a window, once, and the window is recorded", async () => {
    await send({ type: "canoa.tabs.attach", sessionId: "s7", tabId: a.tabId });
    await send({ type: "canoa.tabs.share", sessionId: "s7", tabId: a.tabId, peerId: PEER });
    const opened = await send({ type: "canoa.popout.open", sessionId: "s7", link: LINK, title: "Fix the build", look: "canoa", density: "normal" });
    assert.ok(Number.isSafeInteger(opened.windowId), JSON.stringify(opened));
    windowId = opened.windowId;
    const made = await sw.run(`chrome.windows.get(${windowId}, { populate: true }).then((win) => ({ type: win.type, urls: win.tabs.map((tab) => tab.pendingUrl || tab.url) }))`);
    assert.equal(made.type, "popup");
    const url = new URL(made.urls[0]);
    assert.equal(url.pathname, "/ui/index.html");
    assert.deepEqual([url.searchParams.get("view"), url.searchParams.get("session"), url.searchParams.get("popout")], ["window", "s7", LINK]);
    assert.equal((await send({ type: "canoa.popout.open", sessionId: "s7", link: LINK, title: "x", look: "canoa", density: "normal" })).windowId, windowId, "one window per session");
    assert.equal((await bound("s7")).owner, "panel", "popping out does not move the tab binding");
  });
  await check("a window sees who holds the tab and cannot share, attach, detach, open or close anything", async () => {
    const win = await asWindow("s7");
    try {
      assert.equal(await win.run("window.canoaBridge.client"), OWNER);
      const listed = await ask(win, { type: "canoa.tabs.list" });
      assert.equal(JSON.parse(listed).sessions.find((item) => item.sessionId === "s7").owner, "panel");
      for (const message of [
        { type: "canoa.tabs.share", sessionId: "s7", tabId: b.tabId, peerId: PEER },
        { type: "canoa.tabs.unshare", sessionId: "s7", tabId: a.tabId },
        { type: "canoa.tabs.attach", sessionId: "s8", tabId: b.tabId },
        { type: "canoa.tabs.detach", sessionId: "s7" },
        { type: "canoa.popout.open", sessionId: "s8", link: "6e".repeat(16), title: "x", look: "canoa", density: "normal" },
        { type: "canoa.popout.close", sessionId: "s7" }
      ]) assert.ok(refused(await ask(win, message)), JSON.stringify(message));
      assert.deepEqual(await shared("s7"), [a.tabId], "nothing it asked changed a share");
      assert.equal(await bound("s8"), undefined);
      assert.ok(await sw.run(`chrome.windows.get(${windowId}).then(() => true, () => false)`), "its window is still open");
    } finally {
      await win.call("Page.close").catch(() => {});
    }
  });
  await check("a window's Move here takes the tab binding with no resume: the same tab keeps its share, another tab ends it and grants nothing", async () => {
    const win = await asWindow("s7");
    try {
      assert.deepEqual(JSON.parse(await ask(win, { type: "canoa.tabs.move", sessionId: "s7", tabId: a.tabId })), {});
      const same = await bound("s7");
      assert.deepEqual([same.owner, same.tabId, same.shared], [OWNER, a.tabId, [a.tabId]]);
      assert.equal((await act(a.tabId)).ok, true, "the agent still reads what the panel shared");
      assert.deepEqual(JSON.parse(await ask(win, { type: "canoa.tabs.move", sessionId: "s7", tabId: b.tabId })), {});
      const moved = await bound("s7");
      assert.deepEqual([moved.owner, moved.tabId, moved.shared], [OWNER, b.tabId, []]);
      assert.equal((await act(b.tabId)).ok, false, "the window's move gave the agent nothing on the new tab");
      assert.equal((await act(a.tabId)).ok, false, "and ended what the old tab shared");
      await sw.run(`chrome.tabs.update(${b.tabId}, { active: true }).then(() => true)`);
      const front = JSON.parse(await win.run(`window.canoaBridge.tabs.call("snapshot", "[]")`)).active;
      assert.equal(front, b.tabId, "a window's front tab is the web tab used last");
    } finally {
      await win.call("Page.close").catch(() => {});
    }
  });
  await check("the panel takes the binding back, and a window that is closed hands back what it held", async () => {
    assert.deepEqual(await send({ type: "canoa.tabs.move", sessionId: "s7", tabId: a.tabId }), {});
    assert.equal((await bound("s7")).owner, "panel");
    const win = await asWindow("s7");
    try {
      await ask(win, { type: "canoa.tabs.move", sessionId: "s7", tabId: a.tabId });
      assert.equal((await bound("s7")).owner, OWNER);
    } finally {
      await win.call("Page.close").catch(() => {});
    }
    // The person closing the browser window, as the worker hears it.
    await sw.run(`chrome.windows.remove(${windowId}).then(() => true)`);
    await waitFor("the binding to return to the panel", async () => (await bound("s7")).owner === "panel");
    assert.equal((await sw.run("chrome.storage.session.get('canoa_popouts').then((held) => held.canoa_popouts ?? [])")).length, 0);
  });
  await check("docking back closes the window by its session and returns the binding; a bad request opens nothing", async () => {
    const opened = await send({ type: "canoa.popout.open", sessionId: "s7", link: LINK, title: "Again", look: "canoa", density: "normal" });
    assert.notEqual(opened.windowId, windowId, "a window opened again is a new window");
    const win = await asWindow("s7");
    try {
      await ask(win, { type: "canoa.tabs.move", sessionId: "s7", tabId: a.tabId });
      assert.equal((await bound("s7")).owner, OWNER);
      assert.deepEqual(await send({ type: "canoa.popout.close", sessionId: "s7" }), {});
      await waitFor("the window to go", async () => !(await sw.run(`chrome.windows.get(${opened.windowId}).then(() => true, () => false)`)));
      await waitFor("the binding to return to the panel", async () => (await bound("s7")).owner === "panel");
    } finally {
      await win.call("Page.close").catch(() => {});
    }
    assert.deepEqual(await send({ type: "canoa.popout.close", sessionId: "never-opened" }), {});
    for (const bad of [{ link: "short" }, { look: "a&view=panel" }, { sessionId: "" }]) {
      assert.ok((await send({ type: "canoa.popout.open", sessionId: "s7", link: LINK, title: "x", look: "canoa", density: "normal", ...bad })).error, JSON.stringify(bad));
    }
    await send({ type: "canoa.tabs.detach", sessionId: "s7" });
  });
  if (builtUi) {
    await check("the built panel starts in the extension, beside the worker's tab bridge", async () => {
      const ui = await open(`chrome-extension://${id}/ui/index.html?view=panel`);
      try {
        await waitFor("Flutter", () => ui.run("!!document.querySelector('flt-glass-pane, flutter-view')"), 60_000);
        assert.equal(await ui.run("typeof window.canoaBridge?.tabs?.call"), "function");
        await sleep(3000);
        await shoot(ui, "panel_built");
      } finally {
        await ui.call("Page.close").catch(() => {});
      }
    });
    await check("the built window page starts in the extension as a window, naming itself to the worker", async () => {
      const link = "7a".repeat(16);
      const ui = await open(`chrome-extension://${id}/ui/index.html?view=window&session=s9&popout=${link}&title=Probe`);
      try {
        await waitFor("Flutter", () => ui.run("!!document.querySelector('flt-glass-pane, flutter-view')"), 60_000);
        assert.equal(await ui.run("window.canoaBridge.client"), `window:${link}`);
        // Nothing holds that conversation, so the window says it cannot attach rather than showing another one.
        await waitFor("the window's title", () => ui.run("document.title.includes('Probe')"));
        await sleep(3000);
        await shoot(ui, "popout_window_unattached");
      } finally {
        await ui.call("Page.close").catch(() => {});
      }
    });
  }
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
