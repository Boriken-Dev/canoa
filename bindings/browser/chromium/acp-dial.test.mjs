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

test("an empty message or one past the limit is refused before it is held", () => {
  assert.throws(() => createDeframer()(new Uint8Array(4)));
  assert.throws(() => createDeframer(4)(frame("hello")));
  assert.throws(() => createDeframer()(new Uint8Array([255, 255, 255, 255])));
});
