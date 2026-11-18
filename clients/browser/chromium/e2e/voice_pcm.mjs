// The audio of a real voice, played by the web app's own Web Audio player in a real browser: PCM that Piper made is served beside
// the player's compiled code (app/tool/voice_page.dart), the page plays it as a voice provider's audio arrives, in chunks, and the
// run checks that it was queued whole and that playing it took about as long as the voice's own length, which a player that
// dropped it or ran it at the wrong rate would not.
//
//   piper --model en_US-ljspeech-medium.onnx --output_raw < sentence.txt > voice.pcm          (16-bit mono PCM, 22050 Hz)
//   dart compile js -O1 -o <dir>/page.js tool/voice_page.dart                                 (in app/)
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PAGE=<dir> CANOA_E2E_PCM=voice.pcm [CANOA_E2E_RATE=22050] node e2e/voice_pcm.mjs
//
// CANOA_E2E_TELL_RATE tells the page another rate than the voice's own, which plays the audio at the wrong speed: the run must fail.
import { spawn } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_PAGE: pageDir, CANOA_E2E_PCM: pcm } = process.env;
if (!browser || !pageDir || !pcm) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_PAGE and CANOA_E2E_PCM");
const RATE = Number(process.env.CANOA_E2E_RATE ?? 22050);
const TOLD = Number(process.env.CANOA_E2E_TELL_RATE ?? RATE);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-voice-e2e-"));
copyFileSync(path.join(pageDir, "page.js"), path.join(work, "page.js"));
copyFileSync(pcm, path.join(work, "voice.pcm"));
writeFileSync(path.join(work, "index.html"), '<!doctype html><meta charset="utf-8"><title>voice</title><script src="page.js" defer></script>');

const types = { ".html": "text/html", ".js": "text/javascript", ".pcm": "application/octet-stream" };
const site = createServer((request, answer) => {
  const file = path.join(work, request.url === "/" ? "index.html" : path.normalize(request.url).replace(/^[\\/]+/, ""));
  try {
    const body = readFileSync(file);
    answer.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    answer.end(body);
  } catch {
    answer.writeHead(404).end();
  }
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${site.address().port}`;

async function waitFor(what, test, ms = 30_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 180_000);
try {
  const profile = path.join(work, "profile");
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--autoplay-policy=no-user-gesture-required", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  });
  const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
  const page = new Session(made.webSocketDebuggerUrl);
  await page.call("Page.enable");
  await page.call("Page.navigate", { url: `${origin}/` });
  await waitFor("the page's code", () => page.run("globalThis.canoaReady === true").catch(() => false));
  console.log(`browser ${await page.run("navigator.userAgent")}`);

  const answer = JSON.parse(await page.run(`canoaPlay("/voice.pcm", ${TOLD})`, { gesture: true }));
  console.log(JSON.stringify(answer));
  if (answer.error) throw new Error(answer.error);
  const bytes = readFileSync(pcm).length;
  if (answer.bytes !== bytes) throw new Error(`queued ${answer.bytes} of ${bytes} bytes`);
  const seconds = bytes / (RATE * 2);
  // The audio is scheduled on the browser's own clock: playing it takes its length, give or take the start-up and the end.
  if (answer.tookMs < seconds * 1000 * 0.8 || answer.tookMs > seconds * 1000 + 5000) {
    throw new Error(`${seconds.toFixed(2)} s of voice played in ${answer.tookMs} ms`);
  }
  console.log(`ok   ${seconds.toFixed(2)} s of a real voice was queued whole and played in ${answer.tookMs} ms`);
  exitCode = 0;
  page.close();
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  chrome?.kill();
  site.close();
  await sleep(1500);
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
  } catch {
    // The browser may still hold its profile: the folder is in the temporary folder and goes with it.
  }
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
