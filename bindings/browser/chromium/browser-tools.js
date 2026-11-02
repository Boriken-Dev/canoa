import { CAPABILITY, DIRECTION, isAllowed } from "./permissions.js";
import { TOOLS } from "./tools.js";
import { debugCommand, debugDetach, debugEvents } from "./devtools.js";

export const CAPABILITIES = Object.freeze({
  browser_list_tabs: CAPABILITY.READ,
  browser_read_page: CAPABILITY.READ,
  browser_click: CAPABILITY.INTERACT,
  browser_fill: CAPABILITY.INTERACT,
  browser_list_webmcp: CAPABILITY.READ,
  browser_call_webmcp: CAPABILITY.WEBMCP,
  browser_discover_mcp: CAPABILITY.DISCOVER,
  browser_debug_command: CAPABILITY.DEBUG,
  browser_debug_detach: CAPABILITY.DEBUG,
  browser_debug_events: CAPABILITY.DEBUG
});

const SUPPORTED = new Set(TOOLS.map((tool) => tool.name));
const DENIED = "Canoa browser permission denied";

/**
 * What a click or a fill may name: ids, classes and element names, as `browser_read_page` gives them. An attribute selector or `:has`
 * would answer yes or no about a page's contents, so a grant to interact would become a grant to read, one character at a time.
 */
export function plainSelector(selector) {
  if (typeof selector !== "string" || selector.length > 500) return false;
  // CSS.escape's escapes, which an id with odd characters in it needs.
  return /^[A-Za-z0-9_\-#. >+~,*]+$/.test(selector.replace(/\\./g, ""));
}

/**
 * Runs only inside the extension service worker, after peer authentication.
 * The caller MUST supply trustedPeers from a cryptographically verified
 * connection; accepting peer IDs from website messages would be unsafe.
 */
export async function dispatchBrowserTool({ name, args = {}, peerId, trustedPeers, grants, now = Date.now() }) {
  if (!SUPPORTED.has(name)) throw new Error("Unknown browser tool");
  if (!args || typeof args !== "object" || Array.isArray(args)) throw new Error("Invalid tool arguments");
  if (!(trustedPeers instanceof Set) || !trustedPeers.has(peerId)) throw new Error("Peer not verified");

  // A tab that does not exist reads the same as one not allowed, so asking says nothing about what is open.
  const tabs = name === "browser_list_tabs"
    ? await chrome.tabs.query({})
    : [await chrome.tabs.get(args.tabId).catch(() => {
      throw new Error(DENIED);
    })];

  const results = [];
  for (const tab of tabs) {
    if (!Number.isSafeInteger(tab.id) || !tab.url || !/^https?:\/\//.test(tab.url)) continue;
    const origin = new URL(tab.url).origin;
    const request = {
      direction: DIRECTION.AGENT_TO_PAGE, peerId, origin,
      capability: CAPABILITIES[name], tabId: tab.id, expiresAt: now + 1
    };
    if (!isAllowed({ grants, request, trustedPeers, now })) {
      if (name === "browser_list_tabs") continue;
      throw new Error(DENIED);
    }

    if (name === "browser_list_tabs") {
      results.push({ tabId: tab.id, title: tab.title ?? "", url: tab.url });
      continue;
    }

    if (name === "browser_debug_command") {
      return debugCommand({tabId: tab.id, method: args.method, params: args.params ?? {}});
    }
    if (name === "browser_debug_events") {
      return debugEvents({tabId: tab.id});
    }
    if (name === "browser_debug_detach") {
      return debugDetach({tabId: tab.id});
    }

    if (name === "browser_read_page") {
      const page = await inject(tab.id, readPage, [], "ISOLATED");
      // The tab can have moved on since it was checked, and only what it was checked for may go back.
      if (new URL(page?.url ?? "about:blank").origin !== origin) throw new Error(DENIED);
      return page;
    }
    if ((name === "browser_click" || name === "browser_fill") && !plainSelector(args.selector)) {
      throw new Error("Use a selector made of ids, classes and element names");
    }
    if (name === "browser_click") return inject(tab.id, clickElement, [args.selector], "ISOLATED");
    if (name === "browser_fill") return inject(tab.id, fillElement, [args.selector, args.text], "ISOLATED");
    if (name === "browser_list_webmcp") return inject(tab.id, listWebMcp, [], "MAIN");
    if (name === "browser_call_webmcp") {
      return inject(tab.id, callWebMcp, [args.name, args.arguments ?? {}], "MAIN");
    }
    if (name === "browser_discover_mcp") return inject(tab.id, discoverMcp, [], "ISOLATED");
  }
  if (name === "browser_list_tabs") return results;
  throw new Error(DENIED);
}

async function inject(tabId, func, args, world) {
  const result = await chrome.scripting.executeScript({
    target: { tabId, allFrames: false }, func, args, world
  });
  return result[0]?.result ?? null;
}

function readPage() {
  const visible = (element) =>
    element.getClientRects().length !== 0 &&
    getComputedStyle(element).visibility !== "hidden";
  const interactive = Array.from(document.querySelectorAll(
    "a,button,input,select,textarea,[role=button]"
  )).filter(visible).slice(0, 100);
  return {
    title: document.title.slice(0, 500),
    url: location.href,
    text: (document.body?.innerText ?? "").slice(0, 16000),
    elements: interactive.map((el) => ({
      tag: el.localName,
      selector: el.id && el.id.length <= 200 ? "#" + CSS.escape(el.id) : null,
      label: (el.getAttribute("aria-label") || el.innerText ||
        el.getAttribute("placeholder") || "").slice(0, 120)
    }))
  };
}

function clickElement(selector) {
  if (typeof selector !== "string" || selector.length > 500) throw Error("Invalid selector");
  const element = document.querySelector(selector);
  if (!element || element.getClientRects().length === 0 ||
      getComputedStyle(element).visibility === "hidden" || element.disabled) {
    throw Error("Element not actionable");
  }
  element.click();
  return { clicked: true };
}

function fillElement(selector, value) {
  if (typeof selector !== "string" || selector.length > 500 ||
      typeof value !== "string" || value.length > 10000) throw Error("Invalid input");
  const element = document.querySelector(selector);
  if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) ||
      element.getClientRects().length === 0 || element.disabled || element.readOnly ||
      getComputedStyle(element).visibility === "hidden") {
    throw Error("Input not editable");
  }
  if (element instanceof HTMLInputElement &&
      !["text", "search", "email", "url", "tel", "number"].includes(element.type)) {
    throw Error("Input type not permitted");
  }
  // Update a normal DOM value; some frameworks may need a separate adapter.
  element.value = value;
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
  return { filled: true };
}

