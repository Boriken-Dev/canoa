// The add-in's pages as a host on this computer serves them, in a real Chromium browser (the engine Office's WebView2 is): over
// HTTPS with a certificate the run makes for itself, the chat-only app starts under the host's policy with nothing refused, and
// its key is accepted by the host with no PIN and no `canoa accept`, after which the control channel answers.
//
//   CANOA_E2E_BROWSER=path/to/msedge CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent \
//   CANOA_E2E_WEB=app/build/web [CANOA_E2E_SHOT=<file.png>] node clients/office/e2e/local.mjs
//
// The browser is told to ignore this run's certificate for its own throwaway profile; nothing is put in any trust store. The
// certificate is made with `openssl`, which has to be on PATH.
//
// The browser resolves no name but localhost, so a request to any other host fails: the task pane has to load Office.js, and
// everything it loads, from the host itself (a build made with CANOA_OFFICE_JS=cdn is the one that goes to Microsoft's address,
// and this run then fails on it).
import { execFileSync, spawn } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import readline from 'node:readline';

import { lastCall, Session } from '../../browser/chromium/e2e/cdp.mjs';

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_HOST: hostProgram, CANOA_E2E_AGENT: agent, CANOA_E2E_WEB: web, CANOA_E2E_SHOT: shot } = process.env;
if (!browser || !hostProgram || !agent || !web) {
  throw new Error('set CANOA_E2E_BROWSER, CANOA_E2E_HOST, CANOA_E2E_AGENT and CANOA_E2E_WEB');
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), 'canoa-office-local-e2e-'));
const home = path.join(work, 'home');
const site = path.join(work, 'site');
const PORT = 7800 + (process.pid % 100);

cpSync(web, site, { recursive: true });
execFileSync('openssl', [
  'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=localhost',
  '-addext', 'subjectAltName=DNS:localhost', '-keyout', path.join(work, 'web.key'), '-out', path.join(work, 'web.crt'),
], { stdio: 'ignore' });
writeFileSync(path.join(work, 'canoa.cfg'), [
  '[host.agents.echo]', `program = ${JSON.stringify(agent)}`,
  '[host.control]', 'listen = "127.0.0.1:0"',
  '[host.web]', 'dir = "site"', `listen = "127.0.0.1:${PORT}"`, 'cert = "web.crt"', 'key = "web.key"', 'office = true', 'office_local = true', '',
].join('\n'));

const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: home, CANOA_LOG: 'info' };
const host = spawn(hostProgram, ['serve', '--config', path.join(work, 'canoa.cfg'), '--until-stdin-closes'], { env, stdio: ['pipe', 'pipe', 'inherit'] });
const said = readline.createInterface({ input: host.stdout })[Symbol.asyncIterator]();
const hear = async () => JSON.parse((await said.next()).value);

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
    await sleep(250);
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
  // The agent's line, the control channel's, then the web server's.
  const lines = [await hear(), await hear(), await hear()];
  const printed = lines.find((line) => line.web)?.web;
  if (new URL(printed).port !== String(PORT)) throw new Error(`the web app is at ${printed}`);
  // The host prints the address it bound; the manifest and the browser name it localhost.
  const origin = `https://localhost:${PORT}`;
  console.log(`host serves the add-in at ${origin}`);
  const profile = path.join(work, 'profile');
  chrome = spawn(browser, [
    '--headless=new', `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check',
    '--ignore-certificate-errors', '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1',
    '--window-size=1000,800', 'about:blank',
  ], { stdio: 'ignore' });
  const debug = await waitFor('the browser', () => {
    try {
      return readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0];
    } catch {
      return null;
    }
  });
  const open = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: 'PUT' })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    await page.call('Page.enable');
    await page.call('Runtime.enable');
    await page.call('Page.addScriptToEvaluateOnNewDocument', { source: RECORD });
    await page.call('Page.navigate', { url });
    return page;
  };
  // One JSON object a line: {"peer", "state", "source"}.
  const peers = () => execFileSync(hostProgram, ['peers', '--format', 'json'], { env, encoding: 'utf8' })
    .split('\n').filter(Boolean).map((line) => JSON.parse(line));

  await check('the task pane page is served over HTTPS and loads Office.js, from the host alone, under its policy', async () => {
    const pane = await open(`${origin}/office/taskpane.html`);
    await waitFor('the pane', () => pane.run('document.readyState === "complete"').catch(() => false), 30_000);
    await sleep(2000);
    const refused = await pane.run('window.__refused');
    if (refused.length) throw new Error(`refused: ${refused.join('; ')}`);
    if ((await pane.run('typeof Office')) !== 'object') throw new Error('Office.js is not defined in the pane');
    const loaded = await pane.run('performance.getEntriesByType("resource").map((entry) => entry.name)');
    const elsewhere = loaded.filter((name) => !name.startsWith(`${origin}/`));
    if (elsewhere.length) throw new Error(`the pane asked for ${elsewhere.join(', ')}`);
    if (!loaded.includes(`${origin}/office/js/office.js`)) throw new Error(`Office.js was not loaded from the host: ${loaded.join(', ')}`);
    pane.close();
  });

  await check('the host says it admits the page at a loopback name, and answers beside the app page too', async () => {
    const page = await open(`${origin}/office/app/`);
    await waitFor('the page', () => page.run('document.readyState === "complete"').catch(() => false), 30_000);
    const answered = await page.run(`Promise.all(["/canoa.json", "/office/app/canoa.json"].map((p) => fetch(p).then((r) => r.json())))`);
    for (const index of answered) if (index.admit !== '/office/admit') throw new Error(JSON.stringify(index));
    page.close();
  });

  const app = await open(`${origin}/office/app/?mode=chat`);
  await check('the chat-only app starts in the host\'s page with nothing refused, and its key is admitted with no pairing', async () => {
    await waitFor("Flutter's view", () => app.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false), 60_000);
    const accepted = await waitFor('the page\'s key to be accepted', () => {
      return peers().find((key) => key.state === 'accepted');
    }, 60_000).catch((error) => {
      throw new Error(`${error.message}\npeers: ${JSON.stringify(peers())}\nlogs: ${app.logs.slice(-12).join(' | ')}`);
    });
    console.log(`accepted ${accepted.peer}`);
    await sleep(2000);
    const refused = await app.run('window.__refused');
    if (refused.length) throw new Error(`refused: ${refused.join('; ')}\n${app.logs.slice(-8).join('\n')}`);
  });
  if (shot) {
    const { data } = await app.call('Page.captureScreenshot', { format: 'png' });
    writeFileSync(shot, Buffer.from(data, 'base64'));
  }
  await check('the page found its host and reached the control channel: nothing was logged as failed', async () => {
    const failed = app.logs.filter((line) => /control channel cannot be used|would not accept|never saw/.test(line));
    if (failed.length) throw new Error(failed.join('\n'));
  });

  exitCode = results.every((line) => line.startsWith('ok')) ? 0 : 1;
  app.close();
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
console.log(exitCode === 0 ? 'PASSED' : 'FAILED');
process.exit(exitCode);
