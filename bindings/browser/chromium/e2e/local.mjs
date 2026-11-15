// The extension using the Canoa on this computer by default, in a real browser: it registers the native messaging host for the
// run and removes it, starts `canoa serve` on a home of its own, and drives the extension's own pages over DevTools.
//
//   CANOA_E2E_BROWSER=path/to/msedge.exe CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent
//     [CANOA_E2E_NATIVE_BROWSER=edge|chrome|chromium] node e2e/local.mjs
//
// The browser gets a throwaway profile, and `COQUI_AGENTIC_PEER_HOME` names a home of the run's own, which its native host
// inherits. Registering uses the user's registry keys (Windows) for `dev.boriken.canoa`: a run refuses to start when that is
// already registered, so a person's own registration is never replaced. Needs `npm install` in e2e/, for the ws package.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { createServer as listen } from "node:net";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { findCanoaWorker, lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const { WebSocketServer } = createRequire(import.meta.url)("ws");
const browser = process.env.CANOA_E2E_BROWSER;
const canoa = process.env.CANOA_E2E_HOST;
const agent = process.env.CANOA_E2E_AGENT;
if (!browser || !canoa || !agent) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST and CANOA_E2E_AGENT");
const NATIVE_BROWSER = process.env.CANOA_E2E_NATIVE_BROWSER ?? "edge";
const HOST_NAME = "dev.boriken.canoa";
const KEYS = { edge: "Microsoft\\Edge", chrome: "Google\\Chrome", chromium: "Chromium" };

if (process.platform === "win32") {
  const present = (key) => {
    try {
      execFileSync("reg", ["query", `HKCU\\Software\\${key}\\NativeMessagingHosts\\${HOST_NAME}`], { stdio: "ignore" });
      return true;
    } catch {
      return false;
    }
  };
  for (const key of Object.values(KEYS)) {
    if (present(key)) throw new Error(`${HOST_NAME} is already registered under ${key}: remove it first (canoa native uninstall)`);
  }
}

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-local-"));
const home = path.join(work, "home");
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
const config = path.join(work, "canoa.cfg");
mkdirSync(home);
const pageServer = createServer((request, response) => {
  response.writeHead(200, { "content-type": "text/html" });
  response.end("<!doctype html><title>a page</title><p>page</p>");
});
await new Promise((resolve) => pageServer.listen(0, "127.0.0.1", resolve));
const pageUrl = `http://127.0.0.1:${pageServer.address().port}/`;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const quoted = (text) => JSON.stringify(text);
writeFileSync(config, [
  "[host.agents.scripted]",
  `program = ${quoted(agent)}`,
  "title = \"Scripted\"",
  "",
  // The person's own agent command: no address, reached over the host's own link, and so over the door.
  "[agents.standin]",
  `program = ${quoted(agent)}`,
  "",
].join("\n"));

cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
delete manifest.side_panel;
// A content script may run in the page this run serves, to show what such a script cannot do.
manifest.host_permissions = ["http://127.0.0.1/*"];
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));
mkdirSync(path.join(unpacked, "ui"));
writeFileSync(path.join(unpacked, "ui", "own.html"), '<!doctype html><title>own</title><script src="../acp-page.js"></script><script src="../acp-content.js"></script><p>own</p>');

// The folders the host makes for sessions of the extension's own pages go in the run's own Documents.
const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: home, XDG_DOCUMENTS_DIR: path.join(work, "documents") };

// Two descriptor folders, each naming an agent that is really listening on loopback: the host reads the first, the native host the
// browser starts (and anything it starts) reads the second, so a result says which of them answered.
const PEER = `sha256:${"aa".repeat(32)}`;
const advertised = [];
async function advertise(label) {
  const dir = path.join(work, `descriptors-${label}`);
  mkdirSync(dir);
  const socket = listen();
  await new Promise((resolve) => socket.listen(0, "127.0.0.1", resolve));
  const url = `ws://127.0.0.1:${socket.address().port}/acp`;
  writeFileSync(path.join(dir, "agent.json"), JSON.stringify({ txtvers: 1, proto: "acp", label, url, peer: PEER, method: "noise" }));
  advertised.push(socket);
  return dir;
}
const viaHost = await advertise("Found by the host");
const viaRelay = await advertise("Found by the relay");
const hostEnv = { ...env, COQUI_ACP_DESCRIPTORS: viaHost };
const browserEnv = { ...env, COQUI_ACP_DESCRIPTORS: viaRelay };
const run = (args, options = {}) => execFileSync(canoa, args, { encoding: "utf8", env, ...options });

