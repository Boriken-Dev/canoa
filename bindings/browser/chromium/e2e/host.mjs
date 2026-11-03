// The real extension in a real browser reaching an agent that canoa-host serves on this machine, as Canoa's desktop app runs
// it: the person adds the host's entry by hand, the first connection leaves the browser's key waiting, the key is paired on
// the host, and a whole turn then runs through the host to its agent.
//
//   CANOA_E2E_BROWSER=path/to/browser CANOA_E2E_HOST_URL=ws://127.0.0.1:PORT CANOA_E2E_HOST_PEER=sha256:...
//     [CANOA_E2E_ACCEPT=path/to/canoa-host] node e2e/host.mjs
//
// The entry must echo a prompt as `echo: TEXT`, as canoa-host's stdio_agent example does. With CANOA_E2E_ACCEPT the key is
// paired by `canoa-host accept`, under this environment's COQUI_AGENTIC_PEER_HOME; without it the run waits up to five minutes
// for a person to pair it in Canoa's Hosting dialog, comparing the id this prints.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const browser = process.env.CANOA_E2E_BROWSER;
const hostUrl = process.env.CANOA_E2E_HOST_URL;
const hostPeer = process.env.CANOA_E2E_HOST_PEER;
const accept = process.env.CANOA_E2E_ACCEPT;
if (!browser || !hostUrl || !hostPeer) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST_URL and CANOA_E2E_HOST_PEER");

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-host-"));
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
// What a person's "let this site ask" grants, which needs the browser's own prompt otherwise.
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
manifest.host_permissions = ["http://127.0.0.1/*"];
delete manifest.side_panel;
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const CSP = "default-src 'none'; script-src 'self'; connect-src 'self'";
const site = createServer((request, response) => {
  if (request.url === "/app.js") {
    response.writeHead(200, { "content-type": "text/javascript", "content-security-policy": CSP });
    response.end("window.ready = true;");
    return;
  }
  response.writeHead(200, { "content-type": "text/html", "content-security-policy": CSP });
  response.end('<!doctype html><title>a page</title><script src="/app.js"></script><p>page</p>');
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const pageUrl = `http://127.0.0.1:${site.address().port}/`;

const chrome = spawn(browser, [
  "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
  "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
], { stdio: "ignore" });

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
const step = (text) => console.log(`ok   ${text}`);

let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome.kill();
  process.exit(1);
}, 420_000);
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
  await waitFor("the extension's APIs", async () => (await sw.run("typeof chrome")) === "object");
  const origin = new URL(pageUrl).origin;
  await sw.run(`chrome.scripting.registerContentScripts([
    { id: "m", matches: ["${origin}/*"], js: ["acp-page.js"], runAt: "document_start", world: "MAIN" },
    { id: "i", matches: ["${origin}/*"], js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED" },
  ]).then(() => true)`);
  const openTab = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
    const session = new Session(made.webSocketDebuggerUrl);
    await session.call("Runtime.enable");
    return session;
  };

  // The browser's own id, as its permissions page shows it to the person.
  const permissions = await openTab(`chrome-extension://${new URL(worker.url).hostname}/permissions.html`);
  const shown = await waitFor("the browser's id", () => permissions.run("document.querySelector('#standing-command')?.textContent ?? ''"));
  const browserId = shown.split(" ").at(-1);
  assert.match(browserId, /^sha256:[0-9a-f]{64}$/);
  step(`the browser's key is ${browserId}`);

  const page = await openTab(pageUrl);
  await waitFor("the provider", () => page.run("window.ready === true && typeof document.acp === 'object'"));
  await page.run(`window.asked = document.acp.request({ purpose: "testing" }).then(
      (agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
  const target = await waitFor("the person's window", async () => (await targets()).find((item) => item.url.includes("acp-request.html?ask=")));
  const view = new Session(target.webSocketDebuggerUrl);
  await waitFor("its contents", () => view.run("(document.querySelector('#main')?.childElementCount ?? 0) > 0"));
  await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button')].some((button) => button.disabled)"));
  await view.run(`(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      inputs[0].value = 'Canoa host'; inputs[1].value = ${JSON.stringify(hostUrl)}; inputs[2].value = ${JSON.stringify(hostPeer)};
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Add this agent').click();
    })()`);
  await waitFor("the added agent", () => view.run("document.querySelectorAll('input[type=checkbox]:checked').length >= 1"));
  await view.run(`(() => {
      const inputs = [...document.querySelectorAll('input[type=text]')];
      inputs.at(-1).value = ${JSON.stringify(tmpdir())};
      [...document.querySelectorAll('button')].find((b) => b.textContent === 'Allow').click();
    })()`).catch(() => {});
  assert.equal(await page.run("window.asked"), 1);
  step("the person added the host's entry by hand, pinned to the host's id");

  const turn = `(async () => {
    const conn = await document.acp.connect(window.agent);
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
      const init = await rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } });
      const made = await rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/x', mcpServers: [] } });
      const ended = await rpc({ jsonrpc: '2.0', id: 3, method: 'session/prompt', params: { sessionId: made.result.sessionId, prompt: [{ type: 'text', text: 'hello' }] } });
      const text = said.filter((m) => m.params?.update?.sessionUpdate === 'agent_message_chunk').map((m) => m.params.update.content.text).join('');
      return { verified: conn.agent.identity.verified, version: init.result.protocolVersion, stop: ended.result.stopReason, text };
    } finally {
      conn.close?.();
    }
  })()`;
  const refused = await page.run(`${turn}.then(() => 'served', (e) => e.name ?? String(e))`);
  assert.notEqual(refused, "served", "an unpaired key is served");
  step(`an unpaired key is turned away (${refused}), and waits on the host`);

  if (accept) {
    execFileSync(accept, ["accept", browserId], { encoding: "utf8" });
    step("paired by canoa-host accept");
  } else {
    console.log(`waiting for a person to pair ${browserId} in Canoa's Hosting dialog`);
  }
  const served = await waitFor("the key to be paired", async () => {
    try {
      return await page.run(turn);
    } catch {
      await sleep(2000);
      return null;
    }
  }, 300_000);
  assert.deepEqual(served, { verified: true, version: 1, stop: "end_turn", text: "echo: hello" });
  step("a whole turn ran through the host to its agent: echo: hello");
  exitCode = 0;
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  chrome.kill();
  site.close();
  await sleep(500);
  rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
process.exit(exitCode);
