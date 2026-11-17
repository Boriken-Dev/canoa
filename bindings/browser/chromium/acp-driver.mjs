// A driver for coqui-acp's conformance checks, and for this extension's own tests of the contract: the real page script and content
// script, in a fake page, joined to the real gateway.
import { readFileSync } from "node:fs";
import vm from "node:vm";

import { createGateway, sameGrant, supersedes } from "./acp-gateway.js";

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

export class Agent {
  received_ = [];
  next = 0;
  listeners = [];
  closers = [];
  // How the link ends when the agent does not ask for it: "closed" in order, "cut" with no close from the agent.
  ending = undefined;
  hangUp(ending) {
    this.ending = ending;
    for (const closed of this.closers) closed();
  }
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
      const kept = grants.filter((held) => !supersedes(grant, held));
      grants.splice(0, grants.length, ...kept, grant);
    },
    async deleteGrant(grant) {
      grants.splice(0, grants.length, ...grants.filter((held) => !sameGrant(held, grant)));
    },
    secret: async () => "test-secret",
    known: agents,
  };
}

export function makeDriver() {
  const remote = new Map();
  const store = memoryStore();
  const asked = [];
  const channels = [];
  let dials = 0;
  let dialGate;
  const windows = [];
  const permissionWindows = [];
  let holdPermissions = false;
  let answer;
  const gateway = createGateway({
    store,
    dial: async (agent) => {
      const peer = remote.get(agent.key);
      dials += 1;
      if (dialGate !== undefined) await dialGate;
      const channel = {
        closedByGateway: false,
        proven: agent.proven !== false,
        send: async (text) => peer.deliver(text),
        onMessage: (handler) => peer.listeners.splice(0, peer.listeners.length, handler),
        get ending() {
          return peer.ending;
        },
        onClose: (handler) => peer.closers.splice(0, peer.closers.length, handler),
        close: () => {
          channel.closedByGateway = true;
        },
      };
      channels.push(channel);
      return channel;
    },
    askPerson: async (details) => {
      const decision = answer;
      answer = undefined;
      if (decision?.hold) {
        // A window that stays open until the person answers it (`window.answer`) or the provider closes it (the signal).
        const window = { details, closed: false, answer: undefined };
        windows.push(window);
        const chosen = await new Promise((resolve) => {
          window.answer = resolve;
          details.signal?.addEventListener("abort", () => {
            window.closed = true;
            resolve(null);
          });
        });
        return chosen === null ? null : decide(chosen);
      }
      return decide(decision);
    },
    askPermission: async (details) => {
      asked.push(details.request);
      if (!holdPermissions) return null;
      const window = { details, closed: false, answer: undefined };
      permissionWindows.push(window);
      return new Promise((resolve) => {
        window.answer = resolve;
        details.signal?.addEventListener("abort", () => {
          window.closed = true;
          resolve(null);
        });
      });
    },
  });

  function decide(decision) {
    if (decision === undefined || decision.refuse) return null;
    for (const label of decision.add ?? []) remote.set(label, new Agent());
    return {
      allow: [decision.allow].flat().filter((label) => !(decision.add ?? []).includes(label)),
      add: (decision.add ?? []).map((label) => ({ label, peer: `sha256:${label}` })),
      cwd: decision.cwd,
      permissions: decision.permissions,
      mcpOrigins: decision.mcpOrigins,
      lifetime: decision.lifetime,
    };
  }

  const page = (origin, { tabId = 1 } = {}) => {
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
          gateway.attach(inner, { origin, tabId });
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
      // The windows held open by `answerNext({ hold: true, ... })`, and the permission windows while `holdPermissions` is on.
      windows,
      permissionWindows,
      holdPermissions(on = true) {
        holdPermissions = on;
      },
    },
    store,
    gateway,
    channels,
    get dials() {
      return dials;
    },
    // Dials wait until the returned function is called.
    holdDials() {
      let release;
      dialGate = new Promise((resolve) => {
        release = () => {
          dialGate = undefined;
          resolve();
        };
      });
      return release;
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

