// A web page against the host that serves it, in a real browser: the app's own code for folders, uploads, canoa.cfg, the overlay
// store, the session list and a session of the host's scripted agent (app/tool/served_page.dart, a web target), run by the browser's
// real key proof against a real `canoa serve`. A page is loaded again afterwards, as after a reload, and reads the host's list,
// reopens the session and finds the overlay installed and selected. The app itself (app/lib/main.dart's build) is started too, and
// shows the host's session list.
//
//   python app/tool/build_noise.py; in app/:
//   flutter build web --release --csp --no-web-resources-cdn --output <web>
//   flutter build web --release --csp --no-web-resources-cdn --base-href=/page/ -t tool/served_page.dart --output <page>
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent \
//   CANOA_E2E_WEB=<web> CANOA_E2E_PAGE=<page> [CANOA_E2E_METHOD=noise] [CANOA_E2E_SHOT=<file.png>] node e2e/portal.mjs
//
// The host runs from a throwaway home with git on PATH, a local catalog repository and the scripted agent (`stdio_agent --scripted`).
import { execFileSync, spawn } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";

import { lastCall, Session } from "./cdp.mjs";

const {
  CANOA_E2E_BROWSER: browser, CANOA_E2E_HOST: hostProgram, CANOA_E2E_AGENT: agent, CANOA_E2E_WEB: web,
  CANOA_E2E_PAGE: pageDir, CANOA_E2E_SHOT: shot,
} = process.env;
if (!browser || !hostProgram || !agent || !web || !pageDir) {
  throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST, CANOA_E2E_AGENT, CANOA_E2E_WEB and CANOA_E2E_PAGE");
}
const METHOD = process.env.CANOA_E2E_METHOD ?? "noise";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const forward = (text) => text.replaceAll("\\", "/");
const work = mkdtempSync(path.join(tmpdir(), "canoa-portal-e2e-"));
const home = path.join(work, "home");
const site = path.join(work, "site");
const git = (cwd, ...args) => execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", ...args], { cwd, encoding: "utf8" });

cpSync(web, site, { recursive: true });
cpSync(pageDir, path.join(site, "page"), { recursive: true });
mkdirSync(path.join(home, "projects", "app"), { recursive: true });

// A catalog of one overlay, as a git repository the host reads from its own disk.
const repo = path.join(work, "team-work");
const bare = path.join(home, "repos", "team.git");
mkdirSync(path.join(repo, "overlays", "docs-tools"), { recursive: true });
writeFileSync(path.join(repo, "catalog.toml"), '[overlays.docs-tools]\nsource = "./overlays/docs-tools"\n\n[overlays.docs-tools.about]\nsummary = "Docs tools."\n');
writeFileSync(path.join(repo, "overlays", "docs-tools", "overlay.cfg"), '[env]\nDOCS = "one"\n');
git(repo, "init", "--quiet", "-b", "main");
mkdirSync(path.dirname(bare), { recursive: true });
git(work, "init", "--quiet", "--bare", bare);
git(repo, "add", "-A");
git(repo, "commit", "--quiet", "-m", "catalog");
git(repo, "push", "--quiet", bare, "HEAD:refs/heads/main");

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
async function waitFor(what, test, ms = 30_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}

const RECORD = `window.__refused = [];
  document.addEventListener("securitypolicyviolation", (event) => window.__refused.push(event.violatedDirective + " " + event.blockedURI));`;

