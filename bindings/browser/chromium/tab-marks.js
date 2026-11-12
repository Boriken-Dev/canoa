// What the person sees of a tab an agent can reach: a tab group named for the agent around a tab it acts on, and a badge on the
// toolbar icon over a tab shared with a session. Nothing is drawn inside a page. The rules are in tab-sessions.js.
import { onGrantsChanged, readGrants } from "./browser-grants.js";
import { GROUP_COLOR, groupOf, groupTitle, noteActing, tabsToRelease } from "./tab-sessions.js";

const ACTING = "canoa_acting_tabs";
const BADGED = "canoa_badged_tabs";

const read = async (key) => {
  const value = (await chrome.storage.session.get(key))[key];
  return Array.isArray(value) ? value : [];
};
const write = (key, value) => chrome.storage.session.set({ [key]: value });

let changes = Promise.resolve();
const serial = (work) => {
  const done = changes.then(work);
  changes = done.catch(() => {});
  return done;
};

/** A tool ran on this tab for this agent: the tab joins the agent's group, unless the person has it in a group of their own. */
export function markActing({ tabId, label }) {
  return serial(async () => {
    const acting = await read(ACTING);
    if (acting.some((entry) => entry.tabId === tabId)) return;
    const tab = await chrome.tabs.get(tabId);
    if (tab.groupId !== undefined && tab.groupId !== chrome.tabGroups.TAB_GROUP_ID_NONE) return;
    const existing = groupOf(acting, label, tab.windowId);
    const options = { tabIds: [tabId] };
    if (existing !== undefined) options.groupId = existing;
    else options.createProperties = { windowId: tab.windowId };
    const groupId = await chrome.tabs.group(options);
    await chrome.tabGroups.update(groupId, { title: groupTitle(label), color: GROUP_COLOR });
    await write(ACTING, noteActing(acting, { tabId, label, groupId, windowId: tab.windowId }));
  });
}

async function refresh() {
  const grants = await readGrants();
  const acting = await read(ACTING);
  const gone = tabsToRelease(acting, grants);
  for (const entry of gone) await chrome.tabs.ungroup(entry.tabId).catch(() => {});
  await write(ACTING, acting.filter((entry) => !gone.includes(entry)));

  const shared = new Set(grants.filter((grant) => typeof grant.session === "string").map((grant) => grant.tabId));
  const before = await read(BADGED);
  for (const tabId of before.filter((id) => !shared.has(id))) await chrome.action.setBadgeText({ tabId, text: "" }).catch(() => {});
  for (const tabId of shared) await chrome.action.setBadgeText({ tabId, text: "•" }).catch(() => {});
  await write(BADGED, [...shared]);
}

export function installMarks() {
  onGrantsChanged(() => serial(refresh).catch(() => {}));
  chrome.tabs.onRemoved.addListener((tabId) => serial(async () => {
    await write(ACTING, (await read(ACTING)).filter((entry) => entry.tabId !== tabId));
    await write(BADGED, (await read(BADGED)).filter((id) => id !== tabId));
  }).catch(() => {}));
}
