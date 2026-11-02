/**
 * MCP JSON-RPC tools adapter for Canoa's browser tools.
 *
 * This is a service-worker callable handler, NOT an unauthenticated HTTP or
 * chrome.runtime.onMessage server. A transport must first complete coqui-peer
 * authentication and supply its verified identity / trust set. The browser
 * alone owns tab grants, and this handler never creates permissions.
 */
import {TOOLS} from './tools.js';
import {CAPABILITIES} from './browser-tools.js';
import {DIRECTION, isAllowed} from './permissions.js';

const versions = new Set(['2025-06-18', '2025-11-25']);
const LATEST = '2025-11-25';
const REQUEST_ACCESS = 'browser_request_access';
const MAX_RPC_BYTES = 1024 * 1024;

function fail(id, code, message) {
  return {jsonrpc:'2.0', id, error:{code,message}};
}
function result(id, value) {
  return {jsonrpc:'2.0', id, result:value};
}
/** The one tool an agent has before anyone grants it anything: it can ask, and only the person can answer. */
function requestAccessTool(capabilities) {
  return {
    name: REQUEST_ACCESS,
    description: 'Ask the person to let you use one browser tab. They choose the tab, see your reason, and may refuse. If they allow it, your tool list grows and this answers with the tabId.',
    inputSchema: {
      type: 'object',
      properties: {
        capability: {type: 'string', enum: capabilities},
        reason: {type: 'string', description: 'Why you need it, shown to the person as your own words.'},
      },
      required: ['capability', 'reason'],
      additionalProperties: false,
    },
  };
}
export function toolsForPeer({peerId, trustedPeers, grants, now = Date.now()}) {
  if (!(trustedPeers instanceof Set) || !trustedPeers.has(peerId)) {
    return [];
  }
  const usable = new Set();
  for (const item of grants ?? []) {
    if (item.direction !== DIRECTION.AGENT_TO_PAGE || item.peerId !== peerId) continue;
    const permission = {...item, expiresAt:now + 1};
    if (isAllowed({grants, request: permission, trustedPeers, now})) {
      usable.add(item.capability);
    }
  }
  return TOOLS.filter(tool => usable.has(CAPABILITIES[tool.name]));
}
export async function handleMcpRequest({
  request, peerId, trustedPeers, grants, dispatch, now = Date.now(), access,
}) {
  if (!(trustedPeers instanceof Set) || !trustedPeers.has(peerId)) {
    throw Error('Peer authentication required before MCP');
  }
  if (!request || typeof request !== 'object' || Array.isArray(request) ||
      JSON.stringify(request).length > MAX_RPC_BYTES ||
      request.jsonrpc !== '2.0') {
    return fail(request?.id ?? null, -32600, 'Invalid JSON-RPC request');
  }
  const {id, method, params = {}} = request;
  if (typeof method !== 'string' ||
      (!Number.isSafeInteger(id) && typeof id !== 'string' && id !== undefined)) {
    return fail(id ?? null, -32600, 'Invalid request ID or method');
  }
  if (id === undefined) return null; // Only client notifications; never invoke tools.
  if (!params || typeof params !== 'object' || Array.isArray(params)) {
    return fail(id, -32602, 'Invalid parameters');
  }
  if (method === 'initialize') {
    // The client decides whether it can use the version a server answers with, so one it does not know is no error here.
    return result(id, {
      protocolVersion:versions.has(params.protocolVersion) ? params.protocolVersion : LATEST,
      capabilities:{tools:{listChanged:access !== undefined}},
      serverInfo:{name:'dev.boriken.canoa.browser', version:'0.0.1'},
    });
  }
  if (method === 'ping') return result(id, {});
  if (method === 'tools/list') return result(id, {
    tools: [
      ...(access === undefined ? [] : [requestAccessTool(access.capabilities)]),
      ...toolsForPeer({peerId,trustedPeers,grants,now}),
    ],
  });
  if (method !== 'tools/call') return fail(id, -32601, 'MCP method not found');
  const name = params.name, args = params.arguments ?? {};
  if (name === REQUEST_ACCESS && access !== undefined) {
    if (!args || typeof args !== 'object' || Array.isArray(args)) {
      return fail(id, -32602, 'Invalid tool call');
    }
    const answer = await access.request({capability:args.capability, reason:args.reason});
    return result(id, {
      isError:answer.allowed !== true,
      content:[{type:'text',text:JSON.stringify(answer)}],
    });
  }
  if (typeof name !== 'string' || !TOOLS.some(tool => tool.name === name) ||
      !args || typeof args !== 'object' || Array.isArray(args)) {
    return fail(id, -32602, 'Invalid tool call');
  }
  if (!toolsForPeer({peerId,trustedPeers,grants,now}).some(t=>t.name===name)) {
    return result(id, {
      isError:true, content:[{type:'text',text:'Canoa browser permission denied'}],
    });
  }
  try {
    const value = await dispatch({
      name,args,peerId,trustedPeers,grants,now,
    });
    return result(id, {
      content:[{type:'text',text:JSON.stringify(value)}],
      structuredContent:{result:value},
      isError:false,
    });
  } catch (error) {
    return result(id, {
      isError:true,
      content:[{type:'text',text:'Browser tool failed: ' + String(error.message || error)}],
    });
  }
}
