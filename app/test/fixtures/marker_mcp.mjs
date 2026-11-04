// A stdio MCP server with one tool, `marker`, which answers the value of MARKER_WORD from its environment and writes
// `called` beside this script, so a test can tell the tool was called and the environment reached it. For tests only.
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const called = join(dirname(fileURLToPath(import.meta.url)), 'called');
const send = (message) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`);

for await (const line of createInterface({ input: process.stdin })) {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    continue;
  }
  const { id, method, params } = message;
  if (id === undefined) continue;
  switch (method) {
    case 'initialize':
      send({ id, result: {
        protocolVersion: params?.protocolVersion ?? '2025-06-18',
        capabilities: { tools: {} },
        serverInfo: { name: 'marker', version: '0.0.0' },
      } });
      break;
    case 'ping':
      send({ id, result: {} });
      break;
    case 'tools/list':
      send({ id, result: { tools: [{
        name: 'marker',
        description: 'Returns the marker word.',
        inputSchema: { type: 'object', properties: {} },
      }] } });
      break;
    case 'tools/call':
      writeFileSync(called, `${params?.name ?? ''}\n`, { flag: 'a' });
      send({ id, result: { content: [{ type: 'text', text: process.env.MARKER_WORD ?? '(MARKER_WORD is not set)' }] } });
      break;
    default:
      send({ id, error: { code: -32601, message: 'Method not found' } });
  }
}
