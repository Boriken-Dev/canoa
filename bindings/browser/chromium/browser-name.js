// The name a person gives this browser, so they can tell it from their others. Kept here: nothing in the pairing handshake carries
// a name to the host yet. No chrome.* except the storage a caller passes.
const KEY = "browserName";
const LIMIT = 60;

/** What the person typed, as one line without what could reorder or hide text around it; null when nothing is left. */
export function cleanName(value) {
  const name = String(value ?? "").replace(/\p{Cf}|\p{Cc}/gu, " ").replace(/\s+/g, " ").trim().slice(0, LIMIT);
  return name === "" ? null : name;
}

/** "Edge on Windows" from `navigator.userAgentData`, or "This browser" when it says nothing. */
export function defaultBrowserName(data) {
  const brands = (data?.brands ?? []).map((item) => String(item.brand)).filter((brand) => !/^not/i.test(brand));
  const named = brands.find((brand) => brand !== "Chromium") ?? brands[0];
  if (!named) return "This browser";
  const browser = named.replace(/^(Microsoft|Google) /, "");
  return data?.platform ? `${browser} on ${data.platform}` : browser;
}

export async function readBrowserName(storage) {
  const stored = await storage.get(KEY);
  return cleanName(stored?.[KEY]);
}

/** Stores the cleaned name and returns it, or null (and stores nothing) when the person left it empty. */
export async function writeBrowserName(storage, value) {
  const name = cleanName(value);
  if (name !== null) await storage.set({ [KEY]: name });
  return name;
}