async function listWebMcp() {
  if (!document.modelContext?.getTools) return { available: false, tools: [] };
  const tools = await document.modelContext.getTools();
  return {
    available: true,
    tools: tools.slice(0, 100).map((tool) => ({
      name: tool.name, description: tool.description ?? "",
      inputSchema: tool.inputSchema ?? { type: "object" },
      origin: tool.origin
    }))
  };
}

async function callWebMcp(name, input) {
  if (typeof name !== "string" || name.length > 150 ||
      !input || typeof input !== "object" || Array.isArray(input)) throw Error("Invalid tool call");
  if (!document.modelContext?.getTools || !document.modelContext?.executeTool) {
    throw Error("WebMCP not supported by this browser");
  }
  const tools = await document.modelContext.getTools();
  const selected = tools.find((tool) => tool.name === name);
  if (!selected) throw Error("WebMCP tool unavailable");
  return await document.modelContext.executeTool(selected, input);
}

function discoverMcp() {
  // Canoa-specific convention. This does not verify or contact any endpoint.
  return Array.from(document.querySelectorAll('link[rel~="mcp"][href]'))
    .slice(0, 25).map((link) => {
      try {
        const url = new URL(link.href, location.href);
        if (url.protocol !== "https:" &&
            !(url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname))) {
          return null;
        }
        return { url: url.href, title: link.title ?? "" };
      } catch {
        return null;
      }
    }).filter(Boolean);
}
