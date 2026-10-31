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
const MAX_RPC_BYTES = 1024 * 1024;

function fail(id, code, message) {
  return {jsonrpc:'2.0', id, error:{code,message}};
}
function result(id, value) {
  return {jsonrpc:'2.0', id, result:value};
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
  request, peerId, trustedPeers, grants, dispatch, now = Date.now(),
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
    if (!versions.has(params.protocolVersion)) {
      return fail(id, -32602, 'Unsupported MCP protocol version');
    }
    return result(id, {
      protocolVersion:params.protocolVersion,
      capabilities:{tools:{listChanged:false}},
      serverInfo:{name:'dev.boriken.canoa.browser', version:'0.0.1'},
    });
  }
  if (method === 'ping') return result(id, {});
  if (method === 'tools/list') return result(id, {
    tools: toolsForPeer({peerId,trustedPeers,grants,now}),
  });
  if (method !== 'tools/call') return fail(id, -32601, 'MCP method not found');
  const name = params.name, args = params.arguments ?? {};
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
