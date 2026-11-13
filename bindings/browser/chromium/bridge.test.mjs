import test from "node:test";
import assert from "node:assert/strict";

const tabs = [
  { id: 1, title: "Docs", url: "https://docs.example/a", active: true },
  { id: 2, title: "Settings", url: "chrome://settings" },
  { id: 3, title: "Mail", url: "http://mail.example/" }
];
const sent = [];
let granted = true;
globalThis.window = {};
globalThis.chrome = {
  runtime: {
    sendMessage: async (message) => {
      sent.push(message);
      return message.type === "canoa.tabs.list" ? { sessions: [{ sessionId: "s1", tabId: 1, origin: "https://docs.example", shared: [3] }] } : {};
    },
    getURL: (path) => `chrome-extension://x/${path}`
  },
  tabs: {
    query: async (filter) => (filter.active ? [tabs[0]] : tabs),
    get: async (id) => tabs.find((tab) => tab.id === id),
    onUpdated: { addListener() {} }, onRemoved: { addListener() {} }, onActivated: { addListener() {} }, onCreated: { addListener() {} }
  },
  permissions: { request: async () => granted },
  storage: { onChanged: { addListener() {} } }
};
await import("./bridge.js");
const call = async (name, ...args) => JSON.parse(await window.canoaBridge.tabs.call(name, JSON.stringify(args)));

test("the snapshot lists web tabs only, the one in front, and the worker's sessions", async () => {
  const snapshot = await call("snapshot");
  assert.deepEqual(snapshot.tabs.map((tab) => tab.id), [1, 3]);
  assert.equal(snapshot.active, 1);
  assert.deepEqual(snapshot.sessions[0].shared, [3]);
});

test("sharing asks the browser for the site first, and a refusal shares nothing", async () => {
  sent.length = 0;
  granted = false;
  await assert.rejects(call("share", "s1", 3, "sha256:aa"), /host access was refused/);
  assert.equal(sent.length, 0);
  granted = true;
  await call("share", "s1", 3, "sha256:aa");
  assert.deepEqual(sent.at(-1), { type: "canoa.tabs.share", sessionId: "s1", tabId: 3, peerId: "sha256:aa" });
});

test("a tab that is not a web page cannot be shared, and an unknown call is refused", async () => {
  await assert.rejects(call("share", "s1", 2, "sha256:aa"), /cannot be shared/);
  await assert.rejects(window.canoaBridge.tabs.call("claimEverything", "[]"), /Unknown call/);
  await assert.rejects(window.canoaBridge.tabs.call("constructor", "[]"), /Unknown call/);
});

test("a worker's refusal reaches the panel as an error", async () => {
  const original = chrome.runtime.sendMessage;
  chrome.runtime.sendMessage = async () => ({ error: "Invalid browser permission" });
  await assert.rejects(call("attach", "s1", 1), /Invalid browser permission/);
  chrome.runtime.sendMessage = original;
});

test("move and the pop-out calls reach the worker as the panel sends them, and the panel is the client", async () => {
  sent.length = 0;
  assert.equal(window.canoaBridge.client, "panel");
  await call("move", "s1", 3);
  assert.deepEqual(sent.at(-1), { type: "canoa.tabs.move", sessionId: "s1", tabId: 3 });
  const popout = (name, ...args) => window.canoaBridge.popout.call(name, JSON.stringify(args));
  await popout("open", "s1", "ab".repeat(16), "Title", "canoa", "normal");
  assert.deepEqual(sent.at(-1), { type: "canoa.popout.open", sessionId: "s1", link: "ab".repeat(16), title: "Title", look: "canoa", density: "normal" });
  await popout("close", "s1");
  assert.deepEqual(sent.at(-1), { type: "canoa.popout.close", sessionId: "s1" });
  await assert.rejects(popout("constructor"), /Unknown call/);
});

test("a window of its own knows itself by its link, and its front tab is the web tab used last", async () => {
  const link = "cd".repeat(16);
  globalThis.location = { search: `?view=window&session=s1&popout=${link}` };
  const original = chrome.tabs.query;
  chrome.tabs.query = async (filter) => (filter.active ? [{ ...tabs[2], lastAccessed: 9 }, { ...tabs[0], lastAccessed: 5 }, { id: 8, url: "chrome-extension://x/ui/index.html", lastAccessed: 99 }] : tabs);
  try {
    await import("./bridge.js?window");
    assert.equal(window.canoaBridge.client, `window:${link}`);
    const snapshot = JSON.parse(await window.canoaBridge.tabs.call("snapshot", "[]"));
    assert.equal(snapshot.active, 3);
  } finally {
    chrome.tabs.query = original;
    delete globalThis.location;
  }
});
