import test from "node:test";
import assert from "node:assert/strict";
import { CAPABILITY, DIRECTION, isAllowed, makeGrant, revokeTab, sameGrant } from "./permissions.js";
import {
  addShare, attach, detachSession, detachTab, endSession, fromUiPage, groupTitle, noteActing, sessionsFor, shareGrant, sharedTabs,
  groupOf, leftTheirGroup, tabsToRelease, unshare, clientOf, mayAsk, move, releaseOwner
} from "./tab-sessions.js";

const PEER = `sha256:${"a".repeat(64)}`;
const ORIGIN = "https://example.org";
const share = (sessionId, tabId, origin = ORIGIN) => shareGrant({ sessionId, peerId: PEER, tabId, origin });

test("attaching grants nothing", () => {
  const bindings = attach([], { sessionId: "s1", tabId: 4, origin: `${ORIGIN}/page?q=1` });
  assert.deepEqual(bindings, [{ sessionId: "s1", tabId: 4, origin: ORIGIN, owner: "panel" }]);
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

test("ending a session needs a session: no id ends nothing", () => {
  const ordinary = makeGrant({ direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, tabId: 4, origin: ORIGIN, capability: CAPABILITY.READ, expiresAt: Date.now() + 60_000 });
  for (const bad of [undefined, null, "", 7, {}]) assert.deepEqual(endSession([ordinary], bad), [ordinary]);
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
  const acting = noteActing(noteActing([], { tabId: 4, peer: PEER, label: "Desk" }), { tabId: 5, peer: PEER, label: "Desk" });
  assert.deepEqual(tabsToRelease(acting, [share("s1", 4)]).map((entry) => entry.tabId), [5]);
  assert.deepEqual(tabsToRelease(acting, []).map((entry) => entry.tabId), [4, 5]);
  assert.equal(noteActing(acting, { tabId: 4, peer: PEER, label: "x" }), acting);
});

test("a second tab of the same agent joins the group already made in that window", () => {
  const acting = noteActing([], { tabId: 4, peer: PEER, label: "Desk", groupId: 11, windowId: 1 });
  assert.equal(groupOf(acting, PEER, 1), 11);
  assert.equal(groupOf(acting, PEER, 2), undefined);
  assert.equal(groupOf(acting, `sha256:${"b".repeat(64)}`, 1), undefined);
});

test("groups belong to a peer, not to the name it gave itself: two agents called the same keep two groups", () => {
  const other = `sha256:${"b".repeat(64)}`;
  let acting = noteActing([], { tabId: 4, peer: PEER, label: "Desk", groupId: 11, windowId: 1 });
  acting = noteActing(acting, { tabId: 5, peer: other, label: "Desk", groupId: 12, windowId: 1 });
  assert.equal(groupOf(acting, PEER, 1), 11);
  assert.equal(groupOf(acting, other, 1), 12);
});

test("a tab that left its group, or the browser, is no longer ours to move", () => {
  const acting = noteActing(noteActing(noteActing([], { tabId: 4, peer: PEER, label: "D", groupId: 11, windowId: 1 }), { tabId: 5, peer: PEER, label: "D", groupId: 11, windowId: 1 }), { tabId: 6, peer: PEER, label: "D", groupId: 11, windowId: 1 });
  const nowIn = new Map([[4, 11], [5, 99]]);
  assert.deepEqual(leftTheirGroup(acting, (tabId) => nowIn.get(tabId)).map((entry) => entry.tabId), [5, 6]);
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

const runtime = { id: "abc", getURL: (path) => `chrome-extension://abc/${path}` };
const LINK = "0123456789abcdef0123456789abcdef";
const fromPanel = { id: "abc", url: "chrome-extension://abc/ui/index.html?view=panel" };
const fromWindow = { id: "abc", url: `chrome-extension://abc/ui/index.html?view=window&session=s1&popout=${LINK}` };

test("a binding says which client holds it, the panel unless another is named", () => {
  assert.equal(attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN })[0].owner, "panel");
  assert.equal(attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN, owner: `window:${LINK}` })[0].owner, `window:${LINK}`);
  assert.throws(() => attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN, owner: "someone" }));
  assert.throws(() => attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN, owner: "window:../x" }));
});

