import test from "node:test";
import assert from "node:assert/strict";
import { CAPABILITY, DIRECTION, isAllowed, makeGrant, revokeTab, sameGrant } from "./permissions.js";
import {
  addShare, attach, detachSession, detachTab, endSession, fromUiPage, groupTitle, noteActing, sessionsFor, shareGrant, sharedTabs,
  groupOf, tabsToRelease, unshare
} from "./tab-sessions.js";

const PEER = `sha256:${"a".repeat(64)}`;
const ORIGIN = "https://example.org";
const share = (sessionId, tabId, origin = ORIGIN) => shareGrant({ sessionId, peerId: PEER, tabId, origin });

test("attaching grants nothing", () => {
  const bindings = attach([], { sessionId: "s1", tabId: 4, origin: `${ORIGIN}/page?q=1` });
  assert.deepEqual(bindings, [{ sessionId: "s1", tabId: 4, origin: ORIGIN }]);
  const request = makeGrant({ direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, tabId: 4, origin: ORIGIN, capability: CAPABILITY.READ, expiresAt: 1 });
  assert.equal(isAllowed({ grants: [], request, trustedPeers: new Set([PEER]) }), false);
});

test("a session is in one place: attaching again moves it", () => {
  let bindings = attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN });
  bindings = attach(bindings, { sessionId: "s1", tabId: 9, origin: "https://other.example" });
  assert.deepEqual(bindings.map((binding) => binding.tabId), [9]);
  assert.throws(() => attach([], { sessionId: "", tabId: 1, origin: ORIGIN }));
  assert.throws(() => attach([], { sessionId: "s", tabId: -1, origin: ORIGIN }));
  assert.throws(() => attach([], { sessionId: "s", tabId: 1, origin: "file:///x" }));
});

test("a closed tab or a detached session leaves no binding", () => {
  let bindings = attach(attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN }), { sessionId: "s2", tabId: 5, origin: ORIGIN });
  assert.deepEqual(detachTab(bindings, 4).map((binding) => binding.sessionId), ["s2"]);
  assert.deepEqual(detachSession(bindings, "s2").map((binding) => binding.sessionId), ["s1"]);
});

test("sessionsFor names the sessions of one origin and nothing else", () => {
  const bindings = attach(attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN }), { sessionId: "s2", tabId: 5, origin: "https://other.example" });
  assert.deepEqual(sessionsFor(bindings, `${ORIGIN}/a/b`), ["s1"]);
  assert.deepEqual(sessionsFor(bindings, "https://nobody.example"), []);
  assert.deepEqual(sessionsFor(bindings, "not a url"), []);
});

test("a share is read on that tab, has no expiry and no cap, and only reads", () => {
  let grants = [];
  for (let tab = 1; tab <= 40; tab += 1) grants = addShare(grants, share("s1", tab));
  assert.equal(sharedTabs(grants, "s1").length, 40);
  const read = { ...grants[0] };
  const far = Date.now() + 10 * 365 * 24 * 3600 * 1000;
  assert.equal(isAllowed({ grants, request: read, trustedPeers: new Set([PEER]), now: far }), true);
  assert.equal(isAllowed({ grants, request: { ...read, capability: CAPABILITY.INTERACT }, trustedPeers: new Set([PEER]) }), false);
  assert.equal(isAllowed({ grants, request: { ...read, tabId: 999 }, trustedPeers: new Set([PEER]) }), false);
});

test("sharing a tab twice is one share", () => {
  const grants = addShare(addShare([], share("s1", 4)), share("s1", 4));
  assert.equal(grants.length, 1);
  assert.equal(sameGrant(grants[0], share("s1", 4)), true);
  assert.equal(sameGrant(share("s1", 4), share("s2", 4)), false);
});

test("unshare, a closed tab, a navigation and the end of a session each take the share away", () => {
  const base = addShare(addShare(addShare([], share("s1", 4)), share("s1", 5)), share("s2", 4));
  assert.deepEqual(sharedTabs(unshare(base, { sessionId: "s1", tabId: 4 }), "s1"), [5]);
  assert.deepEqual(sharedTabs(unshare(base, { sessionId: "s1", tabId: 4 }), "s2"), [4]);
  assert.deepEqual(revokeTab(base, 4).map((grant) => grant.session), ["s1"]);
  assert.deepEqual(sharedTabs(endSession(base, "s1"), "s1"), []);
  assert.deepEqual(sharedTabs(endSession(base, "s1"), "s2"), [4]);
});

test("unshare leaves a grant the person made another way", () => {
  const other = makeGrant({ direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, tabId: 4, origin: ORIGIN, capability: CAPABILITY.READ, expiresAt: Date.now() + 60_000 });
  const grants = [other, share("s1", 4)];
  assert.deepEqual(unshare(grants, { sessionId: "s1", tabId: 4 }), [other]);
});

test("a tab leaves its group when no grant names it any more, and not before", () => {
  const acting = noteActing(noteActing([], { tabId: 4, label: "Desk" }), { tabId: 5, label: "Desk" });
  assert.deepEqual(tabsToRelease(acting, [share("s1", 4)]).map((entry) => entry.tabId), [5]);
  assert.deepEqual(tabsToRelease(acting, []).map((entry) => entry.tabId), [4, 5]);
  assert.equal(noteActing(acting, { tabId: 4, label: "x" }), acting);
});

test("a second tab of the same agent joins the group already made in that window", () => {
  const acting = noteActing([], { tabId: 4, label: "Desk", groupId: 11, windowId: 1 });
  assert.equal(groupOf(acting, "Desk", 1), 11);
  assert.equal(groupOf(acting, "Desk", 2), undefined);
  assert.equal(groupOf(acting, "Other", 1), undefined);
});

test("the group is titled with the agent's name, cleaned and bounded", () => {
  assert.equal(groupTitle("  Desk‮   agent "), "Desk agent");
  assert.equal(groupTitle(""), "Canoa");
  assert.equal(groupTitle("x".repeat(100)).length, 40);
});

test("only the extension's own UI pages may bind or share", () => {
  const runtime = { id: "abc", getURL: (path) => `chrome-extension://abc/${path}` };
  assert.equal(fromUiPage({ id: "abc", url: "chrome-extension://abc/ui/index.html?view=panel" }, runtime), true);
  assert.equal(fromUiPage({ id: "abc", url: "chrome-extension://abc/permissions.html" }, runtime), false);
  assert.equal(fromUiPage({ id: "abc", url: "https://evil.example/ui/" }, runtime), false);
  assert.equal(fromUiPage({ id: "other", url: "chrome-extension://abc/ui/x" }, runtime), false);
  assert.equal(fromUiPage(undefined, runtime), false);
});
