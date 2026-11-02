import assert from "node:assert/strict";
import { test } from "node:test";

import { readCandidates } from "./acp-discovery.js";

const KEY = `sha256:${"a".repeat(64)}`;
const OTHER = `sha256:${"b".repeat(64)}`;
const good = { label: "Mine", url: "wss://10.0.0.5:8765/acp", peer: KEY, source: "mdns" };
const withTransport = { ...good, webtransport: 8766, webtransportHash: OTHER };

test("a good candidate comes through with what it claims", () => {
  assert.deepEqual(readCandidates({ candidates: [good] }), [{ ...good, webtransport: undefined, sameKeyAs: undefined }]);
});

test("nothing usable in a bad reply", () => {
  for (const reply of [undefined, null, 7, {}, { candidates: "x" }, { error: "unknown op" }]) {
    assert.deepEqual(readCandidates(reply), []);
  }
});

test("only a secure address, or plain on this machine", () => {
  const url = (text) => readCandidates({ candidates: [{ ...good, url: text }] }).length;
  assert.equal(url("ws://127.0.0.1:1/acp"), 1);
  assert.equal(url("ws://[::1]:1/acp"), 1);
  assert.equal(url("ws://10.0.0.5:1/acp"), 0);
  assert.equal(url("http://10.0.0.5:1/acp"), 0);
  assert.equal(url("javascript:alert(1)"), 0);
  assert.equal(url("not a url"), 0);
});

test("a malformed key is no key, and a blank label is no candidate", () => {
  assert.equal(readCandidates({ candidates: [{ ...good, peer: "sha256:zz" }] })[0].peer, undefined);
  assert.deepEqual(readCandidates({ candidates: [{ ...good, label: "   " }] }), []);
  assert.deepEqual(readCandidates({ candidates: [{ ...good, label: 7 }] }), []);
});

test("an unknown source cannot claim to be this machine", () => {
  assert.equal(readCandidates({ candidates: [{ ...good, source: "trusted" }] })[0].source, "mdns");
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
