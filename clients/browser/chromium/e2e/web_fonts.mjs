// The web app's start under `canoa serve --web`: no service worker is registered or waited for, no request leaves the site's
// own origin, and glyphs Roboto lacks (symbols, an arrow, an emoji, CJK) are drawn from the fallback fonts the site carries,
// with nothing refused by the Content-Security-Policy and no fetch failing. The agent entry's name holds those glyphs, so the
// app draws them in its list.
//
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent \
//   CANOA_E2E_WEB=app/build/web [CANOA_E2E_SHOT=<png>] node e2e/web_fonts.mjs
import { spawn } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_HOST: hostProgram, CANOA_E2E_AGENT: agent, CANOA_E2E_WEB: web, CANOA_E2E_SHOT: shot } = process.env;
if (!browser || !hostProgram || !agent || !web) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST, CANOA_E2E_AGENT and CANOA_E2E_WEB");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-fonts-e2e-"));
const site = path.join(work, "site");
cpSync(web, site, { recursive: true });
const NAME = "echo";
const GLYPHS = "Symbols ★ ✓ ☂  arrows → ⇒  math ∑ ∫ ∞  emoji \u{1F600} \u{1F680}  CJK 日本語 中文 한국어  مرحبا שלום";
writeFileSync(path.join(site, "legacy_worker.js"), 'self.addEventListener("fetch", () => {});\n');
writeFileSync(path.join(work, "canoa.cfg"), [
  `[host.agents.${JSON.stringify(NAME)}]`, `program = ${JSON.stringify(agent)}`,
  "[host.web]", 'dir = "site"', 'listen = "127.0.0.1:0"', "",
].join("\n"));

const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: path.join(work, "home"), CANOA_LOG: "info" };
const host = spawn(hostProgram, ["serve", "--config", path.join(work, "canoa.cfg"), "--until-stdin-closes"],
  { env, stdio: ["pipe", "pipe", "inherit"] });
const said = readline.createInterface({ input: host.stdout })[Symbol.asyncIterator]();

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 800)}`);
  }
  console.log(results.at(-1));
}
async function waitFor(what, test, ms = 60_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}
const RECORD = `window.__refused = [];
  document.addEventListener("securitypolicyviolation", (event) => window.__refused.push(event.violatedDirective + " " + event.blockedURI));`;

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  host.kill();
  process.exit(1);
}, 300_000);
try {
  let webUrl;
  while (!webUrl) {
    const line = (await said.next()).value;
    if (line === undefined) throw new Error("the host ended before it named the site");
    webUrl = JSON.parse(line).web;
  }
  const origin = new URL(webUrl).origin;
  const profile = path.join(work, "profile");
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--window-size=1000,700", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  });

  const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
  const page = new Session(made.webSocketDebuggerUrl);
  const requests = new Map();
  const failed = [];
  const entries = [];
  page.socket.addEventListener("message", (event) => {
    const { method, params } = JSON.parse(event.data);
    if (method === "Network.requestWillBeSent") requests.set(params.requestId, { url: params.request.url });
    if (method === "Network.responseReceived") Object.assign(requests.get(params.requestId) ?? {}, { status: params.response.status });
    if (method === "Network.loadingFailed") failed.push(`${requests.get(params.requestId)?.url ?? params.requestId} ${params.errorText}`);
    if (method === "Log.entryAdded") entries.push(`${params.entry.level}: ${params.entry.text} ${params.entry.url ?? ""}`);
  });
  await page.call("Page.enable");
  await page.call("Network.enable");
  await page.call("Log.enable");
  await page.call("Runtime.enable");
  if (process.env.CANOA_E2E_LATENCY_MS) {
    await page.call("Network.emulateNetworkConditions", { offline: false, latency: Number(process.env.CANOA_E2E_LATENCY_MS), downloadThroughput: -1, uploadThroughput: -1 });
  }
  await page.call("Page.addScriptToEvaluateOnNewDocument", { source: RECORD });
  // A browser that visited an earlier build holds a worker of that build's: Flutter's loader then registers the build's own
  // worker and waits for it. The first visit leaves such a worker behind, and the second is the one measured.
  await page.call("Page.navigate", { url: `${webUrl}/` });
  await waitFor("Flutter's view", () => page.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
  await page.run(`navigator.serviceWorker.register("legacy_worker.js").then((r) => new Promise((resolve) => {
    const w = r.installing || r.waiting || r.active;
    if (w.state === "activated") return resolve();
    w.addEventListener("statechange", () => w.state === "activated" && resolve());
  }))`);
  page.logs.length = 0;
  entries.length = 0;
  requests.clear();
  const started = Date.now();
  await page.call("Page.navigate", { url: `${webUrl}/` });
  await waitFor("Flutter's view", () => page.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
  const startMs = Date.now() - started;
  console.log(`the app's view appeared ${startMs} ms after the navigation`);
  // The fallback fonts are fetched once the first frame draws text the bundled fonts lack.
  const click = async (x, y) => {
    for (const type of ["mousePressed", "mouseReleased"]) await page.call("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
  };
  await waitFor("the first frame", () => page.run('!document.getElementById("splash")').catch(() => false));
  await sleep(1000);
  await click(353, 418);
  await sleep(2000);
  // The composer is Flutter's own text field: text put into it is drawn on the canvas with whatever fonts hold its glyphs.
  await click(600, 360);
  await sleep(500);
  await page.call("Input.insertText", { text: GLYPHS });
  await sleep(1500);
  await sleep(3000);
  const messages = [...page.logs, ...entries];
  if (shot) writeFileSync(shot, Buffer.from((await page.call("Page.captureScreenshot", { format: "png" })).data, "base64"));

  await check("the app starts with no service worker registered or waited for", async () => {
    const worker = messages.filter((line) => /service ?worker/i.test(line));
    if (worker.length) throw new Error(worker.join("; "));
    const registered = await page.run("navigator.serviceWorker.getRegistrations().then((all) => all.length)");
    if (registered !== 0) throw new Error(`${registered} service worker left registered`);
  });
  await check("nothing is requested from outside the site's own origin", async () => {
    const outside = [...requests.values()].map((r) => r.url).filter((url) => !url.startsWith(origin) && !url.startsWith("data:") && !url.startsWith("blob:"));
    if (outside.length) throw new Error(outside.join("; "));
  });
  await check("the fallback fonts are fetched from the site and none fails", async () => {
    const fonts = [...requests.values()].filter((r) => r.url.includes("/assets/fonts/fallback/"));
    if (fonts.length < 2) throw new Error(`only ${fonts.length} fallback fonts were asked for`);
    const bad = fonts.filter((r) => r.status !== 200);
    if (bad.length) throw new Error(bad.map((r) => `${r.url} ${r.status}`).join("; "));
    const warnings = messages.filter((line) => /Failed to load font|failed to complete HTTP request/.test(line));
    if (warnings.length) throw new Error(warnings.join("; "));
    console.log(`fallback fonts fetched: ${fonts.length}`);
  });
  await check("nothing is refused by the policy, no request fails and the console holds no error", async () => {
    const refused = await page.run("window.__refused");
    if (refused.length) throw new Error(`refused: ${refused.join("; ")}`);
    if (failed.length) throw new Error(`failed: ${failed.join("; ")}`);
    const errors = messages.filter((line) => /^(error|exception)/.test(line));
    if (errors.length) throw new Error(errors.join("; "));
  });
  console.log(`console: ${JSON.stringify(messages)}`);

  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  host.stdin.end();
  chrome?.kill();
  await sleep(1500);
  host.kill();
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
