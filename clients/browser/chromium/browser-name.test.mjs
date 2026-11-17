import test from "node:test";
import assert from "node:assert/strict";
import { cleanName, defaultBrowserName, readBrowserName, writeBrowserName } from "./browser-name.js";

function memory() {
  const held = {};
  return { get: async (key) => ({ [key]: held[key] }), set: async (items) => Object.assign(held, items) };
}

test("the default names the browser and its system, never the engine's placeholder brand", () => {
  assert.equal(defaultBrowserName({ brands: [{ brand: "Not A;Brand" }, { brand: "Chromium" }, { brand: "Microsoft Edge" }], platform: "Windows" }), "Edge on Windows");
  assert.equal(defaultBrowserName({ brands: [{ brand: "Chromium" }], platform: "Linux" }), "Chromium on Linux");
  assert.equal(defaultBrowserName({ brands: [{ brand: "Google Chrome" }] }), "Chrome");
  assert.equal(defaultBrowserName(undefined), "This browser");
});

test("a name is trimmed, one line, bounded and never empty", () => {
  assert.equal(cleanName("  Work‮   laptop\n"), "Work laptop");
  assert.equal(cleanName("x".repeat(200)).length, 60);
  assert.equal(cleanName("   "), null);
  assert.equal(cleanName(undefined), null);
});

test("a stored name reads back, and an empty answer stores nothing", async () => {
  const storage = memory();
  assert.equal(await readBrowserName(storage), null);
  assert.equal(await writeBrowserName(storage, " Personal Edge "), "Personal Edge");
  assert.equal(await readBrowserName(storage), "Personal Edge");
  assert.equal(await writeBrowserName(storage, "  "), null);
  assert.equal(await readBrowserName(storage), "Personal Edge");
});
