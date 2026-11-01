// The provider in a real browser: a real page under a strict CSP, the real extension, and an agent on a real socket proving
// itself with the real handshake. Drives the browser over its DevTools protocol, with the person's window clicked by script.
//
//   CANOA_E2E_BROWSER=path/to/chrome node e2e/run.mjs
//
// Needs `npm install` in examples/acp-echo, for its WebSocket server.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createResponder, generateIdentity } from "../vendor/coqui-peer-webcrypto.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const { WebSocketServer } = createRequire(import.meta.url)("../../../../examples/acp-echo/node_modules/ws");
const browser = process.env.CANOA_E2E_BROWSER;
if (!browser) throw new Error("set CANOA_E2E_BROWSER");

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-"));
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
// What a person's "let this site ask" grants, which needs Chrome's own prompt otherwise.
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
manifest.host_permissions = ["http://127.0.0.1/*"];
delete manifest.side_panel;
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The agent: the handshake, then sealed JSON-RPC. It records what reached it.
const identity = await generateIdentity();
const reached = [];
const sockets = new Set();
const agentServer = createServer();
const wss = new WebSocketServer({ server: agentServer });
wss.on("connection", (socket) => {
  sockets.add(socket);
  socket.on("close", () => sockets.delete(socket));
  let auth;
  let session;
  let promptId;
  let chain = Promise.resolve();
  const send = async (message) => socket.send(await session.seal(JSON.stringify(message)));
  socket.on("message", (data, binary) => {
    chain = chain.then(async () => {
      if (!auth) {
        auth = await createResponder(identity, { allowClient: async () => true });
        socket.send(await auth.challenge(data.toString()));
      } else if (!session) {
        const accepted = await auth.accept(data.toString());
        session = accepted.session;
        socket.send(accepted.reply);
      } else {
        assert.ok(binary, "after the proof every message is a record");
        const opened = await session.open(new Uint8Array(data));
        if (opened === null) return socket.close();
        const message = JSON.parse(new TextDecoder().decode(opened));
        reached.push(message);
        const { id, method, params } = message;
        if (method === "initialize") await send({ jsonrpc: "2.0", id, result: { protocolVersion: 1, agentCapabilities: {} } });
        else if (method === "session/new") await send({ jsonrpc: "2.0", id, result: { sessionId: "s1" } });
        else if (method === "session/prompt") {
          promptId = id;
          await send({ jsonrpc: "2.0", id: 900, method: "session/request_permission", params: {
            sessionId: "s1", toolCall: { toolCallId: "t1", title: "Run a command" },
            options: [{ optionId: "yes", name: "Yes", kind: "allow_once" }, { optionId: "no", name: "No", kind: "reject_once" }],
          } });
        } else if (id === 900 && method === undefined) {
          await send({ jsonrpc: "2.0", id: promptId, result: { stopReason: "end_turn" } });
        } else if (method === "session/resume") await send(params.sessionId === "s1" ? { jsonrpc: "2.0", id, result: {} } : { jsonrpc: "2.0", id, error: { code: -32602, message: "no such session" } });
        else if (id !== undefined && method !== undefined) await send({ jsonrpc: "2.0", id, result: {} });
      }
    }).catch(() => socket.close());
  });
});
await new Promise((resolve) => agentServer.listen(0, "127.0.0.1", resolve));
const agentUrl = `ws://127.0.0.1:${agentServer.address().port}/agent`;

// An agent with no proof at all: plain ACP in text frames.
const window_plain = [];
const plainServer = createServer();
const plainWss = new WebSocketServer({ server: plainServer });
plainWss.on("connection", (socket) => {
  socket.on("message", (data) => {
    const { id, method } = JSON.parse(data.toString());
    window_plain.push(method);
    socket.send(JSON.stringify({ jsonrpc: "2.0", id, result: { protocolVersion: 1, agentCapabilities: {} } }));
  });
});
await new Promise((resolve) => plainServer.listen(0, "127.0.0.1", resolve));
const plainUrl = `ws://127.0.0.1:${plainServer.address().port}/agent`;

// The page, as strict as a page gets: nothing inline, and no network but its own origin.
const CSP = "default-src 'none'; script-src 'self'; connect-src 'self'";
const site = createServer((request, response) => {
  if (request.url === "/app.js") {
    response.writeHead(200, { "content-type": "text/javascript", "content-security-policy": CSP });
    response.end("window.addEventListener('acp:ready', (event) => { window.found = event.detail.acp; }); window.dispatchEvent(new Event('acp:request')); window.ready = true;");
    return;
  }
  response.writeHead(200, { "content-type": "text/html", "content-security-policy": CSP });
  response.end('<!doctype html><title>strict page</title><script src="/app.js"></script><p>page</p>');
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const pageUrl = `http://127.0.0.1:${site.address().port}/`;

const chrome = spawn(browser, [
  "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
  "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
], { stdio: "ignore" });

class Session {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.pending = new Map();
    this.next = 1;
    this.opened = new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = reject;
    });
    // A click can close the window it was in, and then no answer comes.
    this.socket.onclose = () => {
      for (const waiting of this.pending.values()) waiting.reject(new Error("the target closed"));
      this.pending.clear();
    };
    this.socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      const waiting = this.pending.get(message.id);
      if (!waiting) return;
      this.pending.delete(message.id);
      if (message.error) waiting.reject(new Error(message.error.message));
      else waiting.resolve(message.result);
    };
  }
  async call(method, params = {}) {
    await this.opened;
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async run(expression, { gesture = false } = {}) {
    const reply = await this.call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, userGesture: gesture });
    if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text);
    return reply.result.value;
  }
}

