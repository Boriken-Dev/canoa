import assert from "node:assert/strict";
import test from "node:test";

import { createOfficeToolsBridge, senderEndpoint } from "./office-tools-bridge.js";

const STREAM = "a".repeat(32);
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

function fakePort() {
  const port = {
    sent: [],
    listeners: [],
    disconnectListeners: [],
    disconnected: false,
    postMessage(message) {
      port.sent.push(message);
    },
    onMessage: { addListener: (fn) => port.listeners.push(fn) },
    onDisconnect: { addListener: (fn) => port.disconnectListeners.push(fn) },
    disconnect() {
      port.disconnected = true;
    },
    emit(message) {
      for (const fn of port.listeners) fn(message);
    },
    drop() {
      for (const fn of port.disconnectListeners) fn();
    },
  };
  return port;
}

function fakeLocalLink() {
  const link = {
    sent: [],
    closed: false,
    handlers: {},
    send: async (text) => {
      const parsed = JSON.parse(text);
      link.sent.push(parsed);
      if (parsed.type === "register") {
        setTimeout(() => link.handlers.message?.(JSON.stringify({ type: "registered", provider: parsed.provider })), 0);
      }
    },
    onMessage: (fn) => { link.handlers.message = fn; },
    onClose: (fn) => { link.handlers.close = fn; },
    close: () => { link.closed = true; },
    hear: (message) => link.handlers.message?.(JSON.stringify(message)),
  };
  return link;
}

test("senderEndpoint accepts granted top frames and matching subframes, and rejects mismatched or unverified senders", () => {
  const opts = {
    runtimeId: "ext-id",
    ownOrigin: "chrome-extension://ext-id",
    uiPrefix: "chrome-extension://ext-id/ui/",
  };
  assert.deepEqual(
    senderEndpoint({ id: "ext-id", origin: "chrome-extension://ext-id", url: "chrome-extension://ext-id/ui/index.html", frameId: 0 }, opts),
    { kind: "own", origin: "chrome-extension://ext-id", tabId: undefined },
  );
  assert.deepEqual(
    senderEndpoint({ id: "ext-id", origin: "https://pane.example", url: "https://pane.example/office/app/?mode=chat", frameId: 8, tab: { id: 5 } }, opts),
    { kind: "page", origin: "https://pane.example", tabId: 5, frameId: 8 },
  );
  assert.equal(senderEndpoint({ id: "ext-id", origin: "https://pane.example", url: "https://other.example/x", frameId: 8, tab: { id: 5 } }, opts), null);
  assert.equal(senderEndpoint({ id: "ext-id", origin: "http://pane.example", url: "http://pane.example/", frameId: 0, tab: { id: 5 } }, opts), null);
  assert.equal(senderEndpoint({ id: "other", origin: "https://pane.example", url: "https://pane.example/", frameId: 0, tab: { id: 5 } }, opts), null);
});

test("createOfficeToolsBridge registers an Office provider on @tools, forwards MCP requests, and cleans up on disconnect", async () => {
  const link = fakeLocalLink();
  const pageTools = [];
  const gateway = {
    attached: [],
    setPageTools(origin, tabId, apps) {
      pageTools.push({ origin, tabId, apps: [...apps] });
    },
    attach(port, meta) {
      gateway.attached.push({ port, meta });
    },
  };
  const bridge = createOfficeToolsBridge({ dialLocal: async () => link, gateway });
  const panePort = fakePort();
  bridge.attach(panePort, { origin: "https://pane.example", tabId: 9 });

  panePort.emit({ id: 1, op: "tools.register", args: { app: "browser" } });
  await settle();
  assert.equal(panePort.sent[0].error.name, "NotAllowedError");

  panePort.emit({ id: 2, op: "tools.register", args: { app: "word" } });
  for (let i = 0; i < 30 && !panePort.sent.some((m) => m.id === 2); i += 1) await settle();
  assert.deepEqual(panePort.sent.find((m) => m.id === 2).result, { ok: true });
  assert.deepEqual(link.sent[0], { type: "register", provider: "word" });
  assert.deepEqual(pageTools.at(-1), { origin: "https://pane.example", tabId: 9, apps: ["word"] });

  link.hear({ type: "open", stream: STREAM, session: { agent: "Claude", via: "app" } });
  link.hear({ type: "mcp", stream: STREAM, message: { jsonrpc: "2.0", id: 11, method: "tools/list", params: {} } });
  for (let i = 0; i < 30 && !panePort.sent.some((m) => m.toolsCall !== undefined); i += 1) await settle();
  const openEv = panePort.sent.find((m) => m.toolsEvent?.event?.type === "open");
  assert.equal(openEv.toolsEvent.app, "word");
  const callMsg = panePort.sent.find((m) => m.toolsCall !== undefined);
  assert.equal(callMsg.event.message.method, "tools/list");

  panePort.emit({ toolsReply: callMsg.toolsCall, message: { jsonrpc: "2.0", id: 11, result: { tools: [{ name: "word_get_selection" }] } } });
  for (let i = 0; i < 30 && !link.sent.some((m) => m.type === "mcp"); i += 1) await settle();
  assert.equal(link.sent.find((m) => m.type === "mcp").message.result.tools[0].name, "word_get_selection");

  panePort.drop();
  assert.equal(link.closed, true);
  assert.deepEqual(pageTools.at(-1), { origin: "https://pane.example", tabId: 9, apps: [] });
});

test("a pane asks for the host's web origin and gets what the native host claimed, or null", async () => {
  const gateway = { setPageTools() {}, attach() {} };
  const pane = fakePort();
  createOfficeToolsBridge({ dialLocal: async () => fakeLocalLink(), gateway, localWeb: async () => "https://localhost:7800" }).attach(pane, { origin: "https://pane.example", tabId: 1 });
  pane.emit({ id: 1, op: "host.web", args: {} });
  await settle();
  assert.deepEqual(pane.sent.find((m) => m.id === 1).result, { web: "https://localhost:7800" });

  const none = fakePort();
  createOfficeToolsBridge({ dialLocal: async () => fakeLocalLink(), gateway, localWeb: async () => { throw new Error("gone"); } }).attach(none, { origin: "https://pane.example", tabId: 1 });
  none.emit({ id: 2, op: "host.web", args: {} });
  await settle();
  assert.deepEqual(none.sent.find((m) => m.id === 2).result, { web: null });
});
