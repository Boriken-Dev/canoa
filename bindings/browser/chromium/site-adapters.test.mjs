import test from "node:test";
import assert from "node:assert/strict";
import { adaptersForOrigin, validateAdapterManifest } from "./site-adapters.js";

const manifest = {
  id: "shop-adapter",
  version: "0.1.0",
  runtime: "javascript",
  entrypoint: "scripts/adapter.js",
  origins: ["https://shop.example", "https://checkout.example"],
  tools: [{
    name: "search_products",
    description: "Search the shop",
    inputSchema: { type: "object", properties: { query: { type: "string" } } }
  }]
};

test("accepts an adapter bound to multiple exact origins", () => {
  const value = validateAdapterManifest(manifest);
  assert.deepEqual(value.origins, ["https://shop.example", "https://checkout.example"]);
  assert.equal(adaptersForOrigin([value], "https://checkout.example").length, 1);
  assert.equal(adaptersForOrigin([value], "https://elsewhere.example").length, 0);
});

test("accepts Wasm entrypoint without treating it as runnable", () => {
  const value = validateAdapterManifest({
    ...manifest, runtime: "wasm", entrypoint: "module.wasm"
  });
  assert.equal(value.runtime, "wasm");
});

test("rejects wildcard, URLs and file traversal", () => {
  assert.throws(() => validateAdapterManifest({
    ...manifest, origins: ["https://*.example"]
  }), TypeError);
  assert.throws(() => validateAdapterManifest({
    ...manifest, entrypoint: "../evil.js"
  }), TypeError);
  assert.throws(() => validateAdapterManifest({
    ...manifest, entrypoint: "https://example.com/run.js"
  }), TypeError);
  assert.throws(() => validateAdapterManifest({
    ...manifest, origins: ["http://remote.example"]
  }), TypeError);
});

test("rejects duplicate or malformed tools", () => {
  assert.throws(() => validateAdapterManifest({
    ...manifest, tools: [manifest.tools[0], manifest.tools[0]]
  }), TypeError);
  assert.throws(() => validateAdapterManifest({
    ...manifest, tools: [{ ...manifest.tools[0], inputSchema: {} }]
  }), TypeError);
});
