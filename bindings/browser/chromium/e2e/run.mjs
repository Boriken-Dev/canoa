// The provider in a real browser: a real page under a strict CSP, the real extension, and an agent on a real socket proving
// itself with the real handshake. Drives the browser over its DevTools protocol, with the person's window clicked by script.
//
//   CANOA_E2E_BROWSER=path/to/chrome [CANOA_E2E_BRIDGE=path/to/coqui-acp-bridge] [CANOA_E2E_PAGES=dir] [CANOA_E2E_METHOD=noise]
//     node e2e/run.mjs
//
// CANOA_E2E_METHOD is the method every agent of the run proves by, `webcrypto` unless set: its in-process agent, the bridges it
// starts and the agents a person adds by hand.
//
// CANOA_E2E_PAGES holds the other languages' pages: wasm/page.js and page_bg.wasm from coqui-acp-page's example through
// wasm-bindgen --target web, and dart/page.js from app/tool/provider_page.dart through dart compile js.
//
// With a bridge, its companion is registered for this user during the run and removed after, replacing any registered before.
// Needs `npm install` in examples/acp-echo, for its WebSocket server.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createSocket } from "node:dgram";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import * as noiseMethod from "../vendor/coqui-peer-noise.mjs";
import * as webcryptoMethod from "../vendor/coqui-peer-webcrypto.mjs";
import { lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const { WebSocketServer } = createRequire(import.meta.url)("../../../../examples/acp-echo/node_modules/ws");
const browser = process.env.CANOA_E2E_BROWSER;
if (!browser) throw new Error("set CANOA_E2E_BROWSER");
const bridge = process.env.CANOA_E2E_BRIDGE;
const pages = process.env.CANOA_E2E_PAGES;
const METHOD = process.env.CANOA_E2E_METHOD ?? "webcrypto";
const { createResponder, generateIdentity } = METHOD === "noise" ? noiseMethod : webcryptoMethod;
// A handshake message as the method reads it: WebCrypto's are text, Noise's bytes.
const handshake = (data) => (METHOD === "noise" ? new Uint8Array(data) : data.toString());

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
// Canoa's own chat as the packaged UI loads the provider, without building Flutter for it.
mkdirSync(path.join(unpacked, "ui"));
writeFileSync(path.join(unpacked, "ui", "own.html"), '<!doctype html><title>own</title><script src="../acp-page.js"></script><script src="../acp-content.js"></script><p>own</p>');
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
        socket.send(await auth.challenge(handshake(data)));
      } else if (!session) {
        const accepted = await auth.accept(handshake(data));
        session = accepted.session;
        // Noise's responder finishes on the initiator's last message, with nothing more to send.
        if (accepted.reply !== undefined) socket.send(accepted.reply);
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

/** A port free for TCP and UDP alike. A system can reserve a number for one and not the other. */
async function freeForBoth() {
  for (;;) {
    const listener = createServer();
    await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
    const { port } = listener.address();
    const free = await new Promise((resolve) => {
      const socket = createSocket("udp4");
      socket.once("error", () => resolve(false));
      socket.bind(port, "127.0.0.1", () => socket.close(() => resolve(true)));
    });
    await new Promise((resolve) => listener.close(resolve));
    if (free) return port;
  }
}

let exitCode = 1;
let companionInstalled = false;
const bridges = [];
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome.kill();
  process.exit(1);
}, process.env.CANOA_E2E_REAL_AGENT ? 600_000 : 300_000);
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

  // Bounded, and saying which half never came: a revoke that hangs is what this has to catch.
  const revokeHere = async () => {
    const said = await page.run(`(async () => {
      const within = (promise) => Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve("timed out"), 15000))]);
      const revoked = await within(document.acp.revoke(window.agent).then(() => "answered"));
      if (revoked !== "answered") return "revoke never answered";
      const closed = await within(window.conn.closed);
      return closed === "timed out" ? "closed never resolved" : closed.reason;
    })()`);
    assert.equal(said, "revoked");
  };

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

  // A window already answered can still be closing, and answering it again would leave the new one waiting.
  const answeredWindows = new Set();
  const prompt = async () => {
    const target = await waitFor("the person's window", async () => (await targets()).find((item) => item.url.includes("acp-request.html?ask=") && !answeredWindows.has(item.id)));
    answeredWindows.add(target.id);
    const view = new Session(target.webSocketDebuggerUrl);
    await waitFor("its contents", () => view.run("(document.querySelector('#main')?.childElementCount ?? 0) > 0"));
    // Its buttons answer only after a moment.
    await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button')].some((button) => button.disabled)"));
    return view;
  };
  const requested = () => page.run(`window.asked = document.acp.request({ purpose: "testing" }).then(
      (agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
  const fill = (peer) => `(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      const set = (input, value) => { input.value = value; };
      set(inputs[0], 'Test agent'); set(inputs[1], '${agentUrl}'); set(inputs[2], '${peer}');
      [...document.querySelectorAll('fieldset')].find((set) => set.querySelector('legend')?.textContent === 'Add one by hand').querySelector('select').value = '${METHOD}';
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
        throw new Error(`${error.message}; it said: ${(await view.run("document.querySelector('#main').textContent")).split("Look for agents nearby")[1]?.slice(0, 500)}`);
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
      await revokeHere();
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
      await revokeHere();
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
    const bridgePeer = execFileSync(bridge, ["keygen", bridgeKey, "--method", METHOD], { encoding: "utf8" }).trim();
    const startBridge = async ({ key = bridgeKey, label = "Real bridge", flags, extra = [], program = [agentProgram] } = {}) => {
      const port = await freeForBoth();
      const transport = flags ?? ["--tls-short-lived", "--webtransport-port", String(port)];
      const child = spawn(bridge, ["serve", "--listen", `127.0.0.1:${port}`, "--identity", key, "--allow", browserPeer, ...transport, ...extra,
        "--advertise", label, "--", process.execPath, ...program],
      { stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, COQUI_ACP_DESCRIPTORS: descriptors, COQUI_AGENTIC_PEER_HOME: path.join(work, "bridge-home") } });
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
      const plainPeer = execFileSync(bridge, ["keygen", plainKey, "--method", METHOD], { encoding: "utf8" }).trim();
      await startBridge({ key: plainKey, label: "Plain bridge", flags: [] });
      await requested();
      const view = await prompt();
      await search(view, "Plain bridge");
      await choose(view, "Plain bridge", "/plain");
      assert.equal(await page.run("window.asked"), 1);
      assert.equal(await page.run("window.agent.identity.peer"), plainPeer);
      assert.deepEqual(await page.run("window.agent.offers.transports"), ["websocket"]);
      assert.deepEqual(await page.run(turn), [true, 1, "real"]);
      await revokeHere();
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
      await search(view, "Choosing this moves Real bridge here");
      await choose(view, "Choosing this moves Real bridge here", "/real");
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

    // An agent with a real MCP client, which starts the stdio server it is given and does what each prompt says.
    const toolLog = path.join(work, "tools.log");
    const toolAgent = path.join(work, "tool-agent.cjs");
    writeFileSync(toolAgent, `const { spawn } = require("node:child_process");
      const { appendFileSync } = require("node:fs");
      const readline = require("node:readline");
      const note = (entry) => appendFileSync(process.argv[2], JSON.stringify(entry) + "\\n");
      let shim;
      let next = 1;
      const waiting = new Map();
      const call = (method, params) => new Promise((resolve) => {
        const id = next++;
        waiting.set(id, resolve);
        shim.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\\n");
      });
      function start(server) {
        const env = { ...process.env };
        for (const { name, value } of server.env) env[name] = value;
        shim = spawn(server.command, server.args, { env, stdio: ["pipe", "pipe", "ignore"] });
        shim.on("exit", () => note({ shimExited: true }));
        readline.createInterface({ input: shim.stdout }).on("line", (line) => {
          const message = JSON.parse(line);
          if (message.method) note({ notified: message.method });
          else waiting.get(message.id)?.(message);
        });
      }
      const answer = (id, result) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\\n");
      readline.createInterface({ input: process.stdin }).on("line", async (line) => {
        const { id, method, params } = JSON.parse(line);
        if (method === "initialize") return answer(id, { protocolVersion: 1, agentCapabilities: {} });
        if (method === "session/new") {
          note({ servers: params.mcpServers });
          if (params.mcpServers.length > 0) {
            start(params.mcpServers[0]);
            const opened = await call("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } });
            note({ mcp: opened.result });
          }
          return answer(id, { sessionId: "tools" });
        }
        if (method === "session/prompt") {
          const asked = JSON.parse(params.prompt[0].text);
          note({ result: await call(asked.method, asked.params) });
          return answer(id, { stopReason: "end_turn" });
        }
        if (id !== undefined) answer(id, {});
      });`);
    const toolEntries = () => {
      try {
        return readFileSync(toolLog, "utf8").trim().split("\n").map((line) => JSON.parse(line));
      } catch {
        return [];
      }
    };
    const toolResult = async (session, method, params = {}) => {
      const before = toolEntries().filter((entry) => entry.result).length;
      const text = JSON.stringify(JSON.stringify({ method, params }));
      const ended = await session.run(`window.rpc({ jsonrpc: '2.0', id: window.nextId++, method: 'session/prompt', params: { sessionId: 'tools', prompt: [{ type: 'text', text: ${text} }] } })`);
      assert.equal(ended.result.stopReason, "end_turn");
      return (await waitFor("the agent's result", () => toolEntries().filter((entry) => entry.result)[before])).result;
    };
    const toolNames = async (session) => (await toolResult(session, "tools/list")).result.tools.map((tool) => tool.name);
    const callTool = (session, name, args) => toolResult(session, "tools/call", { name, arguments: args });
    const openUrl = async (url) => {
      const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
      const session = new Session(made.webSocketDebuggerUrl);
      await session.call("Runtime.enable");
      return session;
    };
    const browserTab = await openUrl(pageUrl);
    const newestTab = () => sw.run(`chrome.tabs.query({}).then((tabs) => Math.max(...tabs.filter((tab) => tab.url === ${JSON.stringify(pageUrl)}).map((tab) => tab.id)))`);
    const tabId = await newestTab();
    const own = await openUrl(`chrome-extension://${new URL(worker.url).hostname}/ui/own.html`);
    await waitFor("the provider in Canoa's own page", () => own.run("typeof document.acp === 'object'"));
    const answerBrowser = (view, allow) => view.run(allow
      ? `(() => {
          [...document.querySelectorAll('input[name=tab]')].find((input) => input.value === '${tabId}').click();
          [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow this tab').click();
        })()`
      : "[...document.querySelectorAll('button')].find((b) => b.textContent === 'Refuse').click()", { gesture: true }).catch(() => {});
    const askBrowser = async (allow) => {
      const asking = callTool(own, "browser_request_access", { capability: "browser.read", reason: "I would like to read the page" });
      const view = await prompt();
      const shown = await view.run("document.querySelector('#main').textContent");
      await answerBrowser(view, allow);
      return { answer: await asking, shown };
    };

    await check("tools: Canoa's own chat dials a link and its session gets a stdio server whose token is not in its arguments", async () => {
      execFileSync(bridge, ["keygen", path.join(work, "tools.key"), "--method", METHOD], { encoding: "utf8" });
      await startBridge({ key: path.join(work, "tools.key"), label: "Tool bridge", extra: ["--mcp-offers"], program: [toolAgent, toolLog] });
      await own.run(`window.asked = document.acp.request({ purpose: "Chat with it in Canoa" }).then((agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); window.nextId = 10; true`, { gesture: true });
      const view = await prompt();
      assert.ok(await view.run("document.querySelector('#main').textContent.includes(\"Canoa's chat asks\")"));
      assert.ok(!(await view.run("document.querySelector('#main').textContent.includes('can approve what the agent does')")));
      await search(view, "Tool bridge");
      assert.ok(await view.run("document.querySelector('#main').textContent.includes('mcp-offer/1')"));
      await choose(view, "Tool bridge", "/tools");
      assert.equal(await own.run("window.asked"), 1);
      assert.equal(await own.run(open), true);
      assert.equal((await own.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })")).result.protocolVersion, 1);
      assert.equal((await own.run("window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } })")).result.sessionId, "tools");
      const { servers } = toolEntries().find((entry) => entry.servers);
      assert.equal(servers.length, 1);
      assert.equal(servers[0].name, "canoa-browser");
      assert.ok(path.isAbsolute(servers[0].command));
      assert.equal(servers[0].type, "stdio");
      const tokenFile = servers[0].env.find((item) => item.name === "COQUI_MCP_OFFER_TOKEN_FILE").value;
      const token = readFileSync(tokenFile, "utf8");
      assert.ok(token.length >= 64 && !JSON.stringify(servers[0]).includes(token));
      assert.equal(toolEntries().find((entry) => entry.mcp).mcp.capabilities.tools.listChanged, true);
    });
    await check("tools: an agent sees only the asking tool, and no debugger in the normal build", async () => {
      assert.deepEqual(await toolNames(own), ["browser_request_access"]);
      const listed = await toolResult(own, "tools/list");
      assert.ok(!listed.result.tools[0].inputSchema.properties.capability.enum.includes("browser.debug"));
      const denied = await callTool(own, "browser_read_page", { tabId });
      assert.equal(denied.result.isError, true);
      const debug = await callTool(own, "browser_request_access", { capability: "browser.debug", reason: "x" });
      assert.equal(debug.result.isError, true);
      assert.equal((await targets()).filter((item) => item.url.includes("acp-request.html?ask=") && !answeredWindows.has(item.id)).length, 0);
    });
    await check("tools: the person is asked in the provider's window, the tool list grows, and the tool reads the real tab", async () => {
      const before = toolEntries().filter((entry) => entry.notified).length;
      const { answer, shown } = await askBrowser(true);
      assert.ok(shown.includes("In its own words: I would like to read the page"), shown);
      assert.ok(shown.includes(origin), shown);
      assert.equal(answer.result.isError, false);
      assert.equal(JSON.parse(answer.result.content[0].text).tabId, tabId);
      await waitFor("the list-changed notice", () => toolEntries().filter((entry) => entry.notified).length > before);
      assert.deepEqual(await toolNames(own), ["browser_request_access", "browser_list_tabs", "browser_read_page", "browser_list_webmcp"]);
      const read = await callTool(own, "browser_read_page", { tabId });
      assert.equal(read.result.isError, false);
      assert.equal(read.result.structuredContent.result.title, "strict page");
      assert.equal(read.result.structuredContent.result.text.trim(), "page");
      // Another tab of the same site was not what the person chose.
      const another = await openUrl(pageUrl);
      const anotherId = await newestTab();
      assert.notEqual(anotherId, tabId);
      assert.equal((await callTool(own, "browser_read_page", { tabId: anotherId })).result.isError, true);
      await another.call("Page.close").catch(() => {});
    });
    await check("tools: taking the grant away tells the agent, and the tool is gone", async () => {
      const before = toolEntries().filter((entry) => entry.notified).length;
      const options = await openUrl(`chrome-extension://${new URL(worker.url).hostname}/permissions.html`);
      await waitFor("the options page", () => options.run("document.readyState === 'complete' && typeof chrome?.runtime?.sendMessage === 'function'"));
      await options.run("chrome.runtime.sendMessage({ type: 'canoa.grants.list' }).then(({ grants }) => chrome.runtime.sendMessage({ type: 'canoa.grants.remove', grant: grants[0] })).then(() => true)");
      await options.call("Page.close").catch(() => {});
      await waitFor("the list-changed notice", () => toolEntries().filter((entry) => entry.notified).length > before);
      assert.deepEqual(await toolNames(own), ["browser_request_access"]);
      assert.equal((await callTool(own, "browser_read_page", { tabId })).result.isError, true);
    });
    await check("tools: navigating the tab away ends the grant", async () => {
      await askBrowser(true);
      assert.ok((await toolNames(own)).includes("browser_read_page"));
      await browserTab.run("location.reload(); true");
      await waitFor("the grant to go", async () => !(await toolNames(own)).includes("browser_read_page"));
    });
    await check("tools: the person refusing is what the agent hears, and it cannot ask again at once", async () => {
      const { answer } = await askBrowser(false);
      assert.equal(answer.result.isError, true);
      assert.deepEqual(await toolNames(own), ["browser_request_access"]);
      const again = await callTool(own, "browser_request_access", { capability: "browser.read", reason: "please" });
      assert.equal(again.result.isError, true);
      assert.ok(JSON.parse(again.result.content[0].text).why.includes("just now"));
    });
    await check("tools: a page's session with that same agent has no browser server", async () => {
      await requested();
      const view = await prompt();
      await choose(view, "Tool bridge", "/page");
      assert.equal(await page.run("window.asked"), 1);
      await page.run(open);
      const before = toolEntries().filter((entry) => entry.servers).length;
      await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })");
      await page.run("window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } })");
      const made = await waitFor("the session", () => toolEntries().filter((entry) => entry.servers)[before]);
      assert.deepEqual(made.servers, []);
      await revokeHere();
    });
    await check("tools: ending the connection ends the shim", async () => {
      await own.run("window.conn.close(); true");
      await waitFor("the shim to exit", () => toolEntries().some((entry) => entry.shimExited));
    });
    await check("tools: the same over a plain WebSocket, for an agent with no WebTransport", async () => {
      execFileSync(bridge, ["keygen", path.join(work, "plain-tools.key"), "--method", METHOD], { encoding: "utf8" });
      await startBridge({ key: path.join(work, "plain-tools.key"), label: "Plain tool bridge", flags: [], extra: ["--mcp-offers"], program: [toolAgent, toolLog] });
      await own.run(`window.asked = document.acp.request({}).then((agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
      const view = await prompt();
      await search(view, "Plain tool bridge");
      await choose(view, "Plain tool bridge", "/tools");
      assert.equal(await own.run("window.asked"), 1);
      assert.deepEqual(await own.run("window.agent.offers.transports"), ["websocket"]);
      assert.equal(await own.run(open), true);
      await own.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } })");
      await own.run("window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } })");
      assert.deepEqual(await toolNames(own), ["browser_request_access"]);
      await own.run("window.conn.close(); true");
    });
    // Spends a little of the person's subscription, in a temporary directory, and prints the shape of what happened, never its text.
    if (process.env.CANOA_E2E_REAL_AGENT === "claude") {
      await check("real agent: Claude asks for a tab, a person allows it, and it reads the page with a tool it did not have when it started", async () => {
        const cwd = path.join(work, "real-agent");
        mkdirSync(cwd);
        const key = path.join(work, "real.key");
        execFileSync(bridge, ["keygen", key, "--method", METHOD], { encoding: "utf8" });
        const npx = path.join(path.dirname(process.execPath), "node_modules", "npm", "bin", "npx-cli.js");
        await startBridge({ key, label: "Real Claude", flags: [], extra: ["--mcp-offers"], program: [npx, "-y", "@agentclientprotocol/claude-agent-acp@0.99.1"] });
        await own.run(`window.asked = document.acp.request({}).then((agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
        const view = await prompt();
        await search(view, "Real Claude");
        await choose(view, "Real Claude", cwd);
        assert.equal(await own.run("window.asked"), 1);
        assert.equal(await own.run(open), true);
        const task = "Use the browser_request_access tool with capability browser.read and any short reason. When it answers with a tabId, use the browser_read_page tool on that tabId, then reply with only the page's title. Do nothing else.";
        await own.run(`(() => {
          const said = { text: "", tools: [], asked: 0, stop: null, error: null, running: false, kinds: {} };
          window.said = said;
          const send = (message) => window.writer.write(JSON.stringify(message));
          (async () => {
            await send({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 2, capabilities: {}, info: { name: 'canoa-e2e', title: 'e2e', version: '1' } } });
            for (;;) {
              const { value, done } = await window.reader.read();
              if (done) return;
              const message = JSON.parse(value);
              if (message.method === undefined && message.id === 1) await send({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } });
              else if (message.method === undefined && message.id === 2) {
                if (message.error) said.error = message.error.message;
                else await send({ jsonrpc: '2.0', id: 3, method: 'session/prompt', params: { sessionId: message.result.sessionId, prompt: [{ type: 'text', text: ${JSON.stringify(task)} }] } });
              } else if (message.method === undefined && message.id === 3) {
                // In v2 this only says the message went in, and the end of the turn is a state update.
                if (message.error) said.stop = message.error.message;
              } else if (message.method === 'session/request_permission') {
                said.asked += 1;
                const allow = message.params.options.find((option) => String(option.kind).startsWith('allow'));
                await send({ jsonrpc: '2.0', id: message.id, result: { outcome: { outcome: 'selected', optionId: allow.optionId } } });
              } else if (message.method === 'session/update') {
                const update = message.params.update;
                const kind = update.sessionUpdate;
                said.kinds[kind] = (said.kinds[kind] ?? 0) + 1;
                // One piece of text, or a list of them.
                const words = Array.isArray(update.content) ? update.content.map((part) => part.text ?? '').join('') : update.content?.text ?? '';
                if (kind === 'agent_message_chunk' || kind === 'agent_message') said.text += words;
                if (kind === 'state_update' && update.state === 'running') said.running = true;
                if (kind === 'state_update' && update.state === 'idle' && said.running) said.stop = update.stopReason ?? 'end_turn';
                if ((kind === 'tool_call' || kind === 'tool_call_update') && update.title && !said.tools.includes(String(update.title))) said.tools.push(String(update.title));
              }
            }
          })().catch((error) => { said.error = String(error); });
          return true;
        })()`);
        const asking = await waitFor("Claude to ask for a tab", async () => {
          const said = await own.run("window.said");
          if (said.error || said.stop) throw new Error(`it ended without asking: ${said.error ?? said.stop}, ${said.tools.length} tools`);
          return (await targets()).find((item) => item.url.includes("acp-request.html?ask=") && !answeredWindows.has(item.id));
        }, 120_000);
        assert.ok(asking);
        const person = await prompt();
        await answerBrowser(person, true);
        let latest;
        const said = await waitFor("Claude to finish", async () => {
          latest = await own.run("window.said");
          return latest.stop || latest.error ? latest : null;
        }, 120_000).catch((error) => {
          throw new Error(`${error.message}: ${latest.asked} asked, kinds ${JSON.stringify(latest.kinds)}, running ${latest.running}`);
        });
        console.log(`     real agent: stop ${said.stop}, ${said.asked} permission questions, tools ${JSON.stringify(said.tools.map((title) => title.replace(/^.*(browser_[a-z_]+).*$/, "$1")))}, reply has the title: ${said.text.includes("strict page")}, kinds ${JSON.stringify(said.kinds)}`);
        assert.equal(said.error, null);
        assert.ok(said.text.includes("strict page"), `the reply did not have the title, ended ${said.stop}`);
        await own.run("window.conn.close(); true");
      });
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
