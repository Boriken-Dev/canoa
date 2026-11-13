import test from "node:test";
import assert from "node:assert/strict";
import { windowUrl } from "./popout-windows.js";

const BASE = "chrome-extension://abc/";
const LINK = "0123456789abcdef0123456789abcdef";
const ok = { sessionId: "s1", link: LINK, title: "Fix the build", look: "canoa", density: "normal" };

test("a window's address is the UI page as a window, with the link that names its channel", () => {
  const url = new URL(windowUrl(BASE, ok));
  assert.equal(`${url.protocol}//${url.host}${url.pathname}`, "chrome-extension://abc/ui/index.html");
  assert.equal(url.searchParams.get("view"), "window");
  assert.equal(url.searchParams.get("session"), "s1");
  assert.equal(url.searchParams.get("popout"), LINK);
  assert.equal(url.searchParams.get("title"), "Fix the build");
});

test("a title is cleaned and cut, and an empty one is named", () => {
  assert.equal(new URL(windowUrl(BASE, { ...ok, title: "a\u202eb\nc" })).searchParams.get("title"), "a b c");
  assert.equal(new URL(windowUrl(BASE, { ...ok, title: "x".repeat(500) })).searchParams.get("title").length, 80);
  assert.equal(new URL(windowUrl(BASE, { ...ok, title: "  " })).searchParams.get("title"), "Canoa");
});

test("a link, a look or a session that is not what the panel makes is refused", () => {
  assert.throws(() => windowUrl(BASE, { ...ok, link: "short" }));
  assert.throws(() => windowUrl(BASE, { ...ok, link: LINK.toUpperCase() }));
  assert.throws(() => windowUrl(BASE, { ...ok, look: "a&view=panel" }));
  assert.throws(() => windowUrl(BASE, { ...ok, density: "" }));
  assert.throws(() => windowUrl(BASE, { ...ok, sessionId: "" }));
  assert.throws(() => windowUrl(BASE, { ...ok, sessionId: "s".repeat(129) }));
});
