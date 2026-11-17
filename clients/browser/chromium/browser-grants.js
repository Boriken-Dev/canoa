// The person's browser grants, which last as long as the browser does. Each change reads the whole list and writes it back, so they
// go one at a time: otherwise one could bring back what another just revoked.
const KEY = "canoa_grants";

export async function readGrants() {
  const value = (await chrome.storage.session.get(KEY))[KEY];
  return Array.isArray(value) ? value : [];
}

let changes = Promise.resolve();

/** Replaces the grants with what `change` makes of them, and resolves to that. */
export function updateGrants(change) {
  const done = changes.then(async () => {
    const updated = await change(await readGrants());
    await chrome.storage.session.set({ [KEY]: updated });
    return updated;
  });
  changes = done.catch(() => {});
  return done;
}

/** Calls `listen` whenever they change, whoever changed them. */
export function onGrantsChanged(listen) {
  chrome.storage.onChanged.addListener((changed, area) => {
    if (area === "session" && changed[KEY] !== undefined) listen();
  });
}
