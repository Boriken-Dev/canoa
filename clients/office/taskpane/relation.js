// What an open Office item relates a chat to: a mail conversation, or a document. Pure: it reads plain values the task pane
// took from Office.js and imports nothing from it, so it runs under `node --test`.
//
// The app keeps the result beside each session it starts and lists the sessions with the same relation first. The key is a hash
// of the item's identity, so the app's session list holds no address, subject or path in it; the label is what the person calls the
// item in that list. Neither is sent to the agent.

/** The kinds the app's `Relation` names for an Office task pane. */
export const CONVERSATION = 'conversation';
export const DOCUMENT = 'document';

/** The page of the chat-only app, under this add-in's own route, and the parameters that ask for the chat alone. */
export const APP_PATH = '/office/app/';
/** Reverse-DNS identifier of Canoa's browser provider in `document.acp.providers`. */
export const EXTENSION_PROVIDER_RDNS = 'dev.boriken.canoa';
/** Window property carrying the extension's Office tool registration bridge (never attached to `document.acp`). */
export const OFFICE_TOOLS_SYMBOL = Symbol.for('dev.boriken.canoa.officeTools');

/**
 * Whether `document.acp` offers Canoa's browser extension provider (by RDNS/UUID, never sniffing extension ids).
 * @param {{acp?: {providers?: Array<{rdns?: string, uuid?: string}>}}|null|undefined} doc
 * @returns {boolean}
 */
export function hasExtensionProvider(doc) {
  const providers = doc?.acp?.providers;
  if (!Array.isArray(providers)) return false;
  return providers.some((provider) => provider?.rdns === EXTENSION_PROVIDER_RDNS || provider?.uuid === EXTENSION_PROVIDER_RDNS);
}

/**
 * Waits briefly for `acp:ready` when `document.acp` is not yet registered at load, then reports whether Canoa is offered.
 * @param {Window|null|undefined} win
 * @param {{waitMs?: number}} [options]
 * @returns {Promise<boolean>}
 */
export async function detectExtensionProvider(win, { waitMs = 150 } = {}) {
  if (!win) return false;
  if (hasExtensionProvider(win.document)) return true;
  if (typeof win.addEventListener !== 'function') return false;
  return new Promise((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      win.removeEventListener?.('acp:ready', check);
      resolve(hasExtensionProvider(win.document));
    };
    const check = () => {
      if (hasExtensionProvider(win.document)) finish();
    };
    const timer = setTimeout(finish, waitMs);
    win.addEventListener('acp:ready', check);
    try {
      if (typeof Event === 'function' && typeof win.dispatchEvent === 'function') {
        win.dispatchEvent(new Event('acp:request'));
      }
    } catch {
      // Older environments without custom Event constructors simply rely on the timeout.
    }
  });
}

/**
 * Returns the extension's Office tools registration bridge when `acp-page.js` defined it on `win`, else null.
 * @param {Window|null|undefined} win
 * @returns {{register: (app: string, handler: Function) => Promise<unknown>, unregister: (app: string) => Promise<void>, web: () => Promise<string|null>}|null}
 */
export function createExtensionToolsBridge(win) {
  const bridge = win?.[OFFICE_TOOLS_SYMBOL];
  if (!bridge || typeof bridge.register !== 'function' || typeof bridge.unregister !== 'function') return null;
  return {
    register: (app, handler) => bridge.register(app, handler),
    unregister: (app) => bridge.unregister(app),
    web: () => (typeof bridge.web === 'function' ? bridge.web() : Promise.resolve(null)),
  };
}

const LABEL_MAX = 80;
const KEY_HEX = 32;

/**
 * The key a relation is stored under: the first 128 bits of SHA-256 over the kind and the item's identity, in hexadecimal.
 * The same input gives the same key on every call and in every client of the same mailbox or document.
 * @param {string} kind
 * @param {string} identity
 * @returns {Promise<string>}
 */
export async function hashKey(kind, identity) {
  const bytes = new TextEncoder().encode(`${kind}\n${identity}`);
  const digest = await globalThis.crypto.subtle.digest('SHA-256', bytes);
  let hex = '';
  for (const byte of new Uint8Array(digest)) hex += byte.toString(16).padStart(2, '0');
  return hex.slice(0, KEY_HEX);
}

/** Whether `value` is a non-blank string. */
function present(value) {
  return typeof value === 'string' && value.trim() !== '';
}

/** `text` on one line, trimmed and cut to what a list shows; undefined when nothing is left. */
function tidy(text) {
  if (!present(text)) return undefined;
  const line = text.replace(/\s+/g, ' ').trim();
  return line.length > LABEL_MAX ? `${line.slice(0, LABEL_MAX - 1)}…` : line;
}

/**
 * The relation of an open mail item, or null when it has none: a new message in compose has no conversation id until it has a
 * subject and is saved, and `item.conversationId` is null or empty then. A reply whose subject the person changes gets a new
 * id, and so a new relation.
 * @param {{conversationId?: string|null, subject?: string|null}} item
 * @returns {Promise<{kind: string, key: string, label?: string}|null>}
 */
export async function relationForMail(item) {
  if (!item || !present(item.conversationId)) return null;
  const relation = { kind: CONVERSATION, key: await hashKey(CONVERSATION, item.conversationId.trim()) };
  const label = tidy(item.subject);
  return label === undefined ? relation : { ...relation, label };
}

