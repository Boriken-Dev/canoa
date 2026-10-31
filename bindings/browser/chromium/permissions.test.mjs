import test from "node:test";
import assert from "node:assert/strict";
import {
  CAPABILITY, DIRECTION, isAllowed, makeGrant, pruneExpired, revokePeer, revokeTab
} from "./permissions.js";

const NOW = 1_800_000_000_000;
const PEER = "sha256:3f15";
const origin = "https://example.org";
const trustedPeers = new Set([PEER]);

function agentTabGrant(overrides = {}) {
  return makeGrant({
    direction: DIRECTION.AGENT_TO_PAGE, peerId: PEER, tabId: 7,
    origin, capability: CAPABILITY.READ, expiresAt: NOW + 1000, ...overrides
  });
}
function request(grant) {
  return { ...grant };
}

test("deny by default, including unverified peers", () => {
  const grant = agentTabGrant();
  assert.equal(isAllowed({ grants: [], request: grant, trustedPeers, now: NOW }), false);
  assert.equal(isAllowed({ grants: [grant], request: grant, trustedPeers: new Set(), now: NOW }), false);
  assert.equal(isAllowed({ grants: [grant], request: grant, trustedPeers, now: NOW }), true);
});

test("identity, origin, tab and capability must all match", () => {
  const grant = agentTabGrant();
  const query = (overrides) => isAllowed({
    grants: [grant], trustedPeers: new Set([PEER, "other"]), now: NOW,
    request: request({ ...grant, ...overrides })
  });
  assert.equal(query({ peerId: "other" }), false);
  assert.equal(query({ origin: "https://other.org" }), false);
  assert.equal(query({ tabId: 8 }), false);
  assert.equal(query({ capability: CAPABILITY.INTERACT }), false);
  assert.equal(query({ capability: CAPABILITY.WEBMCP }), false);
  assert.equal(query({ capability: CAPABILITY.DISCOVER }), false);
});

test("read access never permits writes or page to agent connections", () => {
  const grant = agentTabGrant();
  const anotherDirection = {
    direction: DIRECTION.PAGE_TO_AGENT, peerId: PEER, origin,
    capability: CAPABILITY.CONNECT, expiresAt: NOW + 1000
  };
  assert.equal(isAllowed({ grants: [grant], request: anotherDirection, trustedPeers, now: NOW }), false);
});

test("expiry, navigation revocation and peer revocation", () => {
  const grant = agentTabGrant();
  assert.equal(isAllowed({ grants: [grant], request: grant, trustedPeers, now: NOW + 1000 }), false);
  assert.deepEqual(revokeTab([grant], 7), []);
  assert.deepEqual(revokePeer([grant], PEER), []);
  assert.deepEqual(pruneExpired([grant], NOW + 1000), []);
});

test("page to agent grants do not grant browser access", () => {
  const grant = makeGrant({
    direction: DIRECTION.PAGE_TO_AGENT, peerId: PEER, origin,
    capability: CAPABILITY.PROMPT, expiresAt: NOW + 1000
  });
  assert.equal(isAllowed({ grants: [grant], request: grant, trustedPeers, now: NOW }), true);
  assert.equal(isAllowed({
    grants: [grant],
    request: agentTabGrant(),
    trustedPeers, now: NOW
  }), false);
});

test("reject invalid origins and incompatible scopes", () => {
  assert.throws(() => agentTabGrant({ origin: "https://example.org/path" }), TypeError);
  assert.throws(() => agentTabGrant({ origin: "javascript:alert(1)" }), TypeError);
  assert.throws(() => agentTabGrant({ tabId: -1 }), TypeError);
  assert.throws(() => agentTabGrant({ capability: CAPABILITY.CONNECT }), TypeError);
  assert.throws(() => makeGrant({
    direction: DIRECTION.PAGE_TO_AGENT, peerId: PEER, origin,
    tabId: 7, capability: CAPABILITY.CONNECT, expiresAt: NOW + 1000
  }), TypeError);
});
