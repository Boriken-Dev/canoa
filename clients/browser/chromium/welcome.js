// Closes this tab; the page shows once, on install, and is reachable from the permissions page after.
document.querySelector("#ok").addEventListener("click", async () => {
  const tab = await chrome.tabs.getCurrent();
  if (tab?.id !== undefined) await chrome.tabs.remove(tab.id);
  else window.close();
});