/**
 * `url` as one string a document is known by: its fragment left out, since a fragment is a place inside the document, and its
 * scheme and host in lower case. A query is kept: a SharePoint address names its document there, and two openings that differ
 * only in a query are then not related, which is the safe way to be wrong. A path that is not a URL (a local file's) is kept as
 * written.
 * @param {string} url
 * @returns {string}
 */
export function normalizeDocumentUrl(url) {
  const text = url.trim();
  try {
    const parsed = new URL(text);
    parsed.hash = '';
    return parsed.href;
  } catch {
    return text;
  }
}

/**
 * What the person calls a document: the last part of its address, decoded; undefined for one with none.
 * @param {string} url
 * @returns {string|undefined}
 */
export function documentName(url) {
  let path = url.trim();
  try {
    path = new URL(path).pathname;
  } catch {
    // A local path, read as it is.
  }
  const last = path.split(/[\\/]/).filter((part) => part !== '').pop();
  if (last === undefined) return undefined;
  try {
    return tidy(decodeURIComponent(last));
  } catch {
    return tidy(last);
  }
}

/**
 * The relation of the open document, or null when it has none: `Office.context.document.url` is null where the address is
 * unavailable and empty for a file not saved yet. `app` names the Office application (`word`, `excel`, `powerpoint`), since a
 * `.docx` and an `.xlsx` never share an address but a pane might be shown for either.
 * @param {{app?: string, url?: string|null}} document
 * @returns {Promise<{kind: string, key: string, label?: string}|null>}
 */
export async function relationForDocument(document) {
  if (!document || !present(document.url)) return null;
  const app = present(document.app) ? document.app.trim().toLowerCase() : 'office';
  const relation = {
    kind: DOCUMENT,
    key: await hashKey(DOCUMENT, `${app}\n${normalizeDocumentUrl(document.url)}`),
  };
  const label = documentName(document.url);
  return label === undefined ? relation : { ...relation, label };
}

/**
 * Whether two relations name the same thing: the same kind and key. A label does not count.
 * @param {{kind: string, key: string}|null|undefined} a
 * @param {{kind: string, key: string}|null|undefined} b
 * @returns {boolean}
 */
export function sameRelation(a, b) {
  if (!a || !b) return !a && !b;
  return a.kind === b.kind && a.key === b.key;
}

/**
 * The address of the chat-only app for `relation` (or for none), on `origin`: `mode=chat`, and `relation=<kind>:<key>` with
 * `label=` as the app's `ClientMode` reads them. `client` is the kind of Office application the pane is in (`outlook`, `word`,
 * `excel` or `powerpoint`), which canoa.cfg may give a default loadout of its own; a Word and an Excel pane relate to documents
 * alike, so only this tells them apart.
 * @param {string} origin
 * @param {{kind: string, key: string, label?: string}|null} relation
 * @param {string} [client]
 * @param {string} [key]  the home's secret, when the manifest carries one: the app sends it with its request to be admitted
 * @returns {string}
 */
export function appAddress(origin, relation, client, key) {
  const address = new URL(APP_PATH, origin);
  address.searchParams.set('mode', 'chat');
  if (client) address.searchParams.set('client', client);
  if (key) address.searchParams.set('office_key', key);
  if (relation) {
    address.searchParams.set('relation', `${relation.kind}:${relation.key}`);
    if (relation.label) address.searchParams.set('label', relation.label);
  }
  return address.href;
}

/**
 * The message that tells the framed app what is in front of the client now, as a string a frame of the same origin receives with
 * `postMessage`: `{"canoa":"relation","relation":{kind,key,scope?,label?}|null}`. The app reads it without a reload.
 * @param {{kind: string, key: string, scope?: string, label?: string}|null} relation
 * @returns {string}
 */
export function relationMessage(relation) {
  const body = relation === null || relation === undefined ? null : { kind: relation.kind, key: relation.key };
  if (body && relation.scope) body.scope = relation.scope;
  if (body && relation.label) body.label = relation.label;
  return JSON.stringify({ canoa: 'relation', relation: body });
}

/**
 * What the framed app says to the pane: `ready` once it listens for a relation, `signin` when its session ended. Anything else,
 * and anything not a string, is nothing.
 * @param {unknown} data
 * @returns {'ready'|'signin'|null}
 */
export function parsePaneMessage(data) {
  if (typeof data !== 'string' || data.length > 256) return null;
  let value;
  try {
    value = JSON.parse(data);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value.canoa === 'ready' || value.canoa === 'signin' ? value.canoa : null;
}

/**
 * Whether the host the pane came from keeps a sign-in, from the answer to `GET /office/session`: the portal answers JSON, 200
 * for a signed-in browser and 401 for one that is not. Any other answer (the site's own page, a 404) is a host with no sign-in,
 * and the chat is shown as it is.
 * @param {number} status
 * @param {string|null|undefined} contentType
 * @returns {'signed-in'|'signed-out'|'open'}
 */
export function portalState(status, contentType) {
  const json = typeof contentType === 'string' && /^application\/json\b/i.test(contentType.trim());
  if (!json) return 'open';
  if (status === 200) return 'signed-in';
  if (status === 401) return 'signed-out';
  return 'open';
}

/** The longest string the dialog may send; the portal's are far shorter. */
const HANDOFF_MAX = 128;

/**
 * The string a sign-in in the dialog sent, when it has the shape the portal makes (URL-safe base64): anything else is not sent
 * on to the portal.
 * @param {unknown} message
 * @returns {string|null}
 */
export function handoffFrom(message) {
  return typeof message === 'string' && message.length >= 20 && message.length <= HANDOFF_MAX && /^[A-Za-z0-9_-]+$/.test(message)
    ? message
    : null;
}
