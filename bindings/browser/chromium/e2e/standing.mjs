// A program with no ACP session reaching the browser's tools, in a real browser through the real bridge, from what `mcp-config` prints.
// The client is the official SDK's stdio client, unmodified. The person's window is clicked by script.
//
//   CANOA_E2E_BROWSER=path/to/edge CANOA_E2E_BRIDGE=path/to/coqui-acp-bridge node e2e/standing.mjs
//
// Needs `npm install` in e2e/. CANOA_E2E_TRANSPORTS picks websocket, webtransport or both.
//
// CANOA_E2E_IDLE_MS leaves it alone that long with nothing attached to the worker and no extension page open, then asks whether the
// link is the one it started with: DevTools attached to a worker keeps it alive, so this attaches to none. CANOA_E2E_KEEPALIVE=heartbeat,
// awake, alarm or none leaves only that one in a copy of the extension, to measure what each does. CANOA_E2E_IDLE_TABS=none closes the
// last tab too.
//
// CANOA_E2E_REAL_AGENT=claude-code and CLAUDE_CODE_EXECUTABLE run a real Claude Code turn from the same entry, in a temporary directory
// with no tools of its own, printing the shape of its events and never their text. It spends a little of the person's subscription.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createSocket } from "node:dgram";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { getDefaultEnvironment, StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { LoggingMessageNotificationSchema, ToolListChangedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";

import { lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const browser = process.env.CANOA_E2E_BROWSER;
const bridge = process.env.CANOA_E2E_BRIDGE;
if (!browser || !bridge) throw new Error("set CANOA_E2E_BROWSER and CANOA_E2E_BRIDGE");
const idleMs = Number(process.env.CANOA_E2E_IDLE_MS ?? 0);
const keepalive = process.env.CANOA_E2E_KEEPALIVE ?? "all";
const transports = (process.env.CANOA_E2E_TRANSPORTS ?? "websocket,webtransport").split(",");
const realAgent = process.env.CANOA_E2E_REAL_AGENT === "claude-code" ? process.env.CLAUDE_CODE_EXECUTABLE : undefined;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function waitFor(what, test, ms = 20_000) {
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
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 400)}`);
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

const CSP = "default-src 'none'; script-src 'self'; connect-src 'self'";
const site = createServer((request, response) => {
  if (request.url === "/app.js") {
    response.writeHead(200, { "content-type": "text/javascript", "content-security-policy": CSP });
    response.end("window.ready = true;");
    return;
  }
  response.writeHead(200, { "content-type": "text/html", "content-security-policy": CSP });
  response.end('<!doctype html><title>strict page</title><script src="/app.js"></script><p>page</p>');
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const pageUrl = `http://127.0.0.1:${site.address().port}/`;
const siteOrigin = new URL(pageUrl).origin;

/** The extension as a person's would be, but for the site's host permission, whose prompt is Chrome's own. */
function unpack(to) {
  cpSync(extension, to, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
  const manifest = JSON.parse(readFileSync(path.join(to, "manifest.json"), "utf8"));
  manifest.host_permissions = ["http://127.0.0.1/*"];
  delete manifest.side_panel;
  writeFileSync(path.join(to, "manifest.json"), JSON.stringify(manifest));
  const provider = path.join(to, "acp-provider.js");
  let code = readFileSync(provider, "utf8");
  if (keepalive !== "all" && keepalive !== "heartbeat") {
    assert.ok(code.includes("const HEARTBEAT = {"));
    code = code.replace(/const HEARTBEAT = \{[^}]*\};/, "const HEARTBEAT = undefined;");
  }
  if (keepalive !== "all" && keepalive !== "alarm") {
    code = code.replaceAll("chrome.alarms.create(STANDING_ALARM, { periodInMinutes: ALARM_MINUTES })", "Promise.resolve()");
  }
  if (keepalive !== "all" && keepalive !== "awake") {
    assert.ok(code.includes("stayAwake(status.state"));
    code = code.replace('stayAwake(status.state === "connected");', "");
  }
  writeFileSync(provider, code);
}

