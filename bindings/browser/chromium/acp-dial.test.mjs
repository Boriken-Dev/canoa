import assert from "node:assert/strict";
import { test } from "node:test";

import { createDeframer, frame } from "./acp-dial.js";

const text = (buffer) => new TextDecoder().decode(buffer);

test("a frame is a little-endian length and its body, as coqui-web's vectors say", () => {
  assert.deepEqual([...frame("hi")], [2, 0, 0, 0, 0x68, 0x69]);
});

test("messages arrive whole however the stream cuts them", () => {
  const bytes = new Uint8Array([...frame("hello"), ...frame("x"), ...frame("world")]);
  for (let size = 1; size <= bytes.length; size += 1) {
    const deframe = createDeframer();
    const seen = [];
    for (let at = 0; at < bytes.length; at += size) seen.push(...deframe(bytes.subarray(at, at + size)));
    assert.deepEqual(seen.map(text), ["hello", "x", "world"], `in pieces of ${size}`);
  }
});

test("a limit can be raised once a peer has proved itself, and a large message in small chunks is not quadratic", () => {
  const deframe = createDeframer(8);
  assert.throws(() => createDeframer(8)(frame("x".repeat(9))));
  deframe.raiseLimit(8 * 1024 * 1024);
  const whole = frame("y".repeat(8 * 1024 * 1024));
  const begun = Date.now();
  let seen = [];
  for (let at = 0; at < whole.length; at += 4096) seen = seen.concat(deframe(whole.subarray(at, at + 4096)));
  assert.equal(seen.length, 1);
  assert.equal(seen[0].byteLength, 8 * 1024 * 1024);
  assert.ok(Date.now() - begun < 2000, `took ${Date.now() - begun} ms`);
});

test("messages survive the buffer moving under them", () => {
  const deframe = createDeframer();
  const seen = [];
  for (let round = 0; round < 50; round += 1) {
    const body = String(round).repeat(1000);
    const framed = frame(body);
    seen.push(...deframe(framed.subarray(0, 2000)), ...deframe(framed.subarray(2000)));
  }
  assert.equal(seen.length, 50);
  assert.equal(text(seen[49]), "49".repeat(1000));
});

test("an empty message or one past the limit is refused before it is held", () => {
  assert.throws(() => createDeframer()(new Uint8Array(4)));
  assert.throws(() => createDeframer(4)(frame("hello")));
  assert.throws(() => createDeframer()(new Uint8Array([255, 255, 255, 255])));
});
