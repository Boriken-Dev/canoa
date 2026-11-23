// Passkeys on the portal's pages in a real browser, with a virtual authenticator (the DevTools WebAuthn domain) in the place of a
// person's device: the page script runs `navigator.credentials` for real, so what Rust tests cannot run is run here. A person signs
// in by password, adds a passkey on the Passkeys page after typing the password again, signs out, and signs in by passkey with no
// name typed; a device that does not verify the person is refused; and no page breaks the Content-Security-Policy.
//
// The portal serves from the lib test `serve_a_passkey_portal_for_a_browser` (a Linux machine; it prints the line this reads from
// CANOA_E2E_SERVING), with the loopback network where 10.0.0.0/8 is:
//
//   CANOA_E2E_BROWSER=<msedge or chrome> CANOA_E2E_URL=http://localhost:PORT CANOA_E2E_USER=<account> \
//   CANOA_E2E_PASSWORD=<its password> [CANOA_E2E_SHOTS=<directory for screenshots>] node e2e/passkey.mjs
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
const user = need("CANOA_E2E_USER");
const password = need("CANOA_E2E_PASSWORD");
const shots = process.env.CANOA_E2E_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const profile = mkdtempSync(path.join(tmpdir(), "canoa-passkey-e2e-"));
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
// What the browser says about the page's own breaches of its policy.
const violations = [];
page.socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  // A request answered 401 or 404 is logged too, and some of these are asked to see them refused: only the policy's own are counted.
  if (message.method === "Log.entryAdded" && /Content Security Policy|Refused to/i.test(message.params.entry.text)) violations.push(message.params.entry.text);
});
await page.call("Page.enable");
await page.call("Network.enable");
await page.call("Log.enable");
await page.call("WebAuthn.enable");
// A phone or a laptop's own unlocking: it holds discoverable credentials, verifies the person, and answers at once.
const { authenticatorId } = await page.call("WebAuthn.addVirtualAuthenticator", {
  options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
});

const where = () => page.run("location.pathname + location.search");
const text = () => page.run("document.body.innerText");
async function until(expression, what) {
  for (let i = 0; i < 100; i++) {
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
const mode = (width, height, dark, mobile) =>
  Promise.all([
    page.call("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile }),
    page.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] }),
  ]);
const session = () => page.run(`fetch('/portal/session').then(async (r) => r.status + ' ' + (await r.text()))`);
const signOut = async () => {
  await page.run(`fetch('/portal/logout', { method: 'POST', redirect: 'manual' }).then(() => 0)`);
  const answer = await session();
  if (!answer.startsWith("401")) throw new Error(`still signed in: ${answer}`);
};
const credentials = async () => (await page.call("WebAuthn.getCredentials", { authenticatorId })).credentials;
const passwordSignIn = async () => {
  await go("/portal/login");
  await fill("user", user);
  await fill("password", password);
  await click("form:has([name=password]) button[type=submit]");
  await until("location.pathname === '/'", "the site");
};

await check("the page offers a passkey once its script runs, and the markup hides it until then", async () => {
  const markup = await (await fetch(`${base}/portal/login`)).text();
  if (!markup.includes("<div hidden data-gated>")) throw new Error("the passkey section is not hidden in the markup");
  await go("/portal/login");
  const shown = await page.run("(() => { const f = document.querySelector('form[data-passkey=get]'); return !!f && f.offsetParent !== null; })()");
  if (!shown) throw new Error("no visible passkey form");
  const body = await text();
  if (!body.includes("Sign in with a passkey")) throw new Error(body);
  const policy = await (await fetch(`${base}/portal/login`)).headers.get("content-security-policy");
  if (!policy.includes("script-src 'self'") || !policy.includes("connect-src 'self'") || policy.includes("unsafe-inline")) throw new Error(policy);
  await shot("login-desktop-light");
});

await check("the methods list a password and a passkey from the network the portal treats as inside", async () => {
  const methods = await (await fetch(`${base}/portal/methods`)).json();
  if (JSON.stringify(methods) !== JSON.stringify({ methods: ["password", "passkey"] })) throw new Error(JSON.stringify(methods));
});

await check("a password signs in and the Passkeys page shows none yet and asks for the password again", async () => {
  await passwordSignIn();
  await go("/portal/passkeys");
  const body = await text();
  if (!body.includes("No passkey is set up") || !body.includes("show who you are again")) throw new Error(body);
  if (await page.run("!!document.querySelector('form[data-passkey=create]')")) throw new Error("a passkey may be added with no proof");
  await shot("passkeys-ask-desktop-light");
});

