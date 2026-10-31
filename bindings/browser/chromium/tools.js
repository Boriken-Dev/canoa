// MCP-shaped tool descriptions for the future authenticated MCP adapter.
// This catalogue is not itself an MCP transport or an accessible web API.
export const TOOLS = Object.freeze([
  {
    name: "browser_list_tabs",
    description: "List only browser tabs this verified peer is permitted to read.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
    annotations: { readOnlyHint: true }
  },
  {
    name: "browser_read_page",
    description: "Return bounded page text and actionable element labels for an approved tab.",
    inputSchema: {
      type: "object", properties: { tabId: { type: "integer" } },
      required: ["tabId"], additionalProperties: false
    },
    annotations: { readOnlyHint: true }
  },
  {
    name: "browser_click",
    description: "Click a visible enabled element in an approved tab.",
    inputSchema: {
      type: "object",
      properties: { tabId: { type: "integer" }, selector: { type: "string" } },
      required: ["tabId", "selector"], additionalProperties: false
    }
  },
  {
    name: "browser_fill",
    description: "Fill a visible non-secret text input in an approved tab without submitting it.",
    inputSchema: {
      type: "object",
      properties: {
        tabId: { type: "integer" },
        selector: { type: "string" },
        text: { type: "string" }
      },
      required: ["tabId", "selector", "text"], additionalProperties: false
    }
  },
  {
    name: "browser_list_webmcp",
    description: "Discover tools registered by the page through the experimental WebMCP API.",
    inputSchema: {
      type: "object", properties: { tabId: { type: "integer" } },
      required: ["tabId"], additionalProperties: false
    },
    annotations: { readOnlyHint: true }
  },
  {
    name: "browser_call_webmcp",
    description: "Invoke a specific WebMCP tool on an explicitly interaction-approved tab.",
    inputSchema: {
      type: "object",
      properties: {
        tabId: { type: "integer" },
        name: { type: "string" },
        arguments: { type: "object" }
      },
      required: ["tabId", "name"], additionalProperties: false
    }
  },
  {
    name: "browser_discover_mcp",
    description: "List MCP endpoint hints advertised by the approved page; never connect automatically.",
    inputSchema: {
      type: "object", properties: { tabId: { type: "integer" } },
      required: ["tabId"], additionalProperties: false
    },
    annotations: { readOnlyHint: true }
  }
]);
