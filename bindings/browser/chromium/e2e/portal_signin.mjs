// The portal's sign-in page in a real browser: the password form, the certificate button and enrolling a certificate, a refused
// certificate, a 429 counting down, and screenshots of the page at desktop and phone width, light and dark.
//
// The portal runs elsewhere (a Linux machine with accounts and a test certificate setup) behind a front that stands in for the
// proxy; the browser sets the header the real proxy would, so the certificate method is exercised end to end:
//
//   CANOA_E2E_BROWSER=<msedge or chrome> CANOA_E2E_URL=http://localhost:PORT \
//   CANOA_E2E_USER=<account> CANOA_E2E_PASSWORD=<its password> CANOA_E2E_OTHER_USER=<a second account> \
//   CANOA_E2E_TRUSTED=<file with a trusted certificate as base64 DER> CANOA_E2E_UNTRUSTED=<file with an unknown one for the same account> \
//   CANOA_E2E_SHOTS=<directory for screenshots> node e2e/portal_signin.mjs
//
// The trusted certificate is accepted by the portal's provider as it stands; the untrusted one is accepted only after the account
// enrols it, which the run does and then undoes.
//
// With CANOA_E2E_OUTSIDE=<an address outside 10.0.0.0/8> the portal runs the sign-in policy "a password only from 10.0.0.0/8, a
// certificate from anywhere" (docs/portal.md) behind a front that takes the browser's address from the X-Test-Address header
// (e2e/portal_front.py, whose default is 10.1.2.3), and the run also checks what the page offers, and what a session does, once
// the browser is outside.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
const other = need("CANOA_E2E_OTHER_USER");
const trusted = readFileSync(need("CANOA_E2E_TRUSTED"), "utf8").trim();
const untrusted = readFileSync(need("CANOA_E2E_UNTRUSTED"), "utf8").trim();
const shots = need("CANOA_E2E_SHOTS");
const outside = process.env.CANOA_E2E_OUTSIDE;
mkdirSync(shots, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const profile = mkdtempSync(path.join(tmpdir(), "canoa-signin-e2e-"));
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

async function targets() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((target) => target.type === "page");
      if (page) return page;
    } catch {
      // Not listening yet.
    }
    await sleep(200);
  }
  throw new Error("the browser did not start");
}

const page = new Session((await targets()).webSocketDebuggerUrl);
await page.call("Page.enable");
await page.call("Network.enable");

const mode = async ({ width, height, dark, mobile }) => {
  await page.call("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile });
  await page.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] });
};
// Where the front says the browser is: unset is the front's own default, inside 10.0.0.0/8.
let at;
const header = (cert) =>
  page.call("Network.setExtraHTTPHeaders", { headers: { ...(cert ? { "X-Client-Cert": cert } : {}), ...(at ? { "X-Test-Address": at } : {}) } });
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
async function shot(name) {
  const { data } = await page.call("Page.captureScreenshot", { format: "png" });
  writeFileSync(path.join(shots, `${name}.png`), Buffer.from(data, "base64"));
}
async function signedInAs() {
  const answer = await page.run(`fetch('/portal/session').then(async (r) => r.status + ' ' + (await r.text()))`);
  return answer;
}
const signOut = async () => {
  await page.call("Network.clearBrowserCookies");
};
async function fill(name, value) {
  await page.run(`(() => { const e = document.querySelector('[name=${name}]'); e.focus(); e.value = ${JSON.stringify(value)}; })()`);
}
async function passwordSignIn(account, secret) {
  await fill("user", account);
  await fill("password", secret);
  await click("form:has([name=password]) button[type=submit]");
}

const looks = [
  ["desktop-light", { width: 1280, height: 800, dark: false, mobile: false }],
  ["desktop-dark", { width: 1280, height: 800, dark: true, mobile: false }],
  ["phone-light", { width: 390, height: 844, dark: false, mobile: true }],
  ["phone-dark", { width: 390, height: 844, dark: true, mobile: true }],
];

await check("the methods list a password and a certificate for the proxy's connection", async () => {
  const methods = await (await fetch(`${base}/portal/methods`)).json();
  if (JSON.stringify(methods) !== JSON.stringify({ methods: ["password", "certificate"] })) throw new Error(JSON.stringify(methods));
});

