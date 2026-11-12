import test from "node:test";
import assert from "node:assert/strict";
import { keyAnswer } from "./answer-keys.js";

const press = (key, target = "BODY", extra = {}) => keyAnswer({ key, target: { tagName: target }, ...extra });

test("Escape declines and Enter accepts", () => {
  assert.equal(press("Escape"), "decline");
  assert.equal(press("Enter"), "accept");
});

test("Enter in a field or on a button is that control's own", () => {
  for (const tag of ["INPUT", "SELECT", "TEXTAREA", "BUTTON", "A"]) assert.equal(press("Enter", tag), null);
  assert.equal(press("Escape", "INPUT"), "decline");
});

test("a modified key, a composing one and a handled one answer nothing", () => {
  assert.equal(press("Enter", "BODY", { ctrlKey: true }), null);
  assert.equal(press("Enter", "BODY", { isComposing: true }), null);
  assert.equal(press("Escape", "BODY", { defaultPrevented: true }), null);
  assert.equal(press("a"), null);
});