let hostProcess;
/** Starts `canoa serve` on the run's home and answers once it says it is ready. */
async function startHost() {
  const child = spawn(canoa, ["serve", "--config", config, "--local-sources", "--method", "noise"], { env: hostEnv, stdio: ["pipe", "pipe", "ignore"] });
  let said = "";
  child.stdout.on("data", (chunk) => {
    said += chunk;
  });
  await waitFor("the host to be ready", () => said.includes('"ready"'), 30_000);
  hostProcess = child;
  return child;
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
async function check(name, body) {
  try {
    await body();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 400)}`);
  }
  console.log(results.at(-1));
}

let exitCode = 1;
let registered = false;
let chrome;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  hostProcess?.kill();
  process.exit(1);
}, 420_000);
try {
  await startHost();
  const installed = run(["native", "install", "--browser", NATIVE_BROWSER, "--launch", canoa, "--", "serve", "--config", config, "--local-sources", "--method", "noise"]);
  registered = true;
  await check("install: the native host is registered for the browser and records what Start runs", async () => {
    const status = JSON.parse(run(["native", "status"]));
    assert.ok(status.registered.length >= 1, installed);
    assert.equal(status.running, true);
    assert.equal(path.basename(status.launch.program).toLowerCase(), path.basename(canoa).toLowerCase());
  });

  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
    "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
  ], { stdio: "ignore", env: browserEnv });
  debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const worker = await waitFor("the extension's service worker", async () => findCanoaWorker(await targets()), 20_000);
  const sw = new Session(worker.webSocketDebuggerUrl);
  await waitFor("the extension's APIs", async () => (await sw.run("typeof chrome")) === "object");
  const id = new URL(worker.url).hostname;
  const openTab = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
    const session = new Session(made.webSocketDebuggerUrl);
    await session.call("Runtime.enable");
    return session;
  };
  const options = await openTab(`chrome-extension://${id}/permissions.html`);
  const own = await openTab(`chrome-extension://${id}/ui/own.html`);
  await waitFor("Canoa's own page to have the provider", () => own.run("typeof document.acp === 'object'"));
  const ask = (type, extra = {}) => options.run(`chrome.runtime.sendMessage(${JSON.stringify({ type, ...extra })})`);
  const hostPeer = run(["id", "--method", "noise"]).trim();

  await check("fixed id: the unpacked extension has the id the native host's manifest allows", async () => {
    assert.equal(id, "lpppofcphpjkedfnpbpabhmiamaicaed");
  });

  await check("hello: the extension finds the host running, with its scripted entry and the stand-in over the door, and is not paired", async () => {
    const view = await ask("canoa.host.status");
    assert.equal(view.state, "running", JSON.stringify(view));
    assert.equal(view.paired, false);
    assert.equal(view.canPin, true);
    assert.equal(view.host.peer, hostPeer);
    const names = view.host.entries.map((entry) => entry.name);
    assert.ok(names.includes("scripted") && names.includes("standin"), names.join());
  });

  const dial = (peer, entry) => options.run(`import("./acp-dial.js").then(async (d) => {
    try {
      const link = await d.dialAgent({ native: { entry: ${quoted(entry)} }, peer: ${quoted(peer)}, method: "noise" }, {});
      link.close();
      return "served";
    } catch (e) { return "refused: " + (e.message ?? e); }
  })`);

  await check("unpaired: this browser's key is refused over the door and waits on the host", async () => {
    const said = await dial(hostPeer, "scripted");
    assert.match(said, /^refused/, said);
    const waiting = run(["peers", "--format", "json"]).split("\n").filter(Boolean).map((line) => JSON.parse(line));
    assert.ok(waiting.some((peer) => peer.state === "waiting"), JSON.stringify(waiting));
    // Not the way in: a person pairs it by PIN, so the waiting key is removed again.
    for (const peer of waiting.filter((item) => item.state === "waiting")) run(["revoke", peer.peer]);
  });

  await check("claimed peer: a relay that claims another host's key fails the proof", async () => {
    const other = `sha256:${"11".repeat(32)}`;
    const said = await dial(other, "scripted");
    assert.match(said, /^refused/, said);
  });

  await check("pair: first contact is a PIN shown by the host's computer, typed once, and then the host is paired", async () => {
    const pairing = spawn(canoa, ["pair", "--native", "--method", "noise"], { env, stdio: ["pipe", "pipe", "ignore"] });
    let line = "";
    pairing.stdout.on("data", (chunk) => {
      line += chunk;
    });
    const shown = JSON.parse(await waitFor("the PIN", () => (line.includes("\n") ? line.split("\n")[0] : null), 15_000));
    assert.equal(shown.peer, hostPeer);
    const wrong = shown.pin.slice(0, -1) + (shown.pin.endsWith("0") ? "1" : "0");
    // A wrong PIN is spent.
    const refused = await ask("canoa.host.pair", { pin: wrong });
    assert.ok(refused.error, JSON.stringify(refused));
    // And a PIN is not asked for twice: the pairing door shuts with it.
    const second = await ask("canoa.host.pair", { pin: shown.pin });
    assert.ok(second.error, JSON.stringify(second));
    pairing.kill();
    const again = spawn(canoa, ["pair", "--native", "--method", "noise"], { env, stdio: ["pipe", "pipe", "ignore"] });
    let text = "";
    again.stdout.on("data", (chunk) => {
      text += chunk;
    });
    const fresh = JSON.parse(await waitFor("a new PIN", () => (text.includes("\n") ? text.split("\n")[0] : null), 15_000));
    const view = await ask("canoa.host.pair", { pin: fresh.pin });
    assert.equal(view.paired, true, JSON.stringify(view));
    await waitFor("pairing to end", () => again.exitCode !== null, 15_000);
    assert.equal(again.exitCode, 0);
    assert.equal(view.host.peer, hostPeer);
  });

  await check("agents: Canoa's own page holds the host's entries with nothing to ask, beside any added by hand", async () => {
    await ask("canoa.host.status");
    const labels = JSON.parse(await own.run("document.acp.agents().then((agents) => JSON.stringify(agents.map((a) => ({ label: a.label, source: a.source, verified: a.identity.verified, peer: a.identity.peer }))))"));
    const found = labels.map((item) => item.label);
    assert.ok(found.includes("Scripted") && found.includes("standin"), found.join());
    assert.ok(labels.every((item) => item.source === "local" && item.peer === hostPeer));
  });

  // The provider's own window asks who is nearby; a tab of that page speaks to the worker as the window does.
  const lookNearby = async () => {
    const page = await openTab(`chrome-extension://${id}/acp-request.html`);
    try {
      return await page.run("chrome.runtime.sendMessage({ acp: 'nearby' })");
    } finally {
      await page.call("Page.close").catch(() => {});
    }
  };

  await check("nearby: with Canoa running and paired, look nearby is answered by the host, from its own descriptors", async () => {
    const found = await lookNearby();
    const labels = found.candidates.map((candidate) => candidate.label);
    assert.deepEqual(labels, ["Found by the host"], JSON.stringify(found));
    assert.equal(found.candidates[0].peer, PEER);
    assert.equal(found.candidates[0].source, "local");
    assert.equal(found.needsCanoa, undefined);
  });

  const turn = (label) => `(async () => {
    const agent = (await document.acp.agents()).find((a) => a.label === ${quoted(label)});
    const conn = await document.acp.connect(agent);
    const writer = conn.writable.getWriter();
    const reader = conn.readable.getReader();
    const said = [];
    const rpc = async (message) => {
      await writer.write(JSON.stringify(message));
      for (;;) {
        const next = JSON.parse((await reader.read()).value);
        if (next.id === message.id && next.method === undefined) return next;
        said.push(next);
      }
    };
    try {
      await rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } });
      const made = await rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/ignored', mcpServers: [] } });
      if (made.error) return { error: made.error.message };
      const ended = await rpc({ jsonrpc: '2.0', id: 3, method: 'session/prompt', params: { sessionId: made.result.sessionId, prompt: [{ type: 'text', text: 'hello' }] } });
      const text = said.filter((m) => m.params?.update?.sessionUpdate === 'agent_message_chunk').map((m) => m.params.update.content.text).join('');
      return { verified: conn.agent.identity.verified, stop: ended.result.stopReason, text };
    } finally { conn.close?.(); }
  })()`;

  await check("turn: a whole turn runs through the native host to the scripted entry, proven end to end", async () => {
    assert.deepEqual(await own.run(turn("Scripted")), { verified: true, stop: "end_turn", text: "echo: hello" });
  });
  await check("folder: the host made the session's folder, in its own Documents, and the extension named none", async () => {
    const scratch = path.join(work, "documents", "Canoa", "scratch", "canoa-extension");
    assert.ok(existsSync(scratch) && readdirSync(scratch).length >= 1, scratch);
  });
  await check("turn: and to the person's own agent command that has no address", async () => {
    assert.deepEqual(await own.run(turn("standin")), { verified: true, stop: "end_turn", text: "echo: hello" });
  });

  await check("control: the session ledger and settings are the host's, over the door, and owner-only methods are refused", async () => {
    const put = await ask("canoa.host.control", { method: "ledger/put", params: { id: "e2e-1", record: { title: "from the extension" } } });
    assert.ok(!put.error, JSON.stringify(put));
    const listed = await ask("canoa.host.control", { method: "ledger/list", params: {} });
    assert.ok(JSON.stringify(listed).includes("e2e-1"), JSON.stringify(listed));
    const refused = await ask("canoa.host.control", { method: "host/restart", params: {} });
    assert.ok(refused.error, JSON.stringify(refused));
  });

  await check("start: with the host stopped the extension says so, Start brings it up, and the panel connects", async () => {
    const before = JSON.parse(run(["native", "status"])).holder;
    hostProcess.kill();
    await waitFor("the host to end", () => hostProcess.exitCode !== null || hostProcess.signalCode !== null, 15_000);
    const stopped = await ask("canoa.host.status");
    assert.equal(stopped.state, "stopped", JSON.stringify(stopped));
    // With no host running the native host the browser started looks itself.
    const alone = await lookNearby();
    assert.deepEqual(alone.candidates.map((candidate) => candidate.label), ["Found by the relay"], JSON.stringify(alone));
    // Nothing a web page's scripts, or the extension's own scripts in that page, send starts it: only the extension's own pages are heard.
    const tab = await openTab(pageUrl);
    await waitFor("the page", () => tab.run("document.readyState === 'complete'"));
    const tabId = await sw.run(`chrome.tabs.query({}).then((tabs) => tabs.find((tab) => tab.url === ${quoted(pageUrl)})?.id)`);
    const fromScript = await sw.run(`chrome.scripting.executeScript({ target: { tabId: ${tabId} }, world: "ISOLATED",
      func: () => chrome.runtime.sendMessage({ type: "canoa.host.start" }).then((reply) => JSON.stringify(reply ?? "no reply"), (error) => "refused: " + error.message) })
      .then((results) => results[0].result)`);
    assert.ok(!fromScript.includes("running"), fromScript);
    await sleep(1500);
    assert.equal(JSON.parse(run(["native", "status"])).running, false, "the host stayed stopped");
    const started = await ask("canoa.host.start");
    assert.equal(started.state, "running", JSON.stringify(started));
    const after = JSON.parse(run(["native", "status"])).holder;
    assert.notEqual(after.pid, before.pid);
    hostProcess = { kill: () => {
      try { process.kill(after.pid); } catch { /* gone */ }
    }, exitCode: null };
    assert.equal(started.paired, true, "the same home, so the same pairing");
    assert.deepEqual(await own.run(turn("Scripted")), { verified: true, stop: "end_turn", text: "echo: hello" });
  });

  await check("unpair: the host forgets this browser's key, and it is refused until a PIN pairs it again", async () => {
    const listed = () => run(["peers", "--format", "json"]).split("\n").filter(Boolean).map((line) => JSON.parse(line));
    assert.ok(listed().some((peer) => peer.state === "accepted"), "the browser's key is paired before");
    const view = await ask("canoa.host.unpair");
    assert.equal(view.paired, false, JSON.stringify(view));
    assert.equal(view.notice, undefined, JSON.stringify(view));
    assert.ok(listed().every((peer) => peer.state !== "accepted"), JSON.stringify(listed()));
  });

  await check("unregistered: with the native host removed the extension says Canoa is not installed, and an agent added by hand still works", async () => {
    const plain = new WebSocketServer({ port: 0, host: "127.0.0.1" });
    await new Promise((resolve) => plain.on("listening", resolve));
    plain.on("connection", (socket) => socket.on("message", (data) => {
      const { id } = JSON.parse(data.toString());
      socket.send(JSON.stringify({ jsonrpc: "2.0", id, result: { protocolVersion: 1, agentCapabilities: {} } }));
    }));
    run(["native", "uninstall"]);
    registered = false;
    const none = await lookNearby();
    assert.deepEqual([none.candidates, none.needsCanoa], [[], true], JSON.stringify(none));
    const view = await ask("canoa.host.status");
    assert.equal(view.state, "not-installed", JSON.stringify(view));
    await sw.run(`chrome.storage.local.set({
      canoa_acp_agents: [{ key: "hand", label: "By hand", url: "ws://127.0.0.1:${plain.address().port}/agent", source: "manual", offers: [] }],
      canoa_acp_grants: [{ origin: "chrome-extension://${id}", agent: "hand", lifetime: "always", cwd: "/x", permissions: true, mcpOrigins: [], sessions: [] }],
    }).then(() => true)`);
    const said = await own.run(`(async () => {
      const agent = (await document.acp.agents()).find((a) => a.label === "By hand");
      const conn = await document.acp.connect(agent);
      const writer = conn.writable.getWriter();
      const reader = conn.readable.getReader();
      await writer.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: 1 } }));
      const answer = JSON.parse((await reader.read()).value);
      conn.close?.();
      return answer.result.protocolVersion;
    })()`);
    assert.equal(said, 1);
    plain.close();
  });

  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  pageServer.close();
  for (const socket of advertised) socket.close();
  chrome?.kill();
  hostProcess?.kill();
  if (registered) {
    try {
      run(["native", "uninstall"]);
    } catch {
      // Nothing left to remove.
    }
  }
  await sleep(500);
  rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
process.exit(exitCode);