await check("the sign-in page shows both methods under a strict policy, in four looks", async () => {
  await header(undefined);
  await go("/portal/login");
  const body = await text();
  if (!body.includes("Sign in with your certificate") || !body.includes("Account") || !body.includes("Password")) throw new Error(body);
  const policy = await (await fetch(`${base}/portal/login`)).headers.get("content-security-policy");
  if (policy.includes("unsafe-inline")) throw new Error(policy);
  const overflow = await page.run("document.documentElement.scrollWidth > document.documentElement.clientWidth");
  if (overflow) throw new Error("horizontal scroll");
  for (const [name, look] of looks) {
    await mode(look);
    await shot(`signin-${name}`);
    if (await page.run("document.documentElement.scrollWidth > document.documentElement.clientWidth")) throw new Error(`${name} scrolls sideways`);
  }
  await mode(looks[0][1]);
});

await check("a password signs in", async () => {
  await go("/portal/login");
  await passwordSignIn(user, password);
  await until("location.pathname === '/'", "the site");
  const answer = await signedInAs();
  if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
  await signOut();
});

await check("a wrong password reads the same for an account that exists and one that does not", async () => {
  const seen = [];
  for (const account of [user, "nobodyatall"]) {
    await go("/portal/login");
    await passwordSignIn(account, "definitely-wrong");
    await until("document.querySelector('.notice')", "the notice");
    seen.push(await page.run("document.querySelector('.notice').innerText"));
  }
  if (seen[0] !== seen[1] || seen[0] !== "Sign-in failed.") throw new Error(JSON.stringify(seen));
  await signOut();
});

await check("a trusted certificate signs in by the button", async () => {
  await header(trusted);
  await go("/portal/login");
  await click("form:has([value=certificate]) button");
  await until("location.pathname === '/'", "the site");
  const answer = await signedInAs();
  if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
  await signOut();
});

await check("an unknown certificate is refused and the password form is still there", async () => {
  await header(untrusted);
  await go("/portal/login");
  await click("form:has([value=certificate]) button");
  await until("document.querySelector('.notice')", "the notice");
  const shown = await text();
  if (!shown.includes("That certificate was not accepted here.") || !shown.includes("Password")) throw new Error(shown);
  await shot("certificate-refused-desktop-light");
  await mode(looks[2][1]);
  await shot("certificate-refused-phone-light");
  await mode(looks[0][1]);
  await signOut();
});

await check("a password sign-in offers the browser's new certificate, pinning it lets it sign in alone, and removing it undoes that", async () => {
  await header(untrusted);
  await go("/portal/login");
  await passwordSignIn(user, password);
  await until("location.pathname === '/portal/certificate'", "the certificate page");
  const shown = await text();
  if (!shown.includes("Use this certificate to sign in") || !shown.includes("SHA-256") || !shown.includes("Subject")) throw new Error(shown);
  for (const [name, look] of looks) {
    await mode(look);
    await shot(`certificate-offer-${name}`);
  }
  await mode(looks[0][1]);
  await click("form:has([value=enrol]) button");
  await until("document.body.innerText.includes('now signs you in')", "the confirmation");
  await shot("certificate-pinned-desktop-light");
  await signOut();
  await go("/portal/login");
  await click("form:has([value=certificate]) button");
  await until("location.pathname === '/'", "the site by certificate alone");
  if (!(await signedInAs()).includes(user)) throw new Error("not signed in as the account");
  // A session a certificate opened can see and remove the pin, and cannot add one.
  await go("/portal/certificate");
  const page2 = await text();
  if (!page2.includes("signs you in") || page2.includes("Use this certificate to sign in")) throw new Error(page2);
  await click("form:has([value=remove]) button");
  await until("document.body.innerText.includes('is removed')", "the removal");
  await signOut();
  await go("/portal/login");
  await click("form:has([value=certificate]) button");
  await until("document.querySelector('.notice')", "the refusal after removal");
  await signOut();
});

await check("a certificate that was not offered with the password sign-in asks for the password again before it is pinned", async () => {
  await header(undefined);
  await go("/portal/login");
  await passwordSignIn(user, password);
  await until("location.pathname === '/'", "the site");
  await header(untrusted);
  await go("/portal/certificate");
  if (!(await page.run("!!document.querySelector('form [name=password]')"))) throw new Error("no password field");
  // The certificate page's policy allows no fetch, so the bare post comes from the site's own page.
  await go("/");
  const bare = await page.run(`fetch('/portal/certificate', { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: 'action=enrol' }).then(async (r) => r.status + ' ' + (await r.text()))`);
  if (!bare.startsWith("403") || !bare.includes("Enter your password")) throw new Error(bare.slice(0, 200));
  await go("/portal/certificate");
  await fill("password", password);
  await click("form:has([value=enrol]) button");
  await until("document.body.innerText.includes('now signs you in')", "the confirmation");
  await click("form:has([value=remove]) button");
  await until("document.body.innerText.includes('is removed')", "the removal");
  await signOut();
});

