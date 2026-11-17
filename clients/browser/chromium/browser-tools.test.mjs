// The injected page functions run in the tab, after the permission check: each one must refuse a page that is no longer the origin
// the grant was for. The fake `chrome` runs the function in a stand-in page, as `executeScript` would.
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { dispatchBrowserTool } from "./browser-tools.js";
import { CAPABILITY, DIRECTION, makeGrant } from "./permissions.js";

const GRANTED = "https://granted.example";
const PEER = "peer-1";

/** A tab that was `GRANTED` when checked and has since moved to `now`; returns what the page's DOM was asked. */
function installChrome(now) {
  const touched = [];
  globalThis.HTMLInputElement = class {};
  globalThis.HTMLTextAreaElement = class {};
  const element = Object.assign(new globalThis.HTMLInputElement(), {
    type: "text",
    disabled: false,
    readOnly: false,
    getClientRects: () => [{}],
    click: () => touched.push("click"),
    dispatchEvent: () => touched.push("event"),
  });
  Object.defineProperty(element, "value", { set: () => touched.push("value") });
  globalThis.getComputedStyle = () => ({ visibility: "visible" });
  globalThis.Event = class {};
  globalThis.location = { origin: now, href: now + "/" };
  globalThis.document = {
    title: "t",
    body: { innerText: "text" },
    querySelector: () => (touched.push("query"), element),
    querySelectorAll: () => (touched.push("query"), []),
    modelContext: {
      getTools: async () => (touched.push("tools"), [{ name: "tool" }]),
      executeTool: async () => (touched.push("execute"), "ran"),
    },
  };
  globalThis.chrome = {
    tabs: { get: async () => ({ id: 7, url: GRANTED + "/page", title: "" }), query: async () => [] },
    scripting: {
      executeScript: async ({ func, args }) => [{ result: await func(...args) }],
    },
  };
  return touched;
}

afterEach(() => {
  for (const name of ["chrome", "document", "location", "getComputedStyle", "HTMLInputElement", "HTMLTextAreaElement", "Event"]) {
    delete globalThis[name];
  }
});

const grantFor = (capability) => makeGrant({
  direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, origin: GRANTED, tabId: 7, capability, expiresAt: Number.MAX_SAFE_INTEGER,
});

const call = (name, args, capability) => dispatchBrowserTool({
  name, args: { tabId: 7, ...args }, peerId: PEER, trustedPeers: new Set([PEER]), grants: [grantFor(capability)],
});

const CASES = [
  ["browser_read_page", {}, CAPABILITY.READ],
  ["browser_click", { selector: "#go" }, CAPABILITY.INTERACT],
  ["browser_fill", { selector: "#in", text: "hello" }, CAPABILITY.INTERACT],
  ["browser_list_webmcp", {}, CAPABILITY.READ],
  ["browser_call_webmcp", { name: "tool", arguments: {} }, CAPABILITY.WEBMCP],
  ["browser_discover_mcp", {}, CAPABILITY.DISCOVER],
];

for (const [name, args, capability] of CASES) {
  test(`${name} does nothing in a tab that has moved to another origin`, async () => {
    const touched = installChrome("https://other.example");
    await assert.rejects(call(name, args, capability), /Origin changed/);
    assert.deepEqual(touched, []);
  });

  test(`${name} runs in the origin it was granted for`, async () => {
    const touched = installChrome(GRANTED);
    await call(name, args, capability);
    assert.ok(touched.length > 0);
  });
}
