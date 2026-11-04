// The web app as canoa-host serves it, in a real browser: `serve` with [host.web] serves a `flutter build web` output, which
// starts under the host's Content-Security-Policy with nothing refused; then Canoa's own connection code, compiled from
// app/tool/peer_page.dart and served from the same site, reads canoa-host.json, is turned away by the host's agent entry
// until its key is accepted, and after `canoa-host accept` runs a turn with that entry's scripted agent.
//
//   dart compile js -O1 -o <dir>/page.js tool/peer_page.dart        (in app/)
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_HOST=path/to/canoa-host CANOA_E2E_AGENT=path/to/stdio_agent \
//   CANOA_E2E_WEB=app/build/web CANOA_E2E_APP_PAGE=<dir> [CANOA_E2E_METHOD=noise] [CANOA_E2E_SHOT=<file.png>] \
//   [CANOA_E2E_CONNECT=https://fonts.gstatic.com] node e2e/served.mjs
//
// The agent must echo a prompt as `echo: TEXT`, as canoa-host's stdio_agent example does. CANOA_E2E_SHOT keeps a picture of
// the app as it started. CANOA_E2E_CONNECT, comma-separated, is [host.web] connect: origins the page may reach besides the
// entries, such as the one Flutter's web build fetches its fonts from when the app does not bundle them.
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";

import { lastCall, Session } from "./cdp.mjs";

const {
  CANOA_E2E_BROWSER: browser, CANOA_E2E_HOST: hostProgram, CANOA_E2E_AGENT: agent, CANOA_E2E_WEB: web,
  CANOA_E2E_APP_PAGE: appPage, CANOA_E2E_SHOT: shot, CANOA_E2E_CONNECT: connect,
} = process.env;
if (!browser || !hostProgram || !agent || !web || !appPage) {
  throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST, CANOA_E2E_AGENT, CANOA_E2E_WEB and CANOA_E2E_APP_PAGE");
}
const METHOD = process.env.CANOA_E2E_METHOD ?? "noise";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-served-e2e-"));
const home = path.join(work, "home");
const site = path.join(work, "site");

// The built app, and beside it a bare page running the app's own connection code on the app's own peer bridge.
cpSync(web, site, { recursive: true });
copyFileSync(path.join(appPage, "page.js"), path.join(site, "page.js"));
writeFileSync(path.join(site, "peer.html"),
  '<!doctype html><title>peer</title><script type="module" src="canoa_peer_bridge.mjs"></script><script src="page.js" defer></script>');
writeFileSync(path.join(work, "Canoa.cfg"), [
  "[host.agents.echo]", `program = ${JSON.stringify(agent)}`,
  "[host.web]", 'dir = "site"', 'listen = "127.0.0.1:0"',
  ...(connect ? [`connect = ${JSON.stringify(connect.split(","))}`] : []), "",
].join("\n"));

const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: home, CANOA_LOG: "info" };
const host = spawn(hostProgram, ["serve", "--config", path.join(work, "Canoa.cfg"), "--method", METHOD, "--until-stdin-closes"],
  { env, stdio: ["pipe", "pipe", "inherit"] });
const said = readline.createInterface({ input: host.stdout })[Symbol.asyncIterator]();
const hear = async () => JSON.parse((await said.next()).value);

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
async function waitFor(what, test, ms = 30_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}

// Every violation of the page's policy, recorded from before its first script.
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
  const entry = await hear();
  const { web: webUrl } = await hear();
  console.log(`host ${entry.peer} serves ${entry.name} at ${entry.url}, the app at ${webUrl}`);
  const profile = path.join(work, "profile");
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--window-size=1280,800", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  });
  const open = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    await page.call("Page.enable");
    await page.call("Page.addScriptToEvaluateOnNewDocument", { source: RECORD });
    await page.call("Page.navigate", { url });
    return page;
  };

  const app = await open(`${webUrl}/`);
  await check("the app starts under the host's policy, and nothing it loads is refused", async () => {
    await waitFor("Flutter's view", () => app.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false), 60_000);
    await sleep(3000);
    const refused = await app.run("window.__refused");
    if (refused.length) throw new Error(`refused: ${refused.join("; ")}`);
    const policy = await app.run(`fetch("/").then((answer) => answer.headers.get("content-security-policy"))`);
    if (!policy.includes(`connect-src 'self' ${entry.url}`)) throw new Error(policy);
  });
  if (shot) {
    const { data } = await app.call("Page.captureScreenshot", { format: "png" });
    writeFileSync(shot, Buffer.from(data, "base64"));
  }

  const page = await open(`${webUrl}/peer.html`);
  await waitFor("the page's bridge and code", () => page.run("Boolean(globalThis.canoaPeer && globalThis.canoaTurn && globalThis.canoaServedHost)").catch(() => false));
  console.log(`browser ${await page.run("navigator.userAgent")}`);
  const call = (name, ...args) => page.run(`${name}(${args.map((arg) => JSON.stringify(arg)).join(", ")})`);
  const turn = async (text) => JSON.parse(await call("canoaTurn", entry.url, entry.peer, METHOD, text));

  await check("the app reads the host's entries, its id and its method from canoa-host.json", async () => {
    const served = JSON.parse(await call("canoaServedHost"));
    const expected = { peer: entry.peer, method: METHOD, entries: [{ name: entry.name, kind: entry.kind, url: entry.url }] };
    if (JSON.stringify(served) !== JSON.stringify(expected)) throw new Error(JSON.stringify(served));
  });

  const own = await call("canoaOwnId", METHOD);
  await check("an unpaired key is turned away by the entry, and waits on the host", async () => {
    const refused = await turn("too soon").then(() => "served", (error) => error.message);
    if (refused === "served") throw new Error("an unpaired key was served");
    const waiting = execFileSync(hostProgram, ["peers", "--format", "json"], { env, encoding: "utf8" });
    if (!waiting.includes(own)) throw new Error(`not waiting: ${waiting}`);
  });

  await check("once accepted on the host, a turn runs with its agent through the entry", async () => {
    execFileSync(hostProgram, ["accept", "--", own], { env, encoding: "utf8" });
    const ran = await waitFor("a served turn", () => turn("hello from the served app").catch(() => null), 20_000);
    if (ran.peer !== entry.peer || ran.echoed !== "echo: hello from the served app") throw new Error(JSON.stringify(ran));
  });

  await check("the policy keeps the page from connecting anywhere but this host's entries", async () => {
    if ((await page.run("window.__refused")).length) throw new Error(`refused: ${(await page.run("window.__refused")).join("; ")}`);
    await page.run(`new Promise((resolve) => { const socket = new WebSocket("ws://127.0.0.1:9"); socket.onerror = socket.onclose = resolve; setTimeout(resolve, 2000); }).catch(() => null)`).catch(() => null);
    await page.run(`fetch("https://example.com/").catch(() => null)`);
    const refused = await waitFor("the refusals", async () => {
      const now = await page.run("window.__refused");
      return now.length >= 2 ? now : null;
    }, 5000);
    if (!refused.every((line) => line.startsWith("connect-src"))) throw new Error(refused.join("; "));
  });

  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
  app.close();
  page.close();
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
