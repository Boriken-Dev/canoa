import test from "node:test";
import assert from "node:assert/strict";
import { captureContext, addContext, takeContext } from "./context-transfer.js";

test("captures a user-selected page and redacts URL secrets", () => {
  const got = captureContext({
    pageUrl: "https://example.com/path?access_token=secret#session",
    selectionText: "highlighted",
  }, {id: 7, title: "Test", url: "https://example.com/path"}, 123);
  assert.deepEqual(got, {
    tabId: 7, url: "https://example.com/path", origin: "https://example.com",
    title: "Test", selection: "highlighted", capturedAt: 123
  });
});
test("unsupported pages do not become agent context", () => {
  assert.equal(captureContext({pageUrl:"chrome://settings"}, {id:1}), null);
  assert.equal(captureContext({pageUrl:"file:///tmp/passwords"}, {id:1}), null);
  assert.equal(captureContext({pageUrl:"javascript:alert(1)"}, {id:1}), null);
});
test("queue bounds and consumes context once", () => {
  let q = [];
  for (let i=0; i<12; i++) q=addContext(q, {tabId:i});
  assert.equal(q.length,10);
  assert.equal(takeContext(q).context.tabId,2);
  assert.equal(takeContext(q).remaining.length,9);
});
