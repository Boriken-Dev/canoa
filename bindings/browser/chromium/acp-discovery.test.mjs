import assert from "node:assert/strict";
import { test } from "node:test";

import { lookNearby, readCandidates, searchNotes } from "./acp-discovery.js";

const KEY = `sha256:${"a".repeat(64)}`;
const OTHER = `sha256:${"b".repeat(64)}`;
const good = { label: "Mine", url: "wss://10.0.0.5:8765/acp", peer: KEY, source: "dns-sd" };
const withTransport = { ...good, webtransport: 8766, webtransportHash: OTHER };

test("a good candidate comes through with what it claims", () => {
  assert.deepEqual(readCandidates({ candidates: [good] }), [{ ...good, method: "webcrypto", webtransport: undefined, offers: [], sameKeyAs: undefined, sameNameAs: undefined }]);
});

test("nothing usable in a bad reply", () => {
  for (const reply of [undefined, null, 7, {}, { candidates: "x" }, { error: "unknown op" }]) {
    assert.deepEqual(readCandidates(reply), []);
  }
});

test("only a secure address, or plain on this machine", () => {
  const url = (text) => readCandidates({ candidates: [{ ...good, source: "local", url: text }] }).length;
  assert.equal(url("ws://127.0.0.1:1/acp"), 1);
  assert.equal(url("ws://[::1]:1/acp"), 1);
  assert.equal(url("ws://10.0.0.5:1/acp"), 0);
  assert.equal(url("wss://10.0.0.5:1/acp"), 0, "a local descriptor names this machine");
  assert.equal(url("ws://agent.example.com:443@127.0.0.1:1/acp"), 0, "a user name that reads like a host");
  assert.equal(url(`ws://127.0.0.1:1/${"x".repeat(600)}`), 0);
  assert.equal(url("http://10.0.0.5:1/acp"), 0);
  assert.equal(url("javascript:alert(1)"), 0);
  assert.equal(url("not a url"), 0);
});

test("a record cannot point a person at their own machine, however it is spelled", () => {
  const offered = (url) => readCandidates({ candidates: [{ ...good, url }] }).length;
  assert.equal(offered("wss://10.0.0.5:1/acp"), 1);
  assert.equal(offered("wss://agent.example.com/acp"), 1);
  for (const url of ["wss://localhost:1/acp", "wss://foo.localhost:1/acp", "wss://127.0.0.1:1/acp", "wss://127.1:1/acp", "wss://2130706433:1/acp",
    "wss://0x7f.0.0.1:1/acp", "wss://0.0.0.0:1/acp", "wss://169.254.169.254:1/acp", "wss://[::1]:1/acp", "wss://[::ffff:127.0.0.1]:1/acp",
    "wss://[::127.0.0.1]:1/acp", "wss://[64:ff9b::7f00:1]:1/acp", "wss://[fe80::1]:1/acp", "wss://intranet:1/acp"]) {
    assert.equal(offered(url), 0, url);
  }
});

test("on the network a record has to name a key, and on this machine it need not", () => {
  assert.deepEqual(readCandidates({ candidates: [{ ...good, peer: undefined }] }), []);
  assert.deepEqual(readCandidates({ candidates: [{ ...good, peer: "sha256:zz" }] }), []);
  const [local] = readCandidates({ candidates: [{ label: "Here", url: "ws://127.0.0.1:1/acp", source: "local" }] });
  assert.equal(local.peer, undefined);
});

test("a name cannot hide part of itself or reorder what is shown around it", () => {
  const [found] = readCandidates({ candidates: [{ ...good, label: "a\u202Eb\u200Bc\u00ADd\u0007e" }] });
  assert.equal(found.label, "abcde");
  assert.deepEqual(readCandidates({ candidates: [{ ...good, label: "\u202E\u200B" }] }), []);
});

test("another agent with the same name and a different key says so", () => {
  const [found] = readCandidates({ candidates: [good] }, [{ label: "Mine", url: "wss://10.0.0.6:1/acp", peer: OTHER }]);
  assert.equal(found.sameNameAs, "Mine");
  assert.equal(found.sameKeyAs, undefined);
});

test("a malformed key is no key, and a blank label is no candidate", () => {
  assert.deepEqual(readCandidates({ candidates: [{ ...good, label: "   " }] }), []);
  assert.deepEqual(readCandidates({ candidates: [{ ...good, label: 7 }] }), []);
});

test("an unknown source cannot claim to be this machine", () => {
  assert.equal(readCandidates({ candidates: [{ ...good, source: "trusted" }] })[0].source, "dns-sd");
});

