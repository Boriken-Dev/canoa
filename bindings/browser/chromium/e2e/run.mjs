// The provider in a real browser: a real page under a strict CSP, the real extension, and an agent on a real socket proving
// itself with the real handshake. Drives the browser over its DevTools protocol, with the person's window clicked by script.
//
//   CANOA_E2E_BROWSER=path/to/chrome [CANOA_E2E_BRIDGE=path/to/coqui-acp-bridge] [CANOA_E2E_PAGES=dir] node e2e/run.mjs
//
// CANOA_E2E_PAGES holds the other languages' pages: wasm/page.js and page_bg.wasm from coqui-acp-page's example through
// wasm-bindgen --target web, and dart/page.js from app/tool/provider_page.dart through dart compile js.
//
// With a bridge, its companion is registered for this user during the run and removed after, replacing any registered before.
// Needs `npm install` in examples/acp-echo, for its WebSocket server.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
const bridge = process.env.CANOA_E2E_BRIDGE;
const pages = process.env.CANOA_E2E_PAGES;

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-"));
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
const descriptors = path.join(work, "agents");
mkdirSync(descriptors);
cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
// What a person's "let this site ask" grants, which needs Chrome's own prompt otherwise.
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
manifest.host_permissions = ["http://127.0.0.1/*"];
delete manifest.side_panel;
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The agent: the handshake, then sealed JSON-RPC. It records what reached it.
const identity = await generateIdentity();
// One key per agent, by the path it is reached on: the same key at another address would be the same agent.
const foundIdentity = await generateIdentity();
const fallbackIdentity = await generateIdentity();
const identities = { "/agent/found": foundIdentity, "/agent/fallback": fallbackIdentity };
const reached = [];
let browserPeer;
const sockets = new Set();
const agentServer = createServer();
const wss = new WebSocketServer({ server: agentServer });
wss.on("connection", (socket, request) => {
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
        auth = await createResponder(identities[request.url] ?? identity, { allowClient: async (peer) => {
          browserPeer = peer;
          return true;
        } });
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
// A page in another language, exactly as strict but for what loading WebAssembly needs.
const LANGUAGE_CSP = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'";
const TYPES = { ".js": "text/javascript", ".wasm": "application/wasm" };
const site = createServer((request, response) => {
  if (pages && request.url.startsWith("/pages/")) {
    const file = path.join(pages, ...request.url.slice("/pages/".length).split("/").filter((part) => part !== ".."));
    response.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream", "content-security-policy": LANGUAGE_CSP });
    response.end(readFileSync(file));
    return;
  }
  if (request.url === "/wasm-app.js") {
    response.writeHead(200, { "content-type": "text/javascript", "content-security-policy": LANGUAGE_CSP });
    response.end("import init, { ask_for_agents, run_turn } from '/pages/wasm/page.js'; await init(); window.language = { ask: () => ask_for_agents('from Rust'), turn: run_turn }; window.ready = true;");
    return;
  }
  if (request.url === "/wasm.html" || request.url === "/dart.html") {
    const script = request.url === "/wasm.html" ? '<script type="module" src="/wasm-app.js"></script>' : '<script src="/pages/dart/page.js"></script><script src="/dart-app.js"></script>';
    response.writeHead(200, { "content-type": "text/html", "content-security-policy": LANGUAGE_CSP });
    response.end(`<!doctype html><title>page</title>${script}<p>page</p>`);
    return;
  }
  if (request.url === "/dart-app.js") {
    response.writeHead(200, { "content-type": "text/javascript", "content-security-policy": LANGUAGE_CSP });
    response.end("window.language = { ask: () => window.canoaAsk(), turn: (label, text) => window.canoaTurn(label, text) }; window.ready = true;");
    return;
  }
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
], { stdio: "ignore", env: { ...process.env, COQUI_ACP_DESCRIPTORS: descriptors } });

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
let companionInstalled = false;
const bridges = [];
const watchdog = setTimeout(() => {
  console.log("FAILED: the run took too long");
  chrome.kill();
  process.exit(1);
}, 360_000);
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
    await waitFor("its contents", () => view.run("(document.querySelector('#main')?.childElementCount ?? 0) > 0"));
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
  if (bridge) {
    execFileSync(bridge, ["companion", "install", "--extension-id", new URL(worker.url).hostname], { stdio: "ignore" });
    companionInstalled = true;
    const describe = (name, label, peer, tail) => writeFileSync(path.join(descriptors, name), JSON.stringify({ txtvers: 1, proto: "acp", label, url: `${agentUrl}/${tail}`, peer }));
    const search = async (view, label) => {
      await view.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Look for agents nearby').click()");
      try {
        await waitFor(`${label} in the window`, () => view.run(`document.querySelector('#main').textContent.includes(${JSON.stringify(label)})`), 20_000);
      } catch (error) {
        throw new Error(`${error.message}; it showed: ${await view.run("document.querySelector('#main').textContent")}`);
      }
    };
    const choose = (view, label, cwd) => view.run(`(() => {
      const row = [...document.querySelectorAll('label.pick')].find((item) => item.textContent.includes(${JSON.stringify(label)}));
      row.querySelector('input[type=checkbox]').click();
      [...document.querySelectorAll('input[type=text]')].at(-1).value = ${JSON.stringify(cwd)};
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow').click();
    })()`).catch(() => {});

    await check("discovery: the companion finds an agent, shown unticked with its key, and the page sees nothing until it is chosen", async () => {
      describe("found.json", "Found agent", foundIdentity.peerId, "found");
      await requested();
      const view = await prompt();
      await search(view, "Found agent");
      const row = await view.run(`(() => {
        const item = [...document.querySelectorAll('label.pick')].find((each) => each.textContent.includes('Found agent'));
        return { ticked: item.querySelector('input').checked, text: item.textContent };
      })()`);
      assert.equal(row.ticked, false);
      assert.ok(row.text.includes(foundIdentity.peerId), row.text);
      assert.ok(row.text.includes("on this machine"), row.text);
      const seen = await page.run("document.acp.agents().then((agents) => agents.map((agent) => agent.label))");
      assert.ok(!seen.includes("Found agent"), seen.join());
      await choose(view, "Found agent", "/found");
      assert.equal(await page.run("window.asked"), 1);
      assert.equal(await page.run("window.agent.label"), "Found agent");
      assert.equal(await page.run("window.agent.identity.verified"), false, "shown is not proven");
    });
    await check("discovery: chosen and connected, the real handshake proves the key it named", async () => {
      assert.equal(await page.run(open), true);
      const init = await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })");
      assert.equal(init.result.protocolVersion, 1);
      await page.run("document.acp.revoke(window.agent).then(() => window.conn.closed)");
    });
    await check("fallback: WebTransport that does not open falls back to the WebSocket, under the same proof", async () => {
      writeFileSync(path.join(descriptors, "fallback.json"), JSON.stringify({ txtvers: 1, proto: "acp", label: "Fallback agent", url: `${agentUrl}/fallback`,
        peer: fallbackIdentity.peerId, wt: 1, wth: `sha256:${"0".repeat(64)}` }));
      await requested();
      const view = await prompt();
      await search(view, "Fallback agent");
      await choose(view, "Fallback agent", "/fallback");
      assert.equal(await page.run("window.asked"), 1);
      assert.deepEqual(await page.run("window.agent.offers.transports"), ["webtransport", "websocket"]);
      assert.equal(await page.run(open), true);
      await page.run("document.acp.revoke(window.agent).then(() => window.conn.closed)");
    });
    await check("spoof: an advertisement naming a key the agent does not hold is refused at connect", async () => {
      const someoneElse = await generateIdentity();
      describe("spoof.json", "Spoofed agent", someoneElse.peerId, "spoof");
      const before = reached.length;
      await requested();
      const view = await prompt();
      await search(view, "Spoofed agent");
      await choose(view, "Spoofed agent", "/spoof");
      assert.equal(await page.run("window.asked"), 1);
      assert.equal(await page.run("window.agent.label"), "Spoofed agent");
      assert.equal(await page.run("document.acp.connect(window.agent).then(() => 'connected', (e) => e.name)"), "NetworkError");
      assert.equal(reached.length, before, "no ACP message may reach anyone");
    });

    // The real bridge and a real stdio agent. It serves a certificate no browser would take for a WebSocket, so a turn that
    // completes went over WebTransport.
    const agentProgram = path.join(work, "agent.mjs");
    writeFileSync(agentProgram, `import { createInterface } from "node:readline";
      createInterface({ input: process.stdin }).on("line", (line) => {
        const { id, method } = JSON.parse(line);
        if (id === undefined) return;
        const result = method === "initialize" ? { protocolVersion: 1, agentCapabilities: {} } : method === "session/new" ? { sessionId: "real" } : method === "session/prompt" ? { stopReason: "end_turn" } : {};
        process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\\n");
      });`);
    const bridgeKey = path.join(work, "bridge.key");
    const bridgePeer = execFileSync(bridge, ["keygen", bridgeKey], { encoding: "utf8" }).trim();
    const startBridge = async ({ key = bridgeKey, label = "Real bridge", flags } = {}) => {
      const listener = createServer();
      await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
      const { port } = listener.address();
      await new Promise((resolve) => listener.close(resolve));
      const transport = flags ?? ["--tls-short-lived", "--webtransport-port", String(port)];
      const child = spawn(bridge, ["serve", "--listen", `127.0.0.1:${port}`, "--identity", key, "--allow", browserPeer, ...transport,
        "--advertise", label, "--", process.execPath, agentProgram],
      { stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, COQUI_ACP_DESCRIPTORS: descriptors } });
      let said = "";
      child.stderr.on("data", (chunk) => {
        said += chunk;
      });
      await waitFor("the bridge to listen", () => said.includes("listening on"));
      bridges.push(child);
      return { child, said: () => said };
    };
    const turn = `(async () => {
      await page_open();
      const init = await window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } });
      const made = await window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } });
      return [window.conn.agent.identity.verified, init.result.protocolVersion, made.result.sessionId];
    })()`.replace("page_open()", open);
    await check("websocket: a whole exchange over a WebSocket, through the real bridge, to a real stdio agent", async () => {
      const plainKey = path.join(work, "plain.key");
      const plainPeer = execFileSync(bridge, ["keygen", plainKey], { encoding: "utf8" }).trim();
      await startBridge({ key: plainKey, label: "Plain bridge", flags: [] });
      await requested();
      const view = await prompt();
      await search(view, "Plain bridge");
      await choose(view, "Plain bridge", "/plain");
      assert.equal(await page.run("window.asked"), 1);
      assert.equal(await page.run("window.agent.identity.peer"), plainPeer);
      assert.deepEqual(await page.run("window.agent.offers.transports"), ["websocket"]);
      assert.deepEqual(await page.run(turn), [true, 1, "real"]);
      await page.run("document.acp.revoke(window.agent).then(() => window.conn.closed)");
    });
    let first;
    await check("webtransport: a whole exchange over WebTransport with a pinned certificate, through the real bridge, to a real stdio agent", async () => {
      first = await startBridge();
      await requested();
      const view = await prompt();
      await search(view, "Real bridge");
      await choose(view, "Real bridge", "/real");
      assert.equal(await page.run("window.asked"), 1);
      assert.equal(await page.run("window.agent.identity.peer"), bridgePeer);
      assert.deepEqual(await page.run(turn), [true, 1, "real"]);
      assert.ok(first.said().includes(`${browserPeer} proven`), first.said());
      await page.run("window.conn.close ? window.conn.close() : null; true");
    });
    await check("restart: a new certificate, found again and chosen, moves the same agent, whose grant holds", async () => {
      first.child.kill();
      await waitFor("the bridge to go", () => first.child.exitCode !== null || first.child.signalCode !== null);
      await startBridge();
      await page.call("Page.reload");
      await waitFor("the provider", () => page.run("window.ready === true && typeof document.acp === 'object'"));
      await requested();
      const view = await prompt();
      await search(view, "Choosing this moves it here");
      const row = await view.run(`[...document.querySelectorAll('label.pick')].find((each) => each.textContent.includes('Choosing this moves it here')).textContent`);
      assert.ok(row.includes("Real bridge"), row);
      await choose(view, "Choosing this moves it here", "/real");
      assert.equal(await page.run("window.asked"), 1);
      assert.deepEqual(await page.run(turn), [true, 1, "real"]);
    });
    if (pages) {
      const languagePage = async (url) => {
        const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(`${pageUrl}${url}`)}`, { method: "PUT" })).json();
        const session = new Session(made.webSocketDebuggerUrl);
        await session.call("Runtime.enable");
        await waitFor("the language's page", () => session.run("window.ready === true && typeof document.acp === 'object'"));
        return session;
      };
      const grantToLanguage = async (session) => {
        await session.run("window.asked = window.language.ask().then((count) => count, (error) => String(error)); true", { gesture: true });
        const view = await prompt();
        await choose(view, "Real bridge", "/language");
        assert.equal(await session.run("window.asked"), 1);
      };
      await check("wasm: a Rust page compiled to WebAssembly asks for an agent and runs a turn through the extension", async () => {
        const session = await languagePage("wasm.html");
        await grantToLanguage(session);
        const [initialized, made, answered] = JSON.parse(await session.run("window.language.turn('Real bridge', 'hello from Rust')"));
        assert.equal(JSON.parse(initialized).result.protocolVersion, 1);
        assert.equal(JSON.parse(made).result.sessionId, "real");
        assert.equal(JSON.parse(answered).result.stopReason, "end_turn");
      });
      await check("dart: Canoa's own connection code asks for an agent and runs a turn through the extension", async () => {
        const session = await languagePage("dart.html");
        await grantToLanguage(session);
        const answer = JSON.parse(await session.run("window.language.turn('Real bridge', 'hello from Canoa')"));
        assert.equal(answer.peer, bridgePeer);
        assert.equal(answer.session, "real");
        assert.equal(answer.stop, "end_turn");
      });
    } else {
      console.log("skipped: wasm and dart need CANOA_E2E_PAGES");
    }
  } else {
    console.log("skipped: discovery and spoof need CANOA_E2E_BRIDGE");
  }
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`ABORTED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  if (companionInstalled) execFileSync(bridge, ["companion", "uninstall"], { stdio: "ignore" });
  for (const child of bridges) child.kill();
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
