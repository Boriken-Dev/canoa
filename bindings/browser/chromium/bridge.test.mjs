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
