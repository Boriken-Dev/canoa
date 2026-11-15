// The program Store and accounts of a person's own host through `canoa portal`, in a real browser: the browser signs in on the
// portal's form, and the app's own code (app/tool/store_accounts_page.dart, a web target) installs a made-up program from the
// Store and adds an account by an API key typed into a form the host sealed to this browser's key. The control channel never writes an
// entry, so the person's canoa.cfg gets the entry's `accounts` line from CANOA_E2E_OFFER (a command, as a JSON array, that also
// restarts the host); a second page then opens a session on the account. The host is `portal_standin_host` (canoa-host's example), whose Store is the echo stores, and whose
// Claude entry runs `harness_standin`, so nothing is downloaded and no real account is touched.
//
//   python app/tool/build_noise.py; in app/:
//   flutter build web --release --csp --no-web-resources-cdn --base-href=/page/ -t tool/store_accounts_page.dart --output <page>
//   (the portal's `site` has the page in its `page` folder; the portal and its throwaway accounts are tests/portal_users/setup.sh's)
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PORTAL=http://localhost:PORT CANOA_E2E_USER=name CANOA_E2E_PASSWORD=secret \
//   CANOA_E2E_KEY=the-api-key-to-type CANOA_E2E_OFFER='["wsl", "...", "offer.sh"]' node e2e/portal_store.mjs
//
// The password and the key are read from the environment and never printed.
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { lastCall, Session } from "./cdp.mjs";

const {
  CANOA_E2E_BROWSER: browser, CANOA_E2E_PORTAL: portal, CANOA_E2E_USER: user, CANOA_E2E_PASSWORD: password,
  CANOA_E2E_KEY: key, CANOA_E2E_OFFER: offer,
} = process.env;
if (!browser || !portal || !user || !password || !key || !offer) {
  throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_PORTAL, CANOA_E2E_USER, CANOA_E2E_PASSWORD, CANOA_E2E_KEY and CANOA_E2E_OFFER");
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-portal-store-e2e-"));

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).replaceAll(key, "<key>").slice(0, 800)}`);
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

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 300_000);
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
    await page.call("Runtime.enable");
    await page.call("Page.navigate", { url });
    return page;
  };
  const where = (page) => page.run("location.pathname");

  const first = await open(`${portal}/`);
  await check("a browser with no session is sent to the sign-in form", async () => {
    await waitFor("the form", async () => (await where(first).catch(() => "")) === "/portal/login");
  });
  await check("the right password signs in", async () => {
    await first.run(`document.getElementById('user').value = ${JSON.stringify(user)};
      document.getElementById('password').value = ${JSON.stringify(password)}; document.querySelector('form').submit();`);
    await waitFor("the app", async () => (await where(first).catch(() => "")) === "/", 60_000);
  });

  const page = await open(`${portal}/page/index.html`);
  await waitFor("the page's code", () => page.run("globalThis.canoaReady === true").catch(() => false), 60_000);
  const options = JSON.stringify({ name: "work", key, store: "echo_store" });
  let report;
  await check("the page's run finishes", async () => {
    report = JSON.parse(await page.run(`canoaFirst(${JSON.stringify(options)})`));
    if (!report.ok) throw new Error(JSON.stringify(report).slice(0, 700));
  });
  console.log(`report ${JSON.stringify(report)}`);
  await check("the host grants this browser's key the programs and the accounts", async () => {
    if (!report.grants?.programs || !report.grants?.accounts) throw new Error(JSON.stringify(report.grants));
  });
  await check("the Store lists the made-up store and installs the program from it", async () => {
    if (!report.stores?.includes("echo_store")) throw new Error(JSON.stringify(report.stores));
    if (report.plan?.action !== "install") throw new Error(JSON.stringify(report.plan));
    if (!report.installed?.version) throw new Error(JSON.stringify(report.installed));
    if (!(report.copiesAfter > (report.copiesBefore ?? 0))) throw new Error(`copies ${report.copiesBefore} then ${report.copiesAfter}`);
  });
  await check("the account is added by a key sealed to this browser's key, and listed", async () => {
    if (report.signIn !== "done") throw new Error(`${report.signIn}: ${report.signInProblem}`);
    if (!report.binding) throw new Error("the answer carried no binding");
    if (!report.accounts?.includes("work")) throw new Error(JSON.stringify(report.accounts));
    if (JSON.stringify(report).includes(key)) throw new Error("the key is in the page's report");
  });
  page.close();
  await check("the person's canoa.cfg offers the account under the entry, and the host starts again", async () => {
    const [program, ...args] = JSON.parse(offer);
    execFileSync(program, args, { stdio: "pipe", timeout: 60_000 });
  });
  const again = await open(`${portal}/page/index.html`);
  await waitFor("the page's code", () => again.run("globalThis.canoaReady === true").catch(() => false), 60_000);
  await check("a session opens on the entry that offers the account", async () => {
    const second = JSON.parse(await again.run(`canoaSecond(${JSON.stringify(JSON.stringify({ entry: "Claude", name: "work" }))})`));
    console.log(`second ${JSON.stringify(second)}`);
    if (!second.ok || !second.session) throw new Error(JSON.stringify(second).slice(0, 600));
  });
  again.close();
  first.close();
  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${String(error.stack ?? error.message).replaceAll(key, "<key>")}`);
} finally {
  clearTimeout(watchdog);
  chrome?.kill();
  await sleep(1500);
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