test("length and count are bounded", () => {
  assert.equal(readCandidates({ candidates: [{ ...good, label: "x".repeat(500) }] })[0].label.length, 80);
  assert.equal(readCandidates({ candidates: Array.from({ length: 500 }, () => good) }).length, 64);
});

test("an agent the person has at that address with that key is not offered again", () => {
  assert.deepEqual(readCandidates({ candidates: [good] }, [{ label: "Had", url: good.url, peer: KEY }]), []);
});

test("WebTransport needs a port and a hash, and uses the address and path it came with", () => {
  assert.deepEqual(readCandidates({ candidates: [withTransport] })[0].webtransport, { url: "https://10.0.0.5:8766/acp", hash: OTHER });
  for (const bad of [{ webtransport: 0 }, { webtransport: 70000 }, { webtransport: "8766" }, { webtransportHash: "sha256:zz" }, { webtransportHash: null }]) {
    assert.equal(readCandidates({ candidates: [{ ...withTransport, ...bad }] })[0].webtransport, undefined, JSON.stringify(bad));
  }
});

test("a new certificate at the same address is offered again, so the agent can move to it", () => {
  const had = { label: "Had", url: good.url, peer: KEY, webtransport: { url: "https://10.0.0.5:8766/acp", hash: KEY } };
  const [found] = readCandidates({ candidates: [withTransport] }, [had]);
  assert.equal(found.sameKeyAs, "Had");
  assert.deepEqual(readCandidates({ candidates: [{ ...withTransport, webtransportHash: KEY }] }, [had]), []);
});

test("another address with a key the person knows says so, and stays a new candidate", () => {
  const [found] = readCandidates({ candidates: [good] }, [{ label: "Had", url: "wss://10.0.0.6:1/acp", peer: KEY }]);
  assert.equal(found.sameKeyAs, "Had");
  assert.equal(found.url, good.url);
  const [fresh] = readCandidates({ candidates: [{ ...good, peer: OTHER }] }, [{ label: "Had", url: good.url, peer: KEY }]);
  assert.equal(fresh.sameKeyAs, undefined);
  assert.equal(fresh.peer, OTHER);
});

test("what a candidate says it serves is read as a stranger wrote it", () => {
  const offers = (value) => readCandidates({ candidates: [{ ...good, offers: value }] })[0].offers;
  assert.deepEqual(offers(["acp/1", "mcp"]), ["acp/1", "mcp"]);
  assert.deepEqual(offers("acp/1"), []);
  assert.deepEqual(offers(["a b", "a,b", "a=b", "", 7, null, "\u202eacp", "x".repeat(65), "mcp"]), ["mcp"]);
  assert.equal(offers(Array.from({ length: 100 }, (_, index) => `p${index}`)).length, 16);
});

const found = (state = "ok", extra = {}) => ({ state, reply: { candidates: [good], ...extra } });

test("a host that refuses the key is the answer: the native host is not asked in its place", async () => {
  let relayAsked = false;
  const viaRelay = async () => {
    relayAsked = true;
    return found();
  };
  const refused = await lookNearby({ viaHost: async () => { throw Object.assign(new Error("this key has no discover grant"), { code: -32001 }); }, viaRelay, known: [] });
  assert.deepEqual(refused, { candidates: [], refused: true });
  assert.equal(relayAsked, false);
});

test("a host that cannot be reached, or fails otherwise, or no pairing, leaves the native host to look", async () => {
  const viaRelay = async () => found();
  for (const viaHost of [async () => { throw new Error("the link to Canoa ended"); }, async () => { throw Object.assign(new Error("busy"), { code: -32002 }); }, undefined]) {
    const said = await lookNearby({ viaHost, viaRelay, known: [] });
    assert.equal(said.candidates.length, 1);
    assert.equal(said.refused, undefined);
  }
  const none = await lookNearby({ viaHost: undefined, viaRelay: async () => ({ state: "not-installed" }), known: [] });
  assert.deepEqual(none, { candidates: [], needsCanoa: true, why: undefined });
});

test("what a search says about its folder and its network is given to the person, cut down", () => {
  assert.deepEqual(searchNotes({ candidates: [] }), []);
  assert.equal(searchNotes({ directory: "shared" }).length, 1);
  assert.match(searchNotes({ directory: "unchecked" })[0], /could not be checked/);
  const [net] = searchNotes({ networkError: `no‮ multicast ${"x".repeat(500)}` });
  assert.ok(net.startsWith("The network could not be searched: no multicast") && net.length < 260, net);
  assert.deepEqual(searchNotes({ networkError: "  " }), []);
});
