import test from "node:test";
import assert from "node:assert/strict";
import {
  offeredOptions, selectPermissionOption, denyPermission, cancelPermission
} from "./acp-permissions.js";

const params = {
  sessionId: "s1",
  title: "Execute command?",
  options: [
    { optionId: "yes-1", name: "Allow once", kind: "allow_once" },
    { optionId: "no-1", name: "Reject once", kind: "reject_once" }
  ]
};
const peerId = "sha256:confirmed";
const trustedPeers = new Set([peerId]);

test("preserve the exact optionId offered by the agent", () => {
  assert.deepEqual(selectPermissionOption({ params, optionId: "yes-1", peerId, trustedPeers }),
    { outcome: { outcome: "selected", optionId: "yes-1" } });
  assert.throws(() => selectPermissionOption({ params, optionId: "allow_always", peerId, trustedPeers }));
});

test("unverified agent cannot obtain an approval", () => {
  assert.throws(() => selectPermissionOption({ params, optionId: "yes-1", peerId, trustedPeers: new Set() }));
});

test("policy deny selects a real reject option or cancels", () => {
  assert.deepEqual(denyPermission(params), { outcome: { outcome: "selected", optionId: "no-1" } });
  assert.deepEqual(denyPermission({ ...params, options: [params.options[0]] }), cancelPermission());
});

test("reject duplicate or malformed option IDs", () => {
  assert.throws(() => offeredOptions({ ...params, options: [params.options[0], params.options[0]] }), TypeError);
  assert.throws(() => offeredOptions({ ...params, options: [] }), TypeError);
});
