// The portal's own pages are reachable from the app it serves, in a real browser: the app's code (app/tool/signin_page.dart, a web
// target, which is the portal's site here) reads what `GET /portal/account` says the signed-in person may open, opens the one-time
// code page in the same tab, and the portal's pages link to each other and back to the app. A signed-out browser is told nothing.
//
// The portal serves from the lib test `serve_the_account_pages_for_a_browser` (a Linux machine; it prints the line this reads from
// CANOA_E2E_SERVING) with the page built into the folder it is given:
//
//   CANOA_E2E_BROWSER=<msedge or chrome> CANOA_E2E_URL=http://localhost:PORT CANOA_E2E_USER=<account> \
//   CANOA_E2E_PASSWORD=<its password> [CANOA_E2E_SHOTS=<directory for screenshots>] node e2e/portal_account.mjs
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { Session } from "./cdp.mjs";

const need = (name) => {
  const value = process.env[name];
  if (!value) throw new Error(`set ${name}`);
  return value;
};
const browser = need("CANOA_E2E_BROWSER");
const base = need("CANOA_E2E_URL");
const password = need("CANOA_E2E_PASSWORD");
const user = need("CANOA_E2E_USER");
const shots = process.env.CANOA_E2E_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const profile = mkdtempSync(path.join(tmpdir(), "canoa-account-e2e-"));
const port = 9300 + Math.floor(Math.random() * 500);
const child = spawn(browser, [`--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "--headless=new", "--no-first-run", "--disable-gpu", "about:blank"], { stdio: "ignore" });

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${error.message}`);
  }
}

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((each) => each.type === "page");
      if (page) return page;
    } catch {
      // Not listening yet.
    }
    await sleep(200);
  }
  throw new Error("the browser did not start");
}

const page = new Session((await target()).webSocketDebuggerUrl);
const violations = [];
page.socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.method === "Log.entryAdded" && /Content Security Policy/i.test(message.params.entry.text)) violations.push(message.params.entry.text);
});
await page.call("Page.enable");
await page.call("Network.enable");
await page.call("Log.enable");

const text = () => page.run("document.body.innerText");
const where = () => page.run("location.pathname");
async function until(expression, what) {
  for (let i = 0; i < 300; i++) {
    try {
      if (await page.run(expression)) return;
    } catch {
      // The page is between documents.
    }
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
}
async function go(pathname) {
  await page.call("Page.navigate", { url: base + pathname });
  await until(`location.pathname === ${JSON.stringify(pathname.split("?")[0])} && document.readyState === 'complete'`, pathname);
}
async function click(selector) {
  await page.run(`document.querySelector(${JSON.stringify(selector)}).click()`, { gesture: true });
}
async function fill(name, value) {
  await page.run(`(() => { const e = document.querySelector('[name=${name}]'); e.focus(); e.value = ${JSON.stringify(value)}; })()`);
}
async function shot(name) {
  if (!shots) return;
  const { data } = await page.call("Page.captureScreenshot", { format: "png" });
  writeFileSync(path.join(shots, `${name}.png`), Buffer.from(data, "base64"));
}
const links = () => page.run("[...document.querySelectorAll('nav.pages a, a')].map((a) => a.getAttribute('href'))");
// The app's page, asked for what its Sign-in page lists.
const account = async () => {
  await until("window.canoaReady === true", "the app's page");
  return JSON.parse(await page.run("canoaSigninAccount()"));
};

await check("a browser with no session is not told what its person may open, and is sent to sign in", async () => {
  const answer = await fetch(`${base}/portal/account`);
  if (answer.status !== 401) throw new Error(`status ${answer.status}`);
  const site = await fetch(`${base}/`, { redirect: "manual" });
  if (site.status !== 303) throw new Error(`the app answered ${site.status} to a signed-out browser`);
});

await check("a password signs in and the app's page lists the code, the certificate and sign out as the portal says", async () => {
  await go("/portal/login");
  await fill("user", user);
  await fill("password", password);
  await click("form:has([name=password]) button[type=submit]");
  await until("location.pathname === '/'", "the app");
  const listed = await account();
  if (listed.error) throw new Error(JSON.stringify(listed));
  if (listed.user !== user) throw new Error(JSON.stringify(listed));
  if (listed.code.path !== "/portal/factor" || !listed.code.allowed || listed.codeSetUp) throw new Error(JSON.stringify(listed.code));
  if (listed.passkeys.path !== undefined) throw new Error(`a portal with no passkey provider listed passkeys: ${JSON.stringify(listed.passkeys)}`);
  if (listed.certificate.path !== "/portal/certificate") throw new Error(JSON.stringify(listed.certificate));
  await shot("app");
});

await check("the app opens the code page in the same tab, and it offers to set a code up", async () => {
  await page.run("canoaSigninOpen('code')");
  await until("location.pathname === '/portal/factor'", "the code page");
  await until("document.readyState === 'complete'", "the code page's load");
  const body = await text();
  if (!body.includes("No code is set up") || !body.includes("show who you are again")) throw new Error(body);
  await shot("factor");
});

await check("the code page links to the certificate page and back to the app, and not to itself", async () => {
  const hrefs = await links();
  if (!hrefs.includes("/portal/certificate") || !hrefs.includes("/")) throw new Error(JSON.stringify(hrefs));
  if (hrefs.includes("/portal/factor")) throw new Error(`the page links to itself: ${JSON.stringify(hrefs)}`);
});

await check("the certificate page links to the code page, and the link opens it", async () => {
  await click("nav.pages a[href='/portal/certificate']");
  await until("location.pathname === '/portal/certificate' && document.readyState === 'complete'", "the certificate page");
  const hrefs = await links();
  if (!hrefs.includes("/portal/factor") || !hrefs.includes("/")) throw new Error(JSON.stringify(hrefs));
  await click("nav.pages a[href='/portal/factor']");
  await until("location.pathname === '/portal/factor' && document.readyState === 'complete'", "the code page again");
});

await check("Continue to Canoa returns to the app", async () => {
  await click("nav.pages a[href='/']");
  await until("location.pathname === '/' && window.canoaReady === true", "the app");
});

await check("signing out from the app's session leaves the app's question unanswered", async () => {
  await page.run(`fetch('/portal/logout', { method: 'POST', redirect: 'manual' }).then(() => 0)`);
  const status = await page.run(`fetch('/portal/account').then((r) => r.status)`);
  if (status !== 401) throw new Error(`status ${status}`);
});

await check("no page broke the content security policy", async () => {
  if (violations.length) throw new Error(violations.join("; "));
});

page.close();
child.kill();
await sleep(300);
try {
  rmSync(profile, { recursive: true, force: true });
} catch {
  // The browser may still hold a file; the temp folder is cleaned by the system.
}
console.log(results.join("\n"));
if (results.some((line) => line.startsWith("FAIL"))) process.exit(1);
