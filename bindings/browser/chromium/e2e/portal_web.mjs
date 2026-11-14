// The web app against `canoa portal`, in a real browser: signing in on the portal's own form, the app's code pairing this browser's
// key through /portal/pair after the host turns it away, a session of the scripted agent, a reload that pairs nothing again, and
// signing out from the app. The page's code is app/tool/served_page.dart (a web target), the same one portal.mjs uses.
//
//   python app/tool/build_noise.py; in app/:
//   flutter build web --release --csp --no-web-resources-cdn --output <web>
//   flutter build web --release --csp --no-web-resources-cdn --base-href=/page/ -t tool/served_page.dart --output <page>
//   (the portal's `site` is <web> with <page> copied to <web>/page; its host offers an agent entry `scripted`, and the account's home
//   holds projects/app and a catalog repository)
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PORTAL=http://localhost:PORT CANOA_E2E_USER=name CANOA_E2E_PASSWORD=secret \
//   CANOA_E2E_CATALOG=file:///path/to/team.git@main#catalog.toml [CANOA_E2E_SHOT=<file.png>] node e2e/portal_web.mjs
//
// The password is read from the environment and never printed.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { lastCall, Session } from "./cdp.mjs";

const {
  CANOA_E2E_BROWSER: browser, CANOA_E2E_PORTAL: portal, CANOA_E2E_USER: user, CANOA_E2E_PASSWORD: password,
  CANOA_E2E_CATALOG: catalog, CANOA_E2E_SHOT: shot,
} = process.env;
if (!browser || !portal || !user || !password || !catalog) {
  throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_PORTAL, CANOA_E2E_USER, CANOA_E2E_PASSWORD and CANOA_E2E_CATALOG");
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-portal-web-e2e-"));

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
  // Every request a page makes (the page's own fetches and each WebSocket handshake), as the browser sent it.
  const open = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    page.requests = [];
    page.sockets = [];
    page.statuses = [];
    page.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.method === "Network.requestWillBeSent") page.requests.push(`${message.params.request.method} ${new URL(message.params.request.url).pathname}`);
      if (message.method === "Network.responseReceived") page.statuses.push(`${new URL(message.params.response.url).pathname} ${message.params.response.status}`);
      if (message.method === "Network.webSocketCreated") page.sockets.push(new URL(message.params.url).pathname);
    });
    await page.call("Page.enable");
    await page.call("Network.enable");
    await page.call("Runtime.enable");
    await page.call("Page.addScriptToEvaluateOnNewDocument", { source: RECORD });
    await page.call("Page.navigate", { url });
    return page;
  };
  const ready = (page) => waitFor("the page's code", () => page.run("Boolean(globalThis.canoaReady)").catch(() => false), 60_000).catch(async (error) => {
    throw new Error(`${error.message}; at ${await page.run("location.href").catch(() => "?")}; refused ${await page.run("window.__refused").catch(() => "?")}; log ${page.logs.join(" | ").slice(0, 800)}; requests ${page.requests.join(", ")}`);
  });
  const where = (page) => page.run("location.pathname");
  const loaded = (page, pathname) => waitFor(`${pathname}`, async () => (await where(page).catch(() => "")) === pathname);

  const first = await open(`${portal}/`);
  await check("a browser with no session is sent to the sign-in form, and nothing of the app is served", async () => {
    await loaded(first, "/portal/login");
    const flutter = await first.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))");
    if (flutter) throw new Error("the app was served to a signed-out browser");
  });
  await check("a wrong password is refused and signs nothing in", async () => {
    await first.run(`document.getElementById('user').value = ${JSON.stringify(user)};
      document.getElementById('password').value = 'not-the-password'; document.querySelector('form').submit();`);
    await sleep(1500);
    if ((await where(first)) !== "/portal/login") throw new Error(`landed on ${await where(first)}`);
  });
  await check("the right password signs in and lands on the app, whose cookie no script can read", async () => {
    await loaded(first, "/portal/login");
    await first.run(`document.getElementById('user').value = ${JSON.stringify(user)};
      document.getElementById('password').value = ${JSON.stringify(password)}; document.querySelector('form').submit();`);
    await waitFor("the app", async () => (await where(first).catch(() => "")) === "/" && (await first.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false)), 60_000);
    const cookies = await first.run("document.cookie");
    if (cookies.includes("canoa")) throw new Error(`script can read ${cookies}`);
    const stored = await first.call("Network.getCookies", { urls: [portal] });
    const cookie = stored.cookies.find((each) => each.name === "__Host-canoa");
    if (!cookie || !cookie.httpOnly || !cookie.secure) throw new Error(JSON.stringify(stored.cookies.map(({ name, httpOnly, secure }) => ({ name, httpOnly, secure }))));
  });
  await check("the app starts under the portal's policy with nothing refused, and reads the host from the portal", async () => {
    await sleep(4000);
    const refused = await first.run("window.__refused");
    if (refused.length) throw new Error(`refused: ${refused.join("; ")}`);
    if (!first.requests.includes("GET /canoa.json")) throw new Error(first.requests.join(", "));
  });
  await check("the app's own start pairs: one refused connect, one POST /portal/pair, then the control channel", async () => {
    const pairs = first.requests.filter((each) => each === "POST /portal/pair").length;
    const controls = first.sockets.filter((each) => each === "/portal/ws/control").length;
    if (pairs !== 1 || controls < 2) throw new Error(`pairs ${pairs}, control channels ${controls}: ${first.requests.join(", ")}`);
    const lines = first.logs.filter((line) => line.startsWith("error") || line.startsWith("exception"));
    if (lines.length) throw new Error(lines.join("; ").slice(0, 400));
  });

  // The page's code runs in the same browser, so it carries the same cookie. The key is already paired by the app's start.
  const page = await open(`${portal}/page/index.html`);
  await ready(page);
  const options = JSON.stringify({ entry: "scripted", catalog, overlay: "docs-tools" });
  let report;
  await check("a session of the scripted agent through the portal's relay, with folders, an upload, canoa.cfg and an overlay", async () => {
    report = JSON.parse(await page.run(`canoaFirst(${JSON.stringify(options)})`));
    if (!report.ok || !report.reachable) throw new Error(JSON.stringify(report).slice(0, 600));
    if (JSON.stringify(report.sources) !== '["scripted"]') throw new Error(`sources ${JSON.stringify(report.sources)}`);
    if (!report.answer) throw new Error("the agent did not answer");
    if (!report.upload.path || report.upload.size !== 19) throw new Error(JSON.stringify(report.upload));
    if (JSON.stringify(report.previewAdded) !== '["overlay.cfg"]') throw new Error(JSON.stringify(report.previewAdded));
  });
  await check("none of it asked the portal to pair again", async () => {
    const pairs = page.requests.filter((each) => each === "POST /portal/pair").length;
    if (pairs !== 0) throw new Error(`pairs ${pairs}`);
  });
  page.close();

  const again = await open(`${portal}/page/index.html`);
  await ready(again);
  await check("after a reload the key is still paired: the host's list and the session reopen with no pairing", async () => {
    const second = JSON.parse(await again.run(`canoaSecond(${JSON.stringify(JSON.stringify({ session: report.session, overlay: "docs-tools" }))})`));
    if (!second.ok || !second.found || second.reopened !== report.session) throw new Error(JSON.stringify(second).slice(0, 600));
    const pairs = again.requests.filter((each) => each === "POST /portal/pair").length;
    if (pairs !== 0) throw new Error(`pairs ${pairs}`);
    if (again.sockets.length === 0) throw new Error("no channel was opened");
  });
  again.close();

  // The app again: the signed-in account and Sign out, read from Flutter's accessibility tree.
  const app = await open(`${portal}/`);
  await waitFor("Flutter's view", () => app.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false), 60_000);
  await app.run("document.querySelector('flt-semantics-placeholder')?.click()");
  const labels = () => app.run(`[...document.querySelectorAll('flt-semantics')].map((node) => node.getAttribute('aria-label') || node.textContent).join('\\n')`);
  await check("the app shows the signed-in account and a Sign out", async () => {
    const shown = await waitFor("Sign out", async () => {
      const text = await labels();
      return text.includes("Sign out") ? text : null;
    }, 30_000);
    if (!shown.includes(user)) throw new Error(`the account is not shown: ${shown.slice(0, 400)}`);
    if (app.requests.includes("POST /portal/pair")) throw new Error("a reload paired again");
  });
  if (shot) {
    const { data } = await app.call("Page.captureScreenshot", { format: "png" });
    writeFileSync(shot, Buffer.from(data, "base64"));
  }
  await check("Sign out ends the session: the browser is sent to sign in and the host is no longer reachable", async () => {
    // A real click on the entry, as a person makes it: Flutter reads pointer events, not a DOM click.
    const box = await app.run(`(() => {
      // The smallest node that says it: a parent's text holds its children's.
      const area = (each) => { const rect = each.getBoundingClientRect(); return rect.width * rect.height; };
      const node = [...document.querySelectorAll('flt-semantics')].filter((each) => (each.getAttribute('aria-label') || each.textContent).includes('Sign out')).sort((a, b) => area(a) - area(b))[0];
      const rect = node.getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    })()`);
    for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
      await app.call("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", buttons: type === "mousePressed" ? 1 : 0, clickCount: 1 });
    }
    await loaded(app, "/portal/login").catch(async () => {
      throw new Error(`still at ${await where(app)} after a click at ${JSON.stringify(box)}; requests ${app.requests.slice(-6).join(", ")}`);
    });
    if (!app.requests.includes("POST /portal/logout")) throw new Error(app.requests.join(", "));
    // The sign-in page's own policy allows no fetch, so the index is asked for as a navigation.
    const index = await open(`${portal}/canoa.json`);
    await waitFor("the answer", () => index.statuses.some((each) => each.startsWith("/canoa.json")));
    if (!index.statuses.includes("/canoa.json 401")) throw new Error(`/canoa.json answered ${index.statuses.join(", ")}`);
    index.close();
    const cookies = (await app.call("Network.getCookies", { urls: [portal] })).cookies.filter((each) => each.name === "__Host-canoa");
    if (cookies.length) throw new Error("the cookie is still there");
  });
  await check("a page opened after sign-out goes to sign in and the relay refuses its channel", async () => {
    const out = await open(`${portal}/page/index.html`);
    await loaded(out, "/portal/login");
    const refused = await out.run(`new Promise((resolve) => { const socket = new WebSocket(location.origin.replace('http', 'ws') + '/portal/ws/control'); socket.onopen = () => resolve('open'); socket.onerror = () => resolve('refused'); })`);
    if (refused !== "refused") throw new Error(`channel ${refused}`);
    out.close();
  });
  app.close();
  first.close();

  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${error.stack ?? error.message}`);
} finally {
  clearTimeout(watchdog);
  chrome?.kill();
  await sleep(1500);
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
