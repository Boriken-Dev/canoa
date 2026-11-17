import test from "node:test";
import assert from "node:assert/strict";
import { agentChoices, isPairedPeer } from "./agent-choices.js";

const A = `sha256:${"a".repeat(64)}`;
const B = `sha256:${"b".repeat(64)}`;

test("choices come only from paired agents with a well-formed key", () => {
  const choices = agentChoices([
    { key: "1", label: "Desk", peer: A },
    { key: "2", label: "No key" },
    { key: "3", label: "Bad", peer: "sha256:xyz" },
    { key: "4", label: "Lab‮", peer: B }
  ]);
  assert.deepEqual(choices.map((choice) => choice.value), [A, B]);
  assert.ok(choices[0].label.startsWith("Desk — sha256:aaaa"));
  assert.ok(!/‮/.test(choices[1].label));
  assert.deepEqual(agentChoices(undefined), []);
  assert.deepEqual(agentChoices([]), []);
});

test("a grant may name only an agent the browser holds", () => {
  assert.equal(isPairedPeer([A], A), true);
  assert.equal(isPairedPeer([A], B), false);
  assert.equal(isPairedPeer([], A), false);
  assert.equal(isPairedPeer([A], `sha256:${"a".repeat(63)}`), false);
  assert.equal(isPairedPeer(undefined, A), false);
});