test("move hands the binding over without a resume, and says whether the tab changed", () => {
  const held = attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN });
  const same = move(held, { sessionId: "s1", tabId: 4, origin: ORIGIN, owner: `window:${LINK}` });
  assert.deepEqual(same.bindings, [{ sessionId: "s1", tabId: 4, origin: ORIGIN, owner: `window:${LINK}` }]);
  assert.equal(same.moved, false);
  const other = move(held, { sessionId: "s1", tabId: 9, origin: "https://other.example", owner: "panel" });
  assert.deepEqual(other.bindings.map((binding) => [binding.tabId, binding.origin]), [[9, "https://other.example"]]);
  assert.equal(other.moved, true);
  assert.equal(move([], { sessionId: "s2", tabId: 1, origin: ORIGIN, owner: "panel" }).bindings.length, 1);
});

test("moving grants nothing: a share is still only the person's click", () => {
  const held = attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN });
  const { bindings } = move(held, { sessionId: "s1", tabId: 9, origin: ORIGIN, owner: `window:${LINK}` });
  const request = makeGrant({ direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, tabId: 9, origin: ORIGIN, capability: CAPABILITY.READ, expiresAt: 1 });
  assert.equal(isAllowed({ grants: [], request, trustedPeers: new Set([PEER]) }), false);
  assert.equal(bindings.length, 1);
});

test("a window that goes gives back only the bindings it held", () => {
  let bindings = attach([], { sessionId: "s1", tabId: 4, origin: ORIGIN, owner: `window:${LINK}` });
  bindings = attach(bindings, { sessionId: "s2", tabId: 5, origin: ORIGIN, owner: `window:${"f".repeat(32)}` });
  bindings = attach(bindings, { sessionId: "s3", tabId: 6, origin: ORIGIN });
  assert.deepEqual(releaseOwner(bindings, `window:${LINK}`).map((binding) => binding.owner), ["panel", `window:${"f".repeat(32)}`, "panel"]);
});

test("a client is the panel, or a window named by its link; anything else is no client", () => {
  assert.equal(clientOf(fromPanel, runtime), "panel");
  assert.equal(clientOf({ id: "abc", url: "chrome-extension://abc/ui/index.html?view=full" }, runtime), "panel");
  assert.equal(clientOf(fromWindow, runtime), `window:${LINK}`);
  assert.equal(clientOf({ id: "abc", url: "chrome-extension://abc/ui/index.html?view=window" }, runtime), null);
  assert.equal(clientOf({ id: "abc", url: "chrome-extension://abc/ui/index.html?view=window&popout=../../x" }, runtime), null);
  assert.equal(clientOf({ id: "abc", url: "chrome-extension://abc/permissions.html" }, runtime), null);
  assert.equal(clientOf({ id: "other", url: `chrome-extension://abc/ui/index.html?view=window&popout=${LINK}` }, runtime), null);
  assert.equal(clientOf({ id: "abc", url: `https://evil.example/ui/index.html?view=window&popout=${LINK}` }, runtime), null);
});

test("a window reads and takes the tab binding and asks for nothing that grants", () => {
  const window = clientOf(fromWindow, runtime);
  for (const type of ["canoa.tabs.list", "canoa.tabs.move"]) assert.equal(mayAsk(window, type), true, type);
  for (const type of ["canoa.tabs.share", "canoa.tabs.unshare", "canoa.tabs.attach", "canoa.tabs.detach", "canoa.tabs.sessionsFor", "canoa.popout.open", "canoa.popout.close"]) {
    assert.equal(mayAsk(window, type), false, type);
  }
  assert.equal(mayAsk("panel", "canoa.tabs.share"), true);
  assert.equal(mayAsk(null, "canoa.tabs.list"), false);
});