if (outside) {
  await check("outside the network the page offers the certificate alone, in four looks", async () => {
    at = outside;
    await header(undefined);
    // The sign-in page's policy allows no fetch; the stylesheet is a document of the site's own that allows one.
    await go("/portal/login.css");
    const methods = await page.run(`fetch('/portal/methods').then((r) => r.text())`);
    if (JSON.stringify(JSON.parse(methods)) !== JSON.stringify({ methods: ["certificate"] })) throw new Error(methods);
    await go("/portal/login");
    const body = await text();
    if (!body.includes("Sign in with your certificate") || body.includes("Password") || body.includes("Account")) throw new Error(body);
    for (const [name, look] of looks) {
      await mode(look);
      await shot(`signin-outside-${name}`);
    }
    await mode(looks[0][1]);
  });

  await check("a password posted from outside is refused before anything is asked", async () => {
    at = outside;
    await header(undefined);
    await go("/portal/login.css");
    const refused = await page.run(`fetch('/portal/login', { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: ${JSON.stringify(`user=${user}&password=${password}`)} }).then((r) => r.status + ' ' + r.headers.get('set-cookie'))`);
    if (refused !== "403 null") throw new Error(refused);
    // Nothing counted: the same address still gets the certificate in. The earlier checks may have replaced or removed the pin,
    // so the account pins the trusted certificate again from inside, as its owner would.
    at = undefined;
    await header(trusted);
    await go("/portal/login");
    await passwordSignIn(user, password);
    await until("location.pathname === '/' || location.pathname === '/portal/certificate'", "the sign-in");
    if ((await where()) === "/portal/certificate") {
      await click("form:has([value=enrol]) button");
      await until("document.body.innerText.includes('now signs you in')", "the confirmation");
    }
    await signOut();
    at = outside;
    await header(trusted);
    await go("/portal/login");
    await click("form:has([value=certificate]) button");
    await until("location.pathname === '/'", "the site by certificate from outside");
    const answer = await signedInAs();
    if (!answer.startsWith("200") || !answer.includes(user)) throw new Error(answer);
    const index = await page.run(`fetch('/canoa.json').then((r) => r.status)`);
    if (index !== 200) throw new Error(`/canoa.json ${index}`);
    await signOut();
  });

  await check("a session a password opened inside ends when the browser is seen outside", async () => {
    at = undefined;
    await header(undefined);
    await go("/portal/login");
    await passwordSignIn(user, password);
    await until("location.pathname === '/'", "the site");
    const inside = await page.run(`fetch('/canoa.json').then((r) => r.status)`);
    if (inside !== 200) throw new Error(`inside: ${inside}`);
    at = outside;
    await header(undefined);
    const moved = await page.run(`fetch('/canoa.json').then((r) => r.status)`);
    if (moved !== 401) throw new Error(`outside: ${moved}`);
    at = undefined;
    await header(undefined);
    const back = await page.run(`fetch('/canoa.json').then((r) => r.status)`);
    if (back !== 401) throw new Error(`back inside: ${back}`);
    await signOut();
  });
}

await check("failed passwords end in a 429 whose wait counts down and holds the buttons", async () => {
  await header(undefined);
  for (let i = 0; i < 4; i++) {
    await go("/portal/login");
    await passwordSignIn(other, `wrong-${i}`);
    await until("document.querySelector('.notice')", "the notice");
  }
  const note = await page.run("document.querySelector('.notice').innerText");
  if (!note.includes("Too many failed attempts")) throw new Error(note);
  await until("/Try again in \\d+:\\d\\d/.test(document.querySelector('.notice').innerText)", "the countdown");
  const first = await page.run("document.querySelector('#wait').innerText");
  await sleep(2200);
  const second = await page.run("document.querySelector('#wait').innerText");
  const disabled = await page.run("[...document.querySelectorAll('button[type=submit]')].every((b) => b.disabled)");
  if (!disabled || first === second) throw new Error(`${first} / ${second} / disabled=${disabled}`);
  await shot("throttled-desktop-light");
  await mode(looks[3][1]);
  await shot("throttled-phone-dark");
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
