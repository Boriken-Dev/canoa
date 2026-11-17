/** User-initiated page context, not a permission grant or remote agent call. */
export function captureContext(info, tab, now = Date.now()) {
  if (!info || !tab || !Number.isSafeInteger(tab.id) || tab.id < 0) return null;
  const raw = info.pageUrl || tab.url;
  if (typeof raw !== "string" || raw.length > 8192) return null;
  let url;
  try { url = new URL(raw); } catch { return null; }
  if (!["http:", "https:"].includes(url.protocol)) return null;
  // Do not automatically forward query tokens, fragments or embedded credentials.
  url.search = "";
  url.hash = "";
  url.username = "";
  url.password = "";
  return {
    tabId: tab.id,
    url: url.href,
    origin: url.origin,
    title: String(tab.title || "").slice(0, 250),
    selection: String(info.selectionText || "").slice(0, 8000),
    capturedAt: now
  };
}

export function addContext(queue, context) {
  if (!context) return Array.isArray(queue) ? queue : [];
  return [...(Array.isArray(queue) ? queue : []), context].slice(-10);
}

export function takeContext(queue) {
  const items = Array.isArray(queue) ? queue : [];
  return { context: items[0] ?? null, remaining: items.slice(1) };
}