await check("a passkey is added after the password is typed again, by the browser's own prompt", async () => {
  await page.run(`(() => { const form = document.querySelector('form:has([name=password])'); form.querySelector('[name=password]').value = ${JSON.stringify(password)}; })()`);
  await click("form:has([name=password]) button[type=submit]");
  await until("!!document.querySelector('form[data-passkey=create]')", "the form that adds one");
  await fill("label", "e2e laptop");
  await shot("passkeys-ready-desktop-light");
  await click("form[data-passkey=create] button[type=submit]");
  await until("document.body.innerText.includes('This passkey now signs you in.')", "the confirmation");
  const held = await credentials();
  if (held.length !== 1 || held[0].rpId !== "localhost" || !held[0].isResidentCredential) throw new Error(JSON.stringify(held));
  const body = await text();
  if (!body.includes("e2e laptop") || !body.includes("never used")) throw new Error(body);
  await shot("passkeys-added-desktop-light");
});

await check("adding another asks again, and a passkey shown again is what lets it", async () => {
  const before = await text();
  if (!before.includes("show who you are again")) throw new Error(before);
  await click("form[data-passkey=get] button[type=submit]");
  await until("!!document.querySelector('form[data-passkey=create]')", "the form that adds one, after the passkey");
});

await check("signing out leaves no session, and a passkey signs in with no name typed", async () => {
  await signOut();
  await go("/portal/login");
  if ((await page.run("document.querySelector('[name=user]').value")) !== "") throw new Error("a name was typed");
  await click("form[data-passkey=get] button[type=submit]");
  await until("location.pathname === '/'", "the site, after the passkey");
  const answer = await session();
  if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
  const held = await credentials();
  if (held[0].signCount < 1) throw new Error(`the credential was not used: ${JSON.stringify(held)}`);
});

await check("the Passkeys page then says when the passkey was used", async () => {
  await go("/portal/passkeys");
  const body = await text();
  if (!body.includes("used 20")) throw new Error(body);
});

await check("a device that does not verify the person is refused, and one that does signs in again", async () => {
  await signOut();
  await page.call("WebAuthn.setUserVerified", { authenticatorId, isUserVerified: false });
  await go("/portal/login");
  await click("form[data-passkey=get] button[type=submit]");
  await until("document.querySelector('[data-passkey-status]').innerText.length > 0", "the refusal");
  const said = await page.run("document.querySelector('[data-passkey-status]').innerText");
  const answer = await session();
  if (!answer.startsWith("401")) throw new Error(`signed in: ${answer}`);
  if ((await where()) !== "/portal/login") throw new Error(await where());
  results.push(`     (the unverified device was told: ${said})`);
  await page.call("WebAuthn.setUserVerified", { authenticatorId, isUserVerified: true });
  await go("/portal/login");
  await click("form[data-passkey=get] button[type=submit]");
  await until("location.pathname === '/'", "the site, once verified");
});

await check("removing the passkey ends the session it opened, and the page falls back to the password", async () => {
  await go("/portal/passkeys");
  await click("form:has([value=remove]) button[type=submit]");
  await until("document.body.innerText.includes('The passkey is removed.')", "the removal");
  const answer = await session();
  if (!answer.startsWith("401")) throw new Error(`still signed in after removing the passkey that signed in: ${answer}`);
  await go("/portal/login");
  const before = await credentials();
  if (before.length !== 1) throw new Error("the device should still hold its credential");
  await click("form[data-passkey=get] button[type=submit]");
  await until("document.querySelector('.notice')?.innerText.includes('That passkey was not accepted.')", "the refusal");
  if ((await session()).startsWith("200")) throw new Error("a removed passkey signed in");
});

await check("the pages in four looks do not scroll sideways, and none broke the content security policy", async () => {
  await passwordSignIn();
  for (const [name, look] of [
    ["desktop-light", [1280, 800, false, false]],
    ["desktop-dark", [1280, 800, true, false]],
    ["phone-light", [390, 844, false, true]],
    ["phone-dark", [390, 844, true, true]],
  ]) {
    await mode(...look);
    for (const pathname of ["/portal/passkeys", "/portal/certificate"]) {
      await go(pathname);
      if (await page.run("document.documentElement.scrollWidth > document.documentElement.clientWidth")) throw new Error(`${pathname} scrolls sideways at ${name}`);
      if (pathname === "/portal/passkeys") await shot(`passkeys-${name}`);
    }
  }
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
