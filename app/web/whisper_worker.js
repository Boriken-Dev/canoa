// Whisper in a web worker: canoa-whisper-candle's engine, compiled to WebAssembly by app/tool/build_whisper.py (whisper/canoa_whisper.js
// and its .wasm are generated, never committed), behind a page that posts messages. The page never waits on the engine: a clip takes
// seconds of one thread, which would stop the page's own.
//
// Messages in:  {id, type: "catalog"}
//               {id, type: "transcribe", model, samples: Float32Array (16 kHz mono), language, prompt}
// Messages out: {id, ok: true, catalog} | {id, ok: true, text, language} | {id, ok: false, error} | {id, progress: "words"}
//
// A model's three files are fetched from the pinned addresses the module lists, each checked against its SHA-256 before it is kept,
// and kept in the browser's Cache Storage, so a model is downloaded once. The engine holds one model at a time.
import init, { Whisper, catalog } from "./whisper/canoa_whisper.js";

const CACHE = "canoa-whisper-models";
let ready = null;
let held = null;
let heldId = null;

const hex = (buffer) => [...new Uint8Array(buffer)].map((byte) => byte.toString(16).padStart(2, "0")).join("");

async function module() {
  ready ??= init();
  await ready;
}

async function file(entry, progress) {
  const cache = await caches.open(CACHE);
  const known = await cache.match(entry.url);
  if (known) return new Uint8Array(await known.arrayBuffer());
  progress(`Downloading ${entry.name} (${Math.ceil(entry.bytes / 1e6)} MB)`);
  const response = await fetch(entry.url);
  if (!response.ok) throw new Error(`${entry.url} answered ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const digest = hex(await crypto.subtle.digest("SHA-256", bytes));
  if (digest !== entry.sha256) throw new Error(`${entry.name} is not the file its publisher lists (SHA-256 ${digest})`);
  await cache.put(entry.url, new Response(bytes));
  return bytes;
}

async function load(id, progress) {
  if (held && heldId === id) return held;
  const model = JSON.parse(catalog()).models.find((each) => each.id === id);
  if (!model) throw new Error(`there is no model ${id}`);
  held?.free();
  held = null;
  heldId = null;
  const parts = [];
  for (const name of ["config.json", "tokenizer.json", "model.safetensors"]) {
    parts.push(await file(model.files.find((each) => each.name === name), progress));
  }
  const [config, tokenizer, weights] = parts;
  progress(`Loading ${id}`);
  held = new Whisper(config, tokenizer, weights);
  heldId = id;
  return held;
}

self.onmessage = async ({ data }) => {
  const { id, type } = data;
  const progress = (words) => self.postMessage({ id, progress: words });
  try {
    await module();
    if (type === "catalog") {
      self.postMessage({ id, ok: true, catalog: JSON.parse(catalog()) });
    } else if (type === "transcribe") {
      const whisper = await load(data.model, progress);
      progress("Listening back");
      const said = JSON.parse(whisper.transcribe(data.samples, data.language ?? "", data.prompt ?? ""));
      self.postMessage({ id, ok: true, text: said.text, language: said.language });
    } else {
      throw new Error(`unknown message ${type}`);
    }
  } catch (error) {
    self.postMessage({ id, ok: false, error: String(error?.message ?? error) });
  }
};
