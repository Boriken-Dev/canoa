// coqui-acp's conformance checks against this extension's own code: the real page script and content script, in a fake page,
// joined to the real gateway. A browser run is a separate test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { test } from "node:test";

import { registerProvider } from "./vendor/coqui-acp/registry.mjs";
import { CHECKS, runConformance } from "./vendor/coqui-acp/conformance.mjs";
import { makeDriver } from "./acp-driver.mjs";

test("this extension passes every conformance check", async () => {
  const results = await runConformance(makeDriver);
  assert.equal(results.length, Object.keys(CHECKS).length);
  assert.deepEqual(results.filter((result) => !result.passed), []);
});

test("another provider shares the registry the page script defined, and each agent routes to its own", async () => {
  const driver = makeDriver();
  const page = driver.page("https://a.example");
  const other = {
    info: { uuid: "other", name: "Other", rdns: "dev.example.other" },
    agents: async () => [{ id: "theirs", provider: "other" }],
    request: async () => [],
    connect: async (agent) => ({ agent }),
    revoke: async () => {},
  };
  assert.equal(registerProvider(other, page.window), page.acp);
  assert.equal(page.acp.providers.length, 2);
  assert.equal((await page.acp.connect({ id: "x", provider: "other" })).agent.provider, "other");
  const listed = await page.acp.agents();
  assert.deepEqual(Array.from(listed, (agent) => agent.id), ["theirs"]);
});

test("the page cannot claim activation: the content script decides", async () => {
  const driver = makeDriver();
  driver.discover("agent");
  driver.person.answerNext({ allow: "agent", cwd: "/g" });
  const page = driver.page("https://a.example");
  await assert.rejects(page.acp.request({ activated: true }), { name: "NotAllowedError" });
});

test("the vendored modules are the recorded ones", () => {
  const blob = (path) => {
    const data = readFileSync(new URL(path, import.meta.url));
    return createHash("sha1").update(`blob ${data.length}\0`).update(data).digest("hex");
  };
  const meta = JSON.parse(readFileSync(new URL("./vendor/coqui-acp/SOURCE.json", import.meta.url), "utf8"));
  for (const [name, sha] of Object.entries(meta.files)) assert.equal(blob(`./vendor/coqui-acp/${name}`), sha, name);
  const methods = JSON.parse(readFileSync(new URL("./vendor/COQUI-SOURCE.json", import.meta.url), "utf8"));
  for (const [name, source] of Object.entries(methods)) assert.equal(blob(`./vendor/${name}`), source.git_blob_sha1, name);
});
