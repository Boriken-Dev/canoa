// The web app's loading screen and its caching, in a real browser against `canoa serve` with [host.web]: the loading
// screen shows while the app's code is still arriving (the connection is slowed to make that long enough to see), the
// app then replaces it, and nothing is refused by the host's Content-Security-Policy, in the light and in the dark
// scheme. The light run is the page's second load in that browser, so it also shows the loading screen with its files kept.
//
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_HOST=path/to/canoa CANOA_E2E_WEB=app/build/web \
//   [CANOA_E2E_SHOTS=<dir>] node e2e/splash.mjs
//
// CANOA_E2E_SHOTS keeps a picture of the loading screen in each scheme and of the app after it.
import { spawn } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_HOST: hostProgram, CANOA_E2E_WEB: web, CANOA_E2E_SHOTS: shots } = process.env;
if (!browser || !hostProgram || !web) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST and CANOA_E2E_WEB");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-splash-e2e-"));
const site = path.join(work, "site");
cpSync(web, site, { recursive: true });
writeFileSync(path.join(work, "canoa.cfg"), ["[host.agents.echo]", `program = ${JSON.stringify(hostProgram)}`, "[host.web]", 'dir = "site"', 'listen = "127.0.0.1:0"', ""].join("\n"));
if (shots) mkdirSync(shots, { recursive: true });

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
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 600)}`);
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
  console.log(`the app at ${webUrl}`);
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

  // One page load, its requests counted as the browser saw them.
  async function load(scheme, { slow, shot, label }) {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    const requests = new Map();
    page.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      const { method, params } = message;
      if (method === "Network.requestWillBeSent") requests.set(params.requestId, { url: params.request.url });
      if (method === "Network.responseReceived") Object.assign(requests.get(params.requestId) ?? {}, { status: params.response.status, cache: params.response.fromDiskCache || params.response.fromMemoryCache });
      if (method === "Network.loadingFinished") Object.assign(requests.get(params.requestId) ?? {}, { bytes: params.encodedDataLength });
    });
    await page.call("Page.enable");
    await page.call("Network.enable");
    await page.call("Log.enable");
    await page.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
    await page.call("Page.addScriptToEvaluateOnNewDocument", { source: RECORD });
    if (slow) {
      await page.call("Network.emulateNetworkConditions", { offline: false, latency: 20, downloadThroughput: 1_500_000, uploadThroughput: 1_500_000 });
    }
    const started = Date.now();
    await page.call("Page.navigate", { url: `${webUrl}/` });
    const seen = { splash: false, appFirst: null, shot: false };
    await waitFor("the loading screen", async () => {
      const state = await page.run(`(() => { const s = document.getElementById("splash"); return s ? { shown: getComputedStyle(s).display !== "none" && s.getBoundingClientRect().height > 0, text: s.innerText, app: Boolean(document.querySelector("flutter-view, flt-glass-pane")) } : null; })()`).catch(() => null);
      if (state?.shown && !state.app) {
        seen.splash = true;
        seen.text = state.text;
        if (shot && !seen.shot) {
          seen.shot = true;
          const { data } = await page.call("Page.captureScreenshot", { format: "png" });
          writeFileSync(path.join(shots, `${shot}.png`), Buffer.from(data, "base64"));
        }
      }
      return state?.app;
    });
    seen.appFirst = Date.now() - started;
    const gone = await waitFor("the loading screen to go", () => page.run(`!document.getElementById("splash")`).catch(() => false), 10_000).then(() => Date.now() - started);
    seen.goneAt = gone;
    await sleep(1500);
    if (shot) {
      const { data } = await page.call("Page.captureScreenshot", { format: "png" });
      writeFileSync(path.join(shots, `${shot}_app.png`), Buffer.from(data, "base64"));
    }
    seen.refused = await page.run("window.__refused");
    seen.errors = page.logs.filter((line) => line.startsWith("exception") || line.startsWith("error"));
    const all = [...requests.values()];
    seen.requests = all.length;
    seen.fromCache = all.filter((r) => r.cache).length;
    seen.n304 = all.filter((r) => r.status === 304).length;
    seen.bytes = all.reduce((sum, r) => sum + (r.bytes ?? 0), 0);
    seen.big = all.filter((r) => (r.bytes ?? 0) > 100_000).map((r) => `${r.url.replace(webUrl, "")} ${r.bytes}`);
    console.log(`${label}: ${JSON.stringify(seen)}`);
    page.close();
    return seen;
  }

  let dark;
  await check("the loading screen shows while the code arrives, then the app replaces it, nothing refused (dark)", async () => {
    dark = await load("dark", { slow: true, shot: "splash_dark", label: "dark, first load" });
    if (!dark.splash) throw new Error("no loading screen was seen before the app");
    if (!dark.text.includes("Loading Canoa")) throw new Error(dark.text);
    if (dark.refused.length) throw new Error(`refused: ${dark.refused.join("; ")}`);
    if (dark.errors.length) throw new Error(dark.errors.join("; "));
  });
  await check("the same in the light scheme", async () => {
    const light = await load("light", { slow: true, shot: "splash_light", label: "light, second load" });
    if (!light.splash) throw new Error("no loading screen was seen before the app");
    if (light.refused.length) throw new Error(`refused: ${light.refused.join("; ")}`);
    if (light.errors.length) throw new Error(light.errors.join("; "));
  });

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