let debug;
const targets = async () => (await fetch(`http://127.0.0.1:${debug}/json/list`)).json();
async function waitFor(what, test, ms = 15_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 300)}`);
  }
  console.log(results.at(-1));
}

let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log("FAILED: the run took too long");
  chrome.kill();
  process.exit(1);
}, 150_000);
try {
  debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const worker = await waitFor("the extension's service worker", async () =>
    (await targets()).find((target) => target.type === "service_worker" && target.url.startsWith("chrome-extension://") && target.url.endsWith("/background.js")), 20_000);
  const sw = new Session(worker.webSocketDebuggerUrl);
  // A service worker can be attached to a moment before its extension APIs exist.
  await waitFor("the extension's APIs", async () => (await sw.run("typeof chrome")) === "object");
  const origin = new URL(pageUrl).origin;
  await sw.run(`chrome.scripting.registerContentScripts([
    { id: "m", matches: ["${origin}/*"], js: ["acp-page.js"], runAt: "document_start", world: "MAIN" },
    { id: "i", matches: ["${origin}/*"], js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED" },
  ]).then(() => true)`);

  const created = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(pageUrl)}`, { method: "PUT" })).json();
  const page = new Session(created.webSocketDebuggerUrl);
  await page.call("Runtime.enable");

  await check("strict-csp: document.acp is there under the page's CSP, and found by event", async () => {
    await waitFor("the provider", () => page.run("typeof document.acp === 'object' && window.found === document.acp"));
    assert.equal(await page.run("document.acp.version"), 1);
  });
  await check("no-grant: a page sees no agents, and connecting is refused", async () => {
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 0);
    assert.equal(await page.run("document.acp.connect({ id: 'x', provider: 'dev.boriken.canoa' }).then(() => 'connected', (e) => e.name)"), "NotAllowedError");
  });
  await check("activation: request() without the person acting is refused, whatever the page claims", async () => {
    const name = await page.run("Object.defineProperty(navigator.userActivation, 'isActive', { get: () => true }); document.acp.request().then(() => 'granted', (e) => e.name)");
    assert.equal(name, "NotAllowedError");
  });

  const prompt = async () => {
    const target = await waitFor("the person's window", async () => (await targets()).find((item) => item.url.includes("acp-request.html?ask=")));
    const view = new Session(target.webSocketDebuggerUrl);
    await waitFor("its contents", () => view.run("document.querySelector('#main').childElementCount > 0"));
    return view;
  };
  const requested = () => page.run(`window.asked = document.acp.request({ purpose: "testing" }).then(
      (agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
  const fill = (peer) => `(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      const set = (input, value) => { input.value = value; };
      set(inputs[0], 'Test agent'); set(inputs[1], '${agentUrl}'); set(inputs[2], '${peer}');
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Add this agent').click();
    })()`;

  await check("person-refuses: closing with Refuse gives the page nothing", async () => {
    await requested();
    const view = await prompt();
    await view.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Refuse').click()").catch(() => {});
    assert.equal(await page.run("window.asked"), "NotAllowedError");
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 0);
  });
  await check("person-chooses: an agent added by hand in the prompt is granted", async () => {
    await requested();
    const view = await prompt();
    await view.run(fill(identity.peerId));
    await waitFor("the added agent", () => view.run("document.querySelectorAll('input[type=checkbox]:checked').length >= 1"));
    await view.run(`(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      inputs.at(-1).value = '/granted/dir';
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow').click();
    })()`).catch(() => {});
    assert.equal(await page.run("window.asked"), 1);
    assert.equal(await page.run("window.agent.label"), "Test agent");
    assert.equal(await page.run("window.agent.identity.peer"), identity.peerId);
  });
  const open = `(async () => {
    window.conn = await document.acp.connect(window.agent);
    window.writer = window.conn.writable.getWriter();
    window.reader = window.conn.readable.getReader();
    window.rpc = async (message) => {
      await window.writer.write(JSON.stringify(message));
      return JSON.parse((await window.reader.read()).value);
    };
    return window.conn.agent.identity.verified;
  })()`;
  await check("connect: through the page's strict connect-src, proven by the real handshake", async () => {
    assert.equal(await page.run(open), true);
    const init = await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })");
    assert.equal(init.result.protocolVersion, 1);
  });
  await check("scope: cwd replaced, a stdio server refused and never delivered, _meta removed", async () => {
    const refused = await page.run("window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/etc', mcpServers: [{ name: 'x', command: 'calc.exe', args: [] }] } })");
    assert.ok(refused.error, "a stdio server must be refused");
    const made = await page.run("window.rpc({ jsonrpc: '2.0', id: 3, method: 'session/new', _meta: { a: 1 }, params: { cwd: '/etc', mcpServers: [] } })");
    assert.equal(made.result.sessionId, "s1");
    const arrived = reached.filter((message) => message.method === "session/new");
    assert.equal(arrived.length, 1);
    assert.equal(arrived[0].params.cwd, "/granted/dir");
    assert.equal(arrived[0]._meta, undefined);
  });
  await check("sessions: another session is refused", async () => {
    const answer = await page.run("window.rpc({ jsonrpc: '2.0', id: 4, method: 'session/load', params: { sessionId: 'someone-elses', cwd: '/x', mcpServers: [] } })");
    assert.ok(answer.error);
    assert.equal(reached.filter((message) => message.method === "session/load").length, 0);
  });
  await check("permission-to-person: the provider's window answers, and the page never sees the request", async () => {
    await page.run("window.answering = window.rpc({ jsonrpc: '2.0', id: 6, method: 'session/prompt', params: { sessionId: 's1', prompt: [] } }); true");
    const view = await prompt();
    assert.match(await view.run("document.querySelector('#main').textContent"), /Run a command/);
    await view.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Yes').click()").catch(() => {});
    const answered = await page.run("window.answering");
    assert.equal(answered.id, 6, "the page's next message is its own answer, not the request");
    const decision = reached.find((message) => message.id === 900);
    assert.equal(decision.result.outcome.optionId, "yes");
  });
  await check("resume: after a reload the page finds its agent and resumes, with no new prompt", async () => {
    const before = sockets.size;
    assert.ok(before >= 1);
    await page.call("Page.reload");
    await waitFor("the old connection to end", () => sockets.size < before);
    await waitFor("the provider", () => page.run("window.ready === true && typeof document.acp === 'object'"));
    await page.run("document.acp.agents().then((agents) => { window.agent = agents[0]; return agents.length; })");
    assert.equal(await page.run("window.agent.label"), "Test agent");
    await page.run(open);
    await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })");
    const resumed = await page.run("window.rpc({ jsonrpc: '2.0', id: 5, method: 'session/resume', params: { sessionId: 's1', cwd: '/x', mcpServers: [] } })");
    assert.ok(resumed.result, JSON.stringify(resumed));
    assert.equal(reached.filter((message) => message.method === "session/resume").at(-1).params.cwd, "/granted/dir");
  });
  await check("revoke: closes at once, and connecting is refused afterwards", async () => {
    const reason = await page.run("document.acp.revoke(window.agent).then(() => window.conn.closed).then((c) => c.reason)");
    assert.equal(reason, "revoked");
    assert.equal(await page.run("document.acp.connect(window.agent).then(() => 'connected', (e) => e.name)"), "NotAllowedError");
    await waitFor("the socket to end", () => sockets.size === 0);
  });
  await check("unverified: an agent added with no peer id connects, and says it is unverified", async () => {
    await requested();
    const view = await prompt();
    await view.run(`(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      inputs[0].value = 'Plain agent'; inputs[1].value = '${plainUrl}'; inputs[2].value = '';
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Add this agent').click();
    })()`);
    await waitFor("the added agent", () => view.run("document.querySelectorAll('input[type=checkbox]:checked').length >= 1"));
    await view.run(`(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      inputs.at(-1).value = '/plain';
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow').click();
    })()`).catch(() => {});
    assert.equal(await page.run("window.asked"), 1);
    assert.equal(await page.run("window.agent.identity.verified"), false);
    const plain = window_plain;
    assert.equal(await page.run(`(async () => {
      const conn = await document.acp.connect(window.agent);
      const writer = conn.writable.getWriter();
      const reader = conn.readable.getReader();
      await writer.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } }));
      const answer = JSON.parse((await reader.read()).value);
      await document.acp.revoke(window.agent);
      await conn.closed;
      return answer.id === 1 && conn.agent.identity.verified;
    })()`), false);
    assert.equal(plain.length, 1);
  });
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`ABORTED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  chrome.kill();
  await sleep(1500);
  for (const socket of sockets) socket.terminate();
  wss.close();
  agentServer.close();
  plainWss.close();
  plainServer.close();
  site.close();
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
  } catch {
    // The browser may still hold its profile for a moment.
  }
}
console.log(exitCode === 0 ? "PASS" : "FAILED");
process.exit(exitCode);