class Browser {
  constructor(profile, unpacked, env) {
    this.profile = profile;
    this.unpacked = unpacked;
    this.env = env;
  }
  async start() {
    rmSync(path.join(this.profile, "DevToolsActivePort"), { force: true });
    this.child = spawn(browser, [
      "--headless=new", `--user-data-dir=${this.profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
      `--load-extension=${this.unpacked}`, `--disable-extensions-except=${this.unpacked}`,
      "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
    ], { stdio: "ignore", env: this.env });
    this.debug = await waitFor("the browser", () => {
      try {
        return readFileSync(path.join(this.profile, "DevToolsActivePort"), "utf8").split("\n")[0];
      } catch {
        return null;
      }
    }, 30_000);
  }
  targets = async () => (await fetch(`http://127.0.0.1:${this.debug}/json/list`)).json();
  worker = async () => (await this.targets()).find((target) => target.type === "service_worker" && target.url.endsWith("/background.js"));
  async open(url) {
    const made = await (await fetch(`http://127.0.0.1:${this.debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
    const session = new Session(made.webSocketDebuggerUrl);
    session.id = made.id;
    await session.call("Runtime.enable");
    return session;
  }
  async closeTarget(session) {
    session.close();
    await fetch(`http://127.0.0.1:${this.debug}/json/close/${session.id}`).catch(() => {});
  }
  /** The way a person closes it, with what it keeps for next time written out. */
  async quit() {
    const version = await (await fetch(`http://127.0.0.1:${this.debug}/json/version`)).json();
    const session = new Session(version.webSocketDebuggerUrl);
    await session.call("Browser.close").catch(() => {});
    await waitFor("the browser to go", () => this.child.exitCode !== null || this.child.signalCode !== null, 30_000);
    await sleep(1000);
  }
  kill() {
    this.child?.kill();
  }
}

async function forTransport(transport) {
  console.log(`--- ${transport}, keepalive ${keepalive}${idleMs ? `, idle ${idleMs} ms` : ""}`);
  const work = mkdtempSync(path.join(tmpdir(), "canoa-standing-"));
  const unpacked = path.join(work, "extension");
  const descriptors = path.join(work, "agents");
  mkdirSync(descriptors);
  unpack(unpacked);
  const env = { ...process.env, COQUI_ACP_DESCRIPTORS: descriptors, COQUI_AGENTIC_PEER_HOME: path.join(work, "bridge-home") };
  const edge = new Browser(path.join(work, "profile"), unpacked, env);
  const bridges = [];
  const clients = [];
  try {
    await edge.start();
    const worker = await waitFor("the extension's service worker", () => edge.worker(), 20_000);
    const extensionId = new URL(worker.url).hostname;
    const page = (name) => `chrome-extension://${extensionId}/${name}`;
    // Opened is not loaded: until it has, the target is not yet the extension's.
    const openExtensionPage = async (name) => {
      const session = await edge.open(page(name));
      await waitFor(name, () => session.run("typeof chrome?.runtime?.sendMessage === 'function'"));
      return session;
    };

    // Asks from the permissions page, and shuts it, so nothing of the extension stays open.
    const fromPermissions = async (run) => {
      const options = await openExtensionPage("permissions.html");
      await waitFor("the options page", () => options.run("document.readyState === 'complete' && document.querySelector('#standing-command')?.textContent.length > 0"));
      try {
        return await run(options);
      } finally {
        await edge.closeTarget(options);
      }
    };
    const ask = (session, message) => session.run(`chrome.runtime.sendMessage(${JSON.stringify(message)})`);
    const standingView = () => fromPermissions((options) => ask(options, { type: "canoa.standing.get" }));
    const first = await standingView();
    const browserPeer = first.browser;

    const agentKey = path.join(work, "bridge.key");
    const bridgePeer = execFileSync(bridge, ["keygen", agentKey], { encoding: "utf8", env }).trim();
    const port = await freeForBoth();
    const flags = transport === "webtransport" ? ["--tls-short-lived", "--webtransport-port", String(port)] : [];
    let said = "";
    const startBridge = async () => {
      const child = spawn(bridge, ["serve", "--listen", `127.0.0.1:${port}`, "--identity", agentKey, "--allow", browserPeer, ...flags, "--mcp-offers",
        "--advertise", "Test machine", "--", process.execPath, "--version"], { stdio: ["ignore", "ignore", "pipe"], env });
      child.stderr.on("data", (chunk) => {
        said += chunk;
      });
      bridges.push(child);
      await waitFor("the bridge to listen", () => said.includes("listening on"));
    };
    await startBridge();
    const linksMade = () => said.split("\n").filter((line) => line === `${browserPeer} proven`).length;

    const output = execFileSync(bridge, ["mcp-config", "--peer", browserPeer], { encoding: "utf8", env });
    const entry = JSON.parse(output).mcpServers["canoa-browser"];

    const startClient = async () => {
      const client = new Client({ name: "canoa-e2e", version: "1" }, { capabilities: {} });
      const seen = { listChanged: 0, logs: [], closed: false, stderr: "" };
      client.setNotificationHandler(ToolListChangedNotificationSchema, () => {
        seen.listChanged += 1;
      });
      client.setNotificationHandler(LoggingMessageNotificationSchema, (notification) => {
        seen.logs.push(notification.params.data);
      });
      client.onclose = () => {
        seen.closed = true;
      };
      const transportToShim = new StdioClientTransport({ command: entry.command, args: entry.args, env: { ...getDefaultEnvironment(), ...entry.env }, stderr: "pipe" });
      transportToShim.stderr?.on("data", (chunk) => {
        seen.stderr += chunk;
      });
      await client.connect(transportToShim);
      const held = { client, seen };
      clients.push(held);
      return held;
    };
    const toolNames = async (client) => (await client.listTools()).tools.map((tool) => tool.name);

    await check(`${transport}: with nothing on, the client hears why, off by default`, async () => {
      assert.equal(first.on, null);
      assert.equal(first.status.state, "off");
      await assert.rejects(startClient(), (error) => error.message.includes("no browser is connected"));
      assert.equal(linksMade(), 0, "no link until a person says so");
    });

    const found = execFileSync(bridge, ["discover", "--seconds", "0"], { encoding: "utf8", env }).trim().split("\n").map((line) => JSON.parse(line));
    const record = found.find((item) => item.peer === bridgePeer);
    assert.ok(record?.offers?.includes("mcp-offer/1"), JSON.stringify(record));
    const manage = await openExtensionPage("acp-request.html?manage=1");
    const added = await manage.run(`chrome.runtime.sendMessage({ acp: "add", agent: ${JSON.stringify({
      label: "Test machine",
      url: record.url,
      peer: record.peer,
      source: "local",
      offers: record.offers,
      webtransport: record.webtransport ? { url: `https://127.0.0.1:${record.webtransport}/`, hash: record.webtransportHash } : undefined,
    })} })`);
    await edge.closeTarget(manage);
    const turnOn = () => fromPermissions(async (options) => {
      await options.run(`(() => {
        document.querySelector('#machine').value = ${JSON.stringify(added.key)};
        document.querySelector('#standing-toggle').click();
      })()`);
      await waitFor("that machine", async () => (await ask(options, { type: "canoa.standing.get" })).status.state === "connected");
      await waitFor("the page to say so", () => options.run("document.querySelector('#standing-status').textContent.startsWith('On.')"));
    });
    const turnOff = () => fromPermissions(async (options) => {
      await waitFor("the button", () => options.run("document.querySelector('#standing-toggle').textContent === 'Turn off'"));
      await options.run("document.querySelector('#standing-toggle').click()");
      await waitFor("off", async () => (await ask(options, { type: "canoa.standing.get" })).on === null);
    });
    // Nothing yet has asked for a link.
    await sleep(1500);

    // What a person's "let this site ask" registers, so the page has the extension's own scripts beside it.
    await fromPermissions((options) => options.run(`chrome.scripting.registerContentScripts([
      { id: "m", matches: [${JSON.stringify(`${siteOrigin}/*`)}], js: ["acp-page.js"], runAt: "document_start", world: "MAIN" },
      { id: "i", matches: [${JSON.stringify(`${siteOrigin}/*`)}], js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED" },
    ]).then(() => true)`));
    const siteTab = await edge.open(pageUrl);
    await waitFor("the site", () => siteTab.run("window.ready === true && typeof document.acp === 'object'"));

    const answeredWindows = new Set();
    const prompt = async () => {
      const target = await waitFor("the person's window", async () => (await edge.targets()).find((item) => item.url.includes("acp-request.html?ask=") && !answeredWindows.has(item.id)));
      answeredWindows.add(target.id);
      const view = new Session(target.webSocketDebuggerUrl);
      await waitFor("its contents", () => view.run("(document.querySelector('#main')?.childElementCount ?? 0) > 0"));
      await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button')].some((button) => button.disabled)"));
      return view;
    };
    const allowSite = (view) => view.run(`(() => {
      [...document.querySelectorAll('input[name=tab]')].find((input) => input.dataset.origin === ${JSON.stringify(siteOrigin)}).click();
      const tabId = Number(document.querySelector('input[name=tab]:checked').value);
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow this tab').click();
      return tabId;
    })()`, { gesture: true }).catch(() => undefined);
    const askAndAllow = async (client) => {
      const asking = client.callTool({ name: "browser_request_access", arguments: { capability: "browser.read", reason: "I would like to read the page" } });
      const view = await prompt();
      const shown = await view.run("document.querySelector('#main').textContent");
      await allowSite(view);
      const answer = await asking;
      assert.equal(answer.isError, false, JSON.stringify(answer));
      return { tabId: JSON.parse(answer.content[0].text).tabId, shown };
    };

    await check(`${transport}: a page, a content script and another page of the extension cannot turn it on`, async () => {
      assert.equal(await siteTab.run("typeof chrome === 'undefined' || typeof chrome.runtime?.sendMessage === 'undefined'"), true, "a page's world has no messaging");
      // The world the extension's own scripts run in, which is as far as a page's renderer goes.
      const world = await waitFor("the content script's world", () => siteTab.contexts.find((context) => context.auxData?.type === "isolated" && context.name === "Canoa"));
      const context = world.id;
      const asked = await siteTab.run(`chrome.runtime.sendMessage({ type: "canoa.standing.set", agent: ${JSON.stringify(added.key)} }).then((answer) => answer ?? null, (error) => String(error.message))`, { context });
      assert.ok(asked === null || typeof asked === "string", JSON.stringify(asked));
      const stored = await siteTab.run(`chrome.storage.local.set({ canoa_standing: ${JSON.stringify(added.key)} }).then(() => "stored", (error) => String(error.message))`, { context });
      assert.notEqual(stored, "stored", "storage is for the extension's own pages");
      const read = await siteTab.run(`chrome.storage.local.get(null).then((all) => Object.keys(all).length, (error) => String(error.message))`, { context });
      assert.equal(typeof read, "string", "a content script reads nothing there");
      const viaPort = await siteTab.run(`new Promise((resolve) => {
        const port = chrome.runtime.connect({ name: "canoa-acp" });
        port.onMessage.addListener(resolve);
        port.onDisconnect.addListener(() => resolve("disconnected"));
        port.postMessage({ id: 1, op: "setStanding", args: { agent: ${JSON.stringify(added.key)} } });
      })`, { context });
      assert.equal(viaPort.error?.name ?? viaPort, "NotAllowedError");
      const other = await openExtensionPage("acp-request.html?manage=1");
      assert.equal(await ask(other, { type: "canoa.standing.set", agent: added.key }) ?? null, null);
      await edge.closeTarget(other);
      await sleep(500);
      assert.equal((await standingView()).on, null);
      assert.equal(linksMade(), 0, "no link came of any of it");
    });

    await check(`${transport}: on for one machine, it opens a link with no ACP on it, and what that machine held before goes`, async () => {
      await fromPermissions(async (options) => {
        const siteTabId = await options.run(`chrome.tabs.query({}).then((tabs) => tabs.find((tab) => tab.url === ${JSON.stringify(pageUrl)}).id)`);
        const given = await ask(options, { type: "canoa.grants.add", peerId: bridgePeer, tabId: siteTabId, capability: "browser.read", minutes: 15 });
        assert.equal(given.grants.length, 1, JSON.stringify(given));
      });
      await turnOn();
      assert.equal(linksMade(), 1);
      assert.ok(!said.includes("acp/1"), "no agent ran");
      assert.deepEqual((await fromPermissions((options) => ask(options, { type: "canoa.grants.list" }))).grants, []);
    });

    const doorFile = path.join(env.COQUI_AGENTIC_PEER_HOME, "offers", `standing-${browserPeer.slice("sha256:".length)}.json`);
    let clientA;
    let tabId;
    await check(`${transport}: the client started from the printed entry sees only the asking tool, and calling another is refused`, async () => {
      clientA = await startClient();
      assert.deepEqual(await toolNames(clientA.client), ["browser_request_access"]);
      const denied = await clientA.client.callTool({ name: "browser_read_page", arguments: { tabId: 1 } });
      assert.equal(denied.isError, true);
    });
    await check(`${transport}: a person allows a tab in the window naming the machine, the list grows, and it reads that page`, async () => {
      const before = clientA.seen.listChanged;
      const allowed = await askAndAllow(clientA.client);
      tabId = allowed.tabId;
      assert.ok(allowed.shown.includes("A program on Test machine"), allowed.shown);
      assert.ok(allowed.shown.includes("In its own words: I would like to read the page"), allowed.shown);
      assert.ok(allowed.shown.includes(bridgePeer), allowed.shown);
      assert.ok(allowed.shown.includes(siteOrigin), allowed.shown);
      await waitFor("the list-changed notice", () => clientA.seen.listChanged > before);
      assert.ok((await toolNames(clientA.client)).includes("browser_read_page"));
      const read = await clientA.client.callTool({ name: "browser_read_page", arguments: { tabId } });
      assert.equal(read.isError, false, JSON.stringify(read));
      assert.equal(read.structuredContent.result.title, "strict page");
    });
    await check(`${transport}: a second program at once has a stream of its own, under the one grant`, async () => {
      const clientB = await startClient();
      assert.ok((await toolNames(clientB.client)).includes("browser_read_page"), "the one peer's grant");
      const read = await clientB.client.callTool({ name: "browser_read_page", arguments: { tabId } });
      assert.equal(read.structuredContent.result.title, "strict page");
      assert.equal((await clientA.client.ping()) !== undefined, true);
      await clientB.client.close();
      assert.equal(linksMade(), 1, "on the one link");
      // One still stands for the next, on the same link.
      const clientC = await startClient();
      assert.ok((await toolNames(clientC.client)).includes("browser_read_page"));
      await clientC.client.close();
    });
    await check(`${transport}: turning it off ends access and tells the client, and the grant goes with it`, async () => {
      await turnOff();
      await waitFor("the client to hear", () => clientA.seen.closed);
      assert.ok(clientA.seen.logs.some((text) => text.includes("turned this off")), JSON.stringify(clientA.seen.logs));
      await waitFor("the shim to say so", () => clientA.seen.stderr.includes("ended this server"));
      await assert.rejects(clientA.client.listTools());
      const { grants } = await fromPermissions((options) => ask(options, { type: "canoa.grants.list" }));
      assert.deepEqual(grants, []);
      await waitFor("the door to go", () => !existsSync(doorFile));
      await assert.rejects(startClient(), (error) => error.message.includes("no browser is connected"));
      assert.equal((await standingView()).status.state, "off");
    });
    await check(`${transport}: on again, a browser restart needs no change to the entry`, async () => {
      await turnOn();
      const before = linksMade();
      await siteTab.close();
      await edge.quit();
      const quitAt = Date.now();
      await waitFor("the bridge to hear the browser went", () => !existsSync(doorFile), 40_000);
      console.log(`     the door went ${Date.now() - quitAt} ms after the browser quit`);
      await assert.rejects(startClient(), (error) => error.message.includes("no browser is connected"));
      await edge.start();
      const again = await waitFor("the link back with no page open", async () => linksMade() > before, 40_000);
      assert.ok(again);
      const output2 = execFileSync(bridge, ["mcp-config", "--peer", browserPeer], { encoding: "utf8", env });
      assert.equal(output2, output, "the same entry, byte for byte");
      const client = (await waitFor("the client to get in", () => startClient().catch(() => null), 20_000));
      assert.deepEqual(await toolNames(client.client), ["browser_request_access"], "no grant outlives the browser");
      await client.client.close();
    });

    if (idleMs > 0) {
      await check(`${transport}: left idle ${Math.round(idleMs / 1000)} s with no tab and nothing attached (keepalive ${keepalive})`, async () => {
        const before = linksMade();
        const { client, seen } = await startClient();
        const idleSince = Date.now();
        // Nothing but the blank tab, and nothing attached to it either.
        for (const target of await edge.targets()) {
          if (target.type === "page" && (target.url !== "about:blank" || process.env.CANOA_E2E_IDLE_TABS === "none")) await fetch(`http://127.0.0.1:${edge.debug}/json/close/${target.id}`).catch(() => {});
        }
        let lastAlive = null;
        const checkpoints = [];
        while (Date.now() - idleSince < idleMs) {
          await sleep(Math.min(15_000, idleMs));
          const alive = await edge.worker().then((worker) => worker !== undefined, () => "browser gone");
          if (alive === "browser gone") {
            checkpoints.push(`${Math.round((Date.now() - idleSince) / 1000)}s the browser exited, client ${seen.closed ? "closed" : "open"}, links ${linksMade()}`);
            break;
          }
          if (alive !== lastAlive) {
            checkpoints.push(`${Math.round((Date.now() - idleSince) / 1000)}s worker ${alive ? "there" : "gone"}, client ${seen.closed ? "closed" : "open"}, links ${linksMade()}`);
            lastAlive = alive;
          }
          if (seen.closed && !checkpoints.at(-1).includes("closed")) checkpoints.push(`${Math.round((Date.now() - idleSince) / 1000)}s client closed, links ${linksMade()}`);
        }
        console.log(`     idle: ${checkpoints.join("; ")}; at the end client closed ${seen.closed}, links ${linksMade()} (were ${before})`);
        assert.equal(seen.closed, false, "the client's server ended");
        assert.ok((await client.ping()) !== undefined);
        assert.deepEqual(await toolNames(client), ["browser_request_access"]);
        assert.equal(linksMade(), before, "the same link all along");
        await client.close();
      });
    }

    if (realAgent && transport === transports[0]) {
      await check("real agent: Claude Code from the printed entry asks for a tab, a person allows it, and it reads the page", async () => {
        const site2 = await edge.open(pageUrl);
        await waitFor("the site", () => site2.run("window.ready === true"));
        const cwd = path.join(work, "real-agent");
        mkdirSync(cwd);
        const config = path.join(cwd, "mcp.json");
        writeFileSync(config, output);
        const task = "Use the browser_request_access tool with capability browser.read and any short reason. When it answers with a tabId, use the browser_read_page tool on that tabId, then reply with only the page's title. Do nothing else.";
        const child = spawn(realAgent, ["-p", task, "--output-format", "stream-json", "--verbose", "--mcp-config", config, "--strict-mcp-config", "--tools", "", "--setting-sources", "",
          "--allowedTools", "mcp__canoa-browser__browser_request_access", "mcp__canoa-browser__browser_read_page", "--model", "haiku", "--no-session-persistence", "--disable-slash-commands"],
        { cwd, stdio: ["ignore", "pipe", "pipe"], env: Object.fromEntries(Object.entries(process.env).filter(([name]) => name !== "CLAUDECODE")) });
        let complaint = "";
        child.stderr.on("data", (chunk) => {
          complaint = (complaint + chunk).slice(0, 300);
        });
        const shapes = [];
        let reply = "";
        let buffer = "";
        const tools = [];
        child.stdout.on("data", (chunk) => {
          buffer += chunk;
          const lines = buffer.split("\n");
          buffer = lines.pop();
          for (const line of lines) {
            let event;
            try {
              event = JSON.parse(line);
            } catch {
              shapes.push("not json");
              continue;
            }
            shapes.push([event.type, event.subtype].filter(Boolean).join("/"));
            if (event.type === "system" && event.subtype === "init") {
              console.log(`     real agent init: servers ${JSON.stringify(event.mcp_servers?.map(({ name, status }) => [name, status]))}, tools from it ${JSON.stringify(event.tools?.filter((tool) => tool.includes("canoa-browser")))}`);
            }
            for (const part of event.message?.content ?? []) {
              if (part.type === "tool_use") tools.push(part.name);
            }
            if (event.type === "result") reply = String(event.result ?? "");
          }
        });
        const ended = new Promise((resolve) => {
          child.on("exit", resolve);
          child.on("error", (error) => resolve(`could not start: ${error.code}`));
        });
        const view = await waitFor("Claude Code to ask", async () => (child.exitCode === null ? (await edge.targets()).find((item) => item.url.includes("acp-request.html?ask=") && !answeredWindows.has(item.id)) : "ended"), 180_000);
        if (view !== "ended") await allowSite(await prompt());
        const status = await Promise.race([ended, sleep(180_000).then(() => "timed out")]);
        if (status === "timed out") child.kill();
        console.log(`     real agent: exit ${status}, events ${JSON.stringify(shapes.reduce((counts, shape) => ({ ...counts, [shape]: (counts[shape] ?? 0) + 1 }), {}))}, tools used ${JSON.stringify(tools)}, reply has the title: ${reply.includes("strict page")}`);
        assert.equal(status, 0, complaint);
        assert.ok(tools.includes("mcp__canoa-browser__browser_request_access"));
        assert.ok(tools.includes("mcp__canoa-browser__browser_read_page"));
        assert.ok(reply.includes("strict page"));
      });
    }
  } finally {
    for (const { client } of clients) await client.close().catch(() => {});
    for (const child of bridges) child.kill();
    edge.kill();
    await sleep(1500);
    try {
      rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
    } catch {
      // The browser may still hold its profile for a moment.
    }
  }
}

const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  process.exit(1);
}, 300_000 + 2 * transports.length * idleMs + (realAgent ? 400_000 : 0));
let exitCode = 1;
try {
  for (const transport of transports) await forTransport(transport);
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`ABORTED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  site.close();
}
console.log(`${results.filter((line) => line.startsWith("ok")).length} ok, ${results.filter((line) => line.startsWith("FAIL")).length} failed`);
console.log(exitCode === 0 ? "PASS" : "FAILED");
process.exit(exitCode);
