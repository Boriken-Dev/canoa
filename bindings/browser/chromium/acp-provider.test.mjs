// coqui-acp's conformance checks against this extension's own code: the real page script and content script, in a fake page,
// joined to the real gateway. A browser run is a separate test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { test } from "node:test";
import vm from "node:vm";

import { createGateway } from "./acp-gateway.js";
import { CHECKS, runConformance } from "./vendor/coqui-acp/conformance.mjs";
import { registerProvider } from "./vendor/coqui-acp/registry.mjs";

const PAGE = readFileSync(new URL("./acp-page.js", import.meta.url), "utf8");
const CONTENT = readFileSync(new URL("./acp-content.js", import.meta.url), "utf8");

class FakeWindow extends EventTarget {
  document = {};
  postMessage(data, _origin, transfer = []) {
    const event = new Event("message");
    Object.assign(event, { data, source: this, ports: transfer });
    setTimeout(() => this.dispatchEvent(event), 0);
  }
}

class Agent {
  received_ = [];
  next = 0;
  listeners = [];
  received() {
    return this.received_;
  }
  send(message) {
    for (const listen of this.listeners) listen(JSON.stringify(message));
  }
  deliver(text) {
    const message = JSON.parse(text);
    this.received_.push(message);
    if (message.id === undefined || message.method === undefined) return;
    const result = message.method === "session/new" ? { sessionId: `s${(this.next += 1)}` } : {};
    this.send({ jsonrpc: "2.0", id: message.id, result });
  }
}

function memoryStore() {
  const agents = [];
  const grants = [];
  const same = (a, b) => a.origin === b.origin && a.agent === b.agent;
  return {
    agents: async () => agents,
    async addAgent(agent) {
      const made = { key: agent.label, source: "manual", ...agent };
      agents.push(made);
      return made;
    },
    async markProven() {},
    grants: async (origin) => grants.filter((grant) => grant.origin === origin),
    async saveGrant(grant) {
      const at = grants.findIndex((held) => same(held, grant));
      if (at >= 0) grants[at] = grant;
      else grants.push(grant);
    },
    async deleteGrant(grant) {
      const at = grants.findIndex((held) => same(held, grant));
      if (at >= 0) grants.splice(at, 1);
    },
    secret: async () => "test-secret",
    known: agents,
  };
}

function makeDriver() {
  const remote = new Map();
  const store = memoryStore();
  const asked = [];
  let answer;
  const gateway = createGateway({
    store,
    dial: async (agent) => {
      const peer = remote.get(agent.key);
      return {
        proven: agent.proven !== false,
        send: async (text) => peer.deliver(text),
        onMessage: (handler) => peer.listeners.splice(0, peer.listeners.length, handler),
        onClose: () => {},
        close: () => {},
      };
    },
    askPerson: async () => {
      const decision = answer;
      answer = undefined;
      if (decision === undefined || decision.refuse) return null;
      for (const label of decision.add ?? []) remote.set(label, new Agent());
      return {
        allow: [decision.allow].flat().filter((label) => !(decision.add ?? []).includes(label)),
        add: (decision.add ?? []).map((label) => ({ label, peer: `sha256:${label}` })),
        cwd: decision.cwd,
        permissions: decision.permissions,
        mcpOrigins: decision.mcpOrigins,
        lifetime: "always",
      };
    },
    askPermission: async (details) => {
      asked.push(details.request);
      return null;
    },
  });

  const page = (origin) => {
    const window = new FakeWindow();
    const location = { origin };
    let activated = false;
    let worker;
    const chrome = {
      runtime: {
        connect() {
          const listeners = { message: [], disconnect: [] };
          const toGateway = { message: [], disconnect: [] };
          const inner = {
            postMessage: (message) => listeners.message.forEach((listen) => listen(message)),
            onMessage: { addListener: (listen) => toGateway.message.push(listen) },
            onDisconnect: { addListener: (listen) => toGateway.disconnect.push(listen) },
          };
          gateway.attach(inner, { origin, tabId: 1 });
          worker = {
            postMessage: (message) => toGateway.message.forEach((listen) => listen(structuredClone(message))),
            onMessage: { addListener: (listen) => listeners.message.push(listen) },
            onDisconnect: { addListener: (listen) => listeners.disconnect.push(listen) },
            disconnect: () => toGateway.disconnect.forEach((listen) => listen()),
          };
          return worker;
        },
      },
    };
    const shared = {
      window, document: window.document, location, crypto, Event, CustomEvent, EventTarget, DOMException, MessageChannel,
      ReadableStream, WritableStream, setInterval, clearInterval, console,
    };
    vm.runInNewContext(PAGE, shared);
    vm.runInNewContext(CONTENT, { ...shared, chrome, navigator: { userActivation: { get isActive() { return activated; } } } });
    return {
      get acp() {
        return window.document.acp;
      },
      window,
      activate() {
        activated = true;
      },
      navigate() {
        window.dispatchEvent(new Event("pagehide"));
      },
      settle: () => new Promise((resolve) => setTimeout(resolve, 5)),
    };
  };

  return {
    person: {
      answerNext(next) {
        answer = next;
      },
      permissionsAsked: () => asked,
    },
    page,
    discover(label, { proven = true } = {}) {
      if (!remote.has(label)) {
        remote.set(label, new Agent());
        store.known.push({ key: label, label, source: "dns-sd", peer: `sha256:${label}`, proven });
      }
    },
    agent: (label) => remote.get(label),
  };
}

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

test("the vendored modules are the recorded ones, and match the app's", () => {
  const blob = (path) => {
    const data = readFileSync(new URL(path, import.meta.url));
    return createHash("sha1").update(`blob ${data.length}\0`).update(data).digest("hex");
  };
  const meta = JSON.parse(readFileSync(new URL("./vendor/coqui-acp/SOURCE.json", import.meta.url), "utf8"));
  for (const [name, sha] of Object.entries(meta.files)) assert.equal(blob(`./vendor/coqui-acp/${name}`), sha, name);
  const methods = JSON.parse(readFileSync(new URL("../../../app/web/vendor/COQUI-SOURCE.json", import.meta.url), "utf8"));
  for (const [name, source] of Object.entries(methods)) {
    assert.equal(blob(`./vendor/${name}`), source.git_blob_sha1, name);
    assert.equal(blob(`./vendor/${name}`), blob(`../../../app/web/vendor/${name}`), name);
  }
});
