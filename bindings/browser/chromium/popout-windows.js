// A session shown in a browser window of its own, beside the side panel. The conversation stays in the panel's page; the window is
// a second view of it over a same-origin channel (app/lib/popout). This opens and closes the window, and when it goes it gives the
// tab binding the window held back to the panel. A window gains no permission: what may be asked of the worker is in tab-sessions.js.
import { PANEL, clientOf, mayAsk, releaseOwner } from "./tab-sessions.js";
import { updateBindings } from "./tab-bindings.js";

const KEY = "canoa_popouts";
const TYPES = new Set(["canoa.popout.open", "canoa.popout.focus", "canoa.popout.close"]);
const WORD = /^[A-Za-z0-9_-]{1,24}$/;

/** The address of a window's page: the link names the channel that only the panel and this window know. */
export function windowUrl(base, { sessionId, link, title, look, density }) {
  if (typeof sessionId !== "string" || sessionId.length === 0 || sessionId.length > 128) throw new TypeError("Invalid session");
  if (typeof link !== "string" || !/^[0-9a-f]{32}$/.test(link)) throw new TypeError("Invalid link");
  if (!WORD.test(look) || !WORD.test(density)) throw new TypeError("Invalid look");
  const url = new URL("ui/index.html", base);
  const cut = String(title ?? "").replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, 80);
  for (const [name, value] of Object.entries({ view: "window", session: sessionId, popout: link, title: cut || "Canoa", look, density })) url.searchParams.set(name, value);
  return url.toString();
}

async function read() {
  const value = (await chrome.storage.session.get(KEY))[KEY];
  return Array.isArray(value) ? value : [];
}

async function handle(message) {
  const held = (await read()).find((record) => record.sessionId === message.sessionId);
  if (message.type === "canoa.popout.open") {
    // One window per session: asking again brings it forward.
    if (held && (await chrome.windows.get(held.windowId).catch(() => null))) {
      await chrome.windows.update(held.windowId, { focused: true });
      return { windowId: held.windowId };
    }
    const url = windowUrl(chrome.runtime.getURL(""), message);
    const made = await chrome.windows.create({ url, type: "popup", width: 560, height: 780 });
    const records = [...(await read()).filter((record) => record.sessionId !== message.sessionId), { sessionId: message.sessionId, windowId: made.id, link: message.link }];
    await chrome.storage.session.set({ [KEY]: records });
    return { windowId: made.id };
  }
  if (!held) return {};
  if (message.type === "canoa.popout.focus") await chrome.windows.update(held.windowId, { focused: true }).catch(() => {});
  else await chrome.windows.remove(held.windowId).catch(() => {});
  return {};
}

export function installPopouts() {
  chrome.runtime.onMessage.addListener((message, sender, reply) => {
    if (!TYPES.has(message?.type)) return false;
    // Opening and closing windows is the panel's: a window asks neither for itself nor for another.
    const client = clientOf(sender, chrome.runtime);
    if (client !== PANEL || !mayAsk(client, message.type)) return false;
    handle(message).then(reply, (error) => reply({ error: String(error.message ?? error) }));
    return true;
  });
  chrome.windows.onRemoved.addListener(async (windowId) => {
    const records = await read();
    const gone = records.find((record) => record.windowId === windowId);
    if (!gone) return;
    await chrome.storage.session.set({ [KEY]: records.filter((record) => record !== gone) });
    await updateBindings((bindings) => releaseOwner(bindings, `window:${gone.link}`));
  });
}
