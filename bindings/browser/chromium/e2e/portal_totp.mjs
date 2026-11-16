// The portal's second step in a real browser: password, then a one-time code, on the page a person sees; setting a code up
// (the link, the QR code, the recovery codes) and signing in with it; and screenshots of the pages at desktop and phone width,
// light and dark.
//
// The portal serves from the lib test `serve_for_a_browser` (a Linux machine; it prints the line this reads from CANOA_E2E_SERVING):
//
//   CANOA_E2E_BROWSER=<msedge or chrome> CANOA_E2E_URL=http://localhost:PORT CANOA_E2E_USER=<account> \
//   CANOA_E2E_PASSWORD=<its password> CANOA_E2E_SHOTS=<directory> node e2e/portal_totp.mjs
//
// The codes are computed here from the secret the setup page shows, as an authenticator app does (RFC 6238).
import { createHmac } from "node:crypto";
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
const shots = need("CANOA_E2E_SHOTS");
mkdirSync(shots, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const profile = mkdtempSync(path.join(tmpdir(), "canoa-totp-e2e-"));
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
await page.call("Page.enable");
await page.call("Network.enable");

const base32 = (text) => {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = 0;
  let value = 0;
  const out = [];
  for (const c of text.replace(/=+$/, "")) {
    value = (value << 5) | alphabet.indexOf(c);
    bits += 5;
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 255);
      bits -= 8;
    }
  }
  return Buffer.from(out);
};
// The code for the thirty-second step `ahead` steps from now. A code is accepted once, and only later than the last one used.
const code = (secret, ahead) => {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000) + ahead));
  const digest = createHmac("sha1", base32(secret)).update(counter).digest();
  const offset = digest[19] & 15;
  const number = ((digest[offset] & 127) << 24) | (digest[offset + 1] << 16) | (digest[offset + 2] << 8) | digest[offset + 3];
  return String(number % 1000000).padStart(6, "0");
};

const mode = async ({ width, height, dark, mobile }) => {
  await page.call("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile });
  await page.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] });
};
const looks = [
  ["desktop-light", { width: 1280, height: 800, dark: false, mobile: false }],
  ["desktop-dark", { width: 1280, height: 800, dark: true, mobile: false }],
  ["phone-light", { width: 390, height: 844, dark: false, mobile: true }],
  ["phone-dark", { width: 390, height: 844, dark: true, mobile: true }],
];
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
  await until(`location.pathname === ${JSON.stringify(pathname)} && document.readyState === 'complete'`, pathname);
}
async function click(selector) {
  await page.run(`document.querySelector(${JSON.stringify(selector)}).click()`, { gesture: true });
}
async function shot(name) {
  const { data } = await page.call("Page.captureScreenshot", { format: "png" });
  writeFileSync(path.join(shots, `${name}.png`), Buffer.from(data, "base64"));
}
async function shots4(name) {
  for (const [look, size] of looks) {
    await mode(size);
    await shot(`${name}-${look}`);
    if (await page.run("document.documentElement.scrollWidth > document.documentElement.clientWidth")) throw new Error(`${name} ${look} scrolls sideways`);
  }
  await mode(looks[0][1]);
}
async function fill(selector, value) {
  await page.run(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); e.focus(); e.value = ${JSON.stringify(value)}; })()`);
}
async function password_step() {
  await go("/portal/login");
  await fill("[name=user]", user);
  await fill("[name=password]", password);
  await click("form:has([name=password]) button[type=submit]");
  await until("location.pathname === '/portal/login' && document.querySelector('[name=code], [href=\"/portal/factor\"]')", "the next step");
}
const signedInAs = () => page.run(`fetch('/portal/session').then(async (r) => r.status + ' ' + (await r.text()))`);
const signOut = () => page.call("Network.clearBrowserCookies");

let secret;
let recovery;

await check("a password alone asks for a code, and offers to set one up when none is", async () => {
  await password_step();
  const body = await text();
  if (!body.includes("One more step") || !body.includes("Set up a code") || !body.includes("Start over")) throw new Error(body);
  if (await page.run("!!document.querySelector('[name=password]')")) throw new Error("the password is asked again");
  await shots4("next-step-no-code");
});

await check("setting a code up shows the link, a QR code and recovery codes, and the first code signs in", async () => {
  await click("a[href='/portal/factor']");
  await until("location.pathname === '/portal/factor'", "the factor page");
  await click("form:has([value=begin]) button");
  await until("document.querySelector('svg.qr')", "the QR code");
  const body = await text();
  const link = body.match(/otpauth:\/\/totp\/[^\s]+/)?.[0];
  if (!link) throw new Error(body);
  secret = new URL(link).searchParams.get("secret");
  recovery = await page.run("[...document.querySelectorAll('.codes li')].map((e) => e.innerText)");
  if (recovery.length !== 10) throw new Error(`${recovery.length} recovery codes`);
  if (await page.run("!!document.querySelector('script')")) throw new Error("a script on the setup page");
  await shots4("setup");
  await fill("#code", code(secret, 0));
  await click("form:has([value=confirm]) button");
  await until("location.pathname === '/'", "the site");
  const answer = await signedInAs();
  if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
  await signOut();
});

await check("a wrong code is refused with the page still asking, and the right one signs in", async () => {
  await password_step();
  const body = await text();
  if (body.includes("Set up a code")) throw new Error("offers a setup to an account that has a code");
  await shots4("next-step");
  await fill("#code", "000000");
  await click("form:has(#code) button[type=submit]");
  await until("document.querySelector('.notice')", "the notice");
  const notice = await page.run("document.querySelector('.notice').innerText");
  if (notice !== "That code was not accepted.") throw new Error(notice);
  await shot("code-refused-desktop-light");
  await fill("#code", code(secret, 1));
  await click("form:has(#code) button[type=submit]");
  await until("location.pathname === '/'", "the site");
  const answer = await signedInAs();
  if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
  await signOut();
});

await check("a recovery code signs in once", async () => {
  for (const [round, expected] of [[0, "site"], [1, "refused"]]) {
    await password_step();
    await page.run("document.querySelector('details.recovery summary').click()");
    await fill("#recovery", recovery[0]);
    await click("form:has(#recovery) button[type=submit]");
    if (expected === "site") {
      await until("location.pathname === '/'", "the site");
      await signOut();
    } else {
      await until("document.querySelector('.notice')", "the notice");
      const notice = await page.run("document.querySelector('.notice').innerText");
      if (notice !== "That code was not accepted.") throw new Error(`round ${round}: ${notice}`);
    }
  }
});

await check("starting over goes back to the first step and the code field is gone", async () => {
  await signOut();
  await password_step();
  await click("form:has([value=cancel]) button");
  await until("!!document.querySelector('[name=password]') && !document.querySelector('#code')", "the first step");
});

console.log(results.join("\n"));
child.kill();
await sleep(500);
try {
  rmSync(profile, { recursive: true, force: true });
} catch {
  // The browser may still hold its profile.
}
process.exit(results.some((line) => line.startsWith("FAIL")) ? 1 : 0);
