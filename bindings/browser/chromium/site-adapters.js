/**
 * Validate descriptive manifests for user-installed per-site MCP adapters.
 * This module NEVER evaluates source code or grants site/browser permission.
 */
const runtimes = new Set(["javascript", "wasm"]);
const idPattern = /^[a-z][a-z0-9._-]{2,63}$/;
const versionPattern = /^\d+\.\d+\.\d+(?:-[a-z0-9.-]+)?$/;
const toolPattern = /^[a-zA-Z][a-zA-Z0-9_.-]{0,79}$/;

function originOf(input) {
  if (typeof input !== "string" || input.length > 512) throw new TypeError("Invalid site origin");
  let url;
  try { url = new URL(input); } catch { throw new TypeError("Invalid site origin"); }
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (url.hostname.includes("*")) throw new TypeError("Wildcard origins are not supported");
  if (!(url.protocol === "https:" || (url.protocol === "http:" && local)) ||
      input !== url.origin || url.username || url.password) {
    throw new TypeError("Adapter origins must be exact HTTPS origins or local HTTP");
  }
  return url.origin;
}

function validateEntrypoint(input, runtime) {
  if (typeof input !== "string" || input.length > 200 ||
      !/^[a-zA-Z0-9_./-]+$/.test(input) || input.startsWith("/") ||
      input.split("/").some((part) => !part || part === "." || part === "..") ||
      !input.endsWith(runtime === "javascript" ? ".js" : ".wasm")) {
    throw new TypeError("Adapter entrypoint must be a local relative file");
  }
  return input;
}

/**
 * Normalizes an adapter description; no runtime code or installation happens.
 * Paths may only refer to files the user explicitly installed.
 */
export function validateAdapterManifest(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new TypeError("Invalid adapter manifest");
  }
  if (typeof input.id !== "string" || !idPattern.test(input.id) ||
      typeof input.version !== "string" || !versionPattern.test(input.version) ||
      !runtimes.has(input.runtime)) {
    throw new TypeError("Invalid adapter identity or runtime");
  }
  const entrypoint = validateEntrypoint(input.entrypoint, input.runtime);
  if (!Array.isArray(input.origins) || input.origins.length === 0 ||
      input.origins.length > 32) throw new TypeError("Invalid adapter origin list");
  const origins = [...new Set(input.origins.map(originOf))];
  if (!Array.isArray(input.tools) || input.tools.length === 0 || input.tools.length > 64) {
    throw new TypeError("Invalid adapter tool list");
  }
  const tools = [];
  const names = new Set();
  for (const tool of input.tools) {
    if (!tool || typeof tool !== "object" || Array.isArray(tool) ||
        typeof tool.name !== "string" || !toolPattern.test(tool.name) ||
        names.has(tool.name) || typeof tool.description !== "string" ||
        tool.description.length > 1000 || !tool.inputSchema ||
        typeof tool.inputSchema !== "object" || Array.isArray(tool.inputSchema) ||
        tool.inputSchema.type !== "object") {
      throw new TypeError("Invalid adapter tool declaration");
    }
    names.add(tool.name);
    tools.push({
      name: tool.name,
      description: tool.description,
      inputSchema: structuredClone(tool.inputSchema)
    });
  }
  return {
    id: input.id, version: input.version, runtime: input.runtime,
    entrypoint, origins, tools
  };
}

/** Read-only selection by the committed origin; never a permission grant. */
export function adaptersForOrigin(adapters, origin) {
  if (!Array.isArray(adapters)) return [];
  const normalized = originOf(origin);
  return adapters.filter((adapter) => adapter?.origins?.includes(normalized));
}