// The browser's key belongs to the origin the host serves, so the origin is fixed before anything runs: a plain file server holds
// the port while the key's id is read, then the host takes it.
const types = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm", ".css": "text/css" };
const fixed = createServer((request, response) => {
  try {
    const file = path.join(site, decodeURIComponent(new URL(request.url, "http://x").pathname));
    if (!file.startsWith(site)) throw new Error("out");
    response.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    response.end(readFileSync(file));
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((resolve) => fixed.listen(0, "127.0.0.1", resolve));
const webPort = fixed.address().port;
const origin = `http://127.0.0.1:${webPort}`;

let chrome;
let host;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  host?.kill();
  process.exit(1);
}, 420_000);
try {
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
  const loadPage = async () => {
    const page = await open(`${origin}/page/index.html`);
    await waitFor("the page's code", () => page.run("globalThis.canoaReady === true").catch(() => false), 60_000);
    return page;
  };

  let page = await loadPage();
  const own = await page.run(`canoaOwnId(${JSON.stringify(METHOD)})`);
  console.log(`browser key ${own}`);
  page.close();
  await new Promise((resolve) => fixed.close(resolve));

  mkdirSync(path.join(work, "cfg"), { recursive: true });
  writeFileSync(path.join(work, "cfg", "canoa.cfg"), [
    "[host.control]", 'listen = "127.0.0.1:0"',
    `grants = { ${JSON.stringify(own)} = ["ledger.read", "ledger.write", "folders", "config.read", "config.write", "config.write.sessions", "upload", "overlay.read", "overlay.write"] }`,
    "[host.agents.scripted]", `program = ${JSON.stringify(forward(agent))}`, 'args = ["--scripted"]', 'listen = "127.0.0.1:0"',
    `working_roots = [${JSON.stringify(forward(home))}]`, `peers = [${JSON.stringify(own)}]`, "own_profile = true",
    "[host.web]", `dir = ${JSON.stringify(forward(site))}`, `listen = "127.0.0.1:${webPort}"`, "",
  ].join("\n"));
  const env = { ...process.env, HOME: home, USERPROFILE: home, COQUI_AGENTIC_PEER_HOME: path.join(work, "peers"), CANOA_LOG: "info" };
  host = spawn(hostProgram, ["serve", "--config", path.join(work, "cfg", "canoa.cfg"), "--method", METHOD, "--until-stdin-closes"],
    { env, stdio: ["pipe", "pipe", "inherit"] });
  const said = readline.createInterface({ input: host.stdout })[Symbol.asyncIterator]();
  const lines = [];
  for (let line; !(line = JSON.parse((await said.next()).value)).web;) lines.push(line);
  console.log(`host serves ${lines.map((line) => line.name).join(", ")} and the app at ${origin}`);

  const options = JSON.stringify({ entry: "scripted", catalog: `${forward(bare)}@main#catalog.toml`, overlay: "docs-tools" });
  page = await loadPage();
  const first = async () => JSON.parse(await page.run(`canoaFirst(${JSON.stringify(options)})`));

  let report;
  await check("a key the host has not accepted is turned away at its control channel, and the page learns it at once", async () => {
    report = await first();
    if (report.reachable !== false || !String(report.problem).includes("control channel")) throw new Error(JSON.stringify(report));
  });
  execFileSync(hostProgram, ["accept", "--", own], { env, encoding: "utf8" });

  await check("once accepted: folders, an upload, a session of the scripted agent in that folder, model and effort, an overlay installed and selected", async () => {
    report = await first();
    if (!report.ok) throw new Error(JSON.stringify(report));
    if (!report.reachable) throw new Error("not reachable");
    if (JSON.stringify(report.sources) !== '["scripted"]') throw new Error(`sources ${JSON.stringify(report.sources)}`);
    if (!forward(report.folder).endsWith("/projects/app")) throw new Error(`folder ${report.folder}`);
    if (!report.upload.path || report.upload.size !== 19) throw new Error(JSON.stringify(report.upload));
    if (!report.answer) throw new Error("the agent did not answer");
    if (report.model === "unchanged" || report.effort === "unchanged") throw new Error(`model ${report.model}, effort ${report.effort}`);
    if (JSON.stringify(report.previewAdded) !== '["overlay.cfg"]') throw new Error(JSON.stringify(report.previewAdded));
    if (report.restart !== false) throw new Error("[overlays] needs no restart");
    if (!report.profileItems.includes("env.DOCS")) throw new Error(`the overlay did not reach the session: ${JSON.stringify(report.profileItems)}`);
  });
  page.close();

  // After a reload: a new page reads what the host kept.
  page = await loadPage();
  await check("after a reload the host's session list has the session, it reopens, and the overlay is installed and selected", async () => {
    const again = JSON.parse(await page.run(`canoaSecond(${JSON.stringify(JSON.stringify({ session: report.session, overlay: "docs-tools" }))})`));
    if (!again.ok) throw new Error(JSON.stringify(again));
    if (!again.found || again.reopened !== report.session) throw new Error(JSON.stringify(again));
    if (!again.entries.some((entry) => entry.title === "Fix calc" && forward(entry.cwd) === forward(report.folder))) throw new Error(JSON.stringify(again.entries));
    if (JSON.stringify(again.selected) !== '["docs-tools"]') throw new Error(JSON.stringify(again.selected));
    if (again.installed !== report.digest) throw new Error(`installed ${again.installed}, previewed ${report.digest}`);
  });
  page.close();

  // The app itself starts with the host's session list and sources, under the host's policy.
  const app = await open(`${origin}/`);
  await app.call("Runtime.enable");
  await check("the app starts under the host's policy, with the host's control channel, and nothing it loads is refused", async () => {
    await waitFor("Flutter's view", () => app.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false), 60_000);
    await sleep(6000);
    const refused = await app.run("window.__refused");
    if (refused.length) throw new Error(`refused: ${refused.join("; ")}`);
    // What the app shows, read from Flutter's accessibility tree: the host's source, and no complaint of its control channel.
    await app.run("document.querySelector('flt-semantics-placeholder')?.click()");
    const shown = await waitFor("the app's texts", async () => {
      const text = await app.run(`[...document.querySelectorAll('flt-semantics')].map((node) => node.getAttribute('aria-label') || node.textContent).join('\\n')`);
      return text.includes("scripted") ? text : null;
    }, 15_000).catch(() => "");
    console.log(`app shows: ${JSON.stringify(shown.slice(0, 600))}`);
    const broken = app.logs.filter((line) => line.startsWith("error") || line.startsWith("exception"));
    if (broken.length) throw new Error(`the app logged: ${broken.join("; ").slice(0, 500)}`);
    if (!shown.includes("scripted")) throw new Error("the host's source is not offered");
    if (!shown.includes("Fix calc")) throw new Error("the host's session list is not shown");
    if (shown.includes("control channel")) throw new Error("the app could not use the control channel");
  });
  if (shot) {
    const { data } = await app.call("Page.captureScreenshot", { format: "png" });
    writeFileSync(shot, Buffer.from(data, "base64"));
  }
  app.close();

  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${error.stack ?? error.message}`);
} finally {
  clearTimeout(watchdog);
  host?.stdin.end();
  chrome?.kill();
  await sleep(1500);
  host?.kill();
  fixed.close();
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
