// A stand-in for a coding agent, speaking ACP v1 on its standard streams, so integration_test/coding_task_test.dart can be run
// without spending a subscription. Each prompt of that test gets one fixed answer: tools asked about, a plan, a stop it obeys.
// What it changes on disk is not the point; what reaches the screen is. Its conversations live in the file --state= names.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

const option = (name) => process.argv.find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3);
const statePath = option('state');
// What it says of itself, so one script can stand in for several harnesses: `agentInfo` only when named, and with
// --no-reopen neither session/load nor session/list.
const agentInfo = option('name') ? { name: option('name'), version: option('version') ?? '0.0.0' } : undefined;
const reopens = !process.argv.includes('--no-reopen');
const load = () => (statePath && existsSync(statePath) ? JSON.parse(readFileSync(statePath, 'utf8')) : {});
const save = (state) => statePath && writeFileSync(statePath, JSON.stringify(state));

let nextId = 1000;
const waiting = new Map();
let turn = null;

const send = (message) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`);
const update = (sessionId, body) => send({ method: 'session/update', params: { sessionId, update: body } });

function ask(sessionId, toolCall) {
  const id = nextId++;
  send({
    id,
    method: 'session/request_permission',
    params: {
      sessionId,
      toolCall,
      options: [
        { optionId: 'yes', name: 'Allow', kind: 'allow_once' },
        { optionId: 'no', name: 'Reject', kind: 'reject_once' },
      ],
    },
  });
  return new Promise((resolve) => waiting.set(id, (answer) => {
    const outcome = answer.result?.outcome;
    resolve(outcome?.outcome === 'cancelled' ? 'cancelled' : outcome?.optionId === 'yes' ? 'allowed' : 'refused');
  }));
}

function remember(sessionId, role, text) {
  const state = load();
  state[sessionId].history.push({ role, text });
  state[sessionId].updatedAt = new Date().toISOString();
  save(state);
}

function say(sessionId, text) {
  update(sessionId, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text } });
  remember(sessionId, 'agent', text);
}

async function tool(sessionId, id, title, kind, rawInput, done) {
  update(sessionId, { sessionUpdate: 'tool_call', toolCallId: id, title, kind, status: 'pending', rawInput });
  const answer = await ask(sessionId, { toolCallId: id, title, kind, rawInput });
  if (answer === 'allowed') update(sessionId, { sessionUpdate: 'tool_call_update', toolCallId: id, status: 'completed', content: done });
  else update(sessionId, { sessionUpdate: 'tool_call_update', toolCallId: id, status: 'failed' });
  return answer;
}

async function prompt(sessionId, blocks) {
  const text = blocks.filter((block) => block.type === 'text').map((block) => block.text).join(' ');
  const attached = blocks.filter((block) => block.type === 'resource').map((block) => block.resource?.uri ?? 'a file');
  remember(sessionId, 'user', text);
  if (/^Say /.test(text)) {
    say(sessionId, text.slice('Say '.length));
    return 'end_turn';
  }
  update(sessionId, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'Looking at the repository.' } });
  update(sessionId, {
    sessionUpdate: 'plan',
    entries: [
      { content: 'Read the code', status: 'completed', priority: 'high' },
      { content: 'Change it', status: 'in_progress', priority: 'high' },
      { content: 'Run the tests', status: 'pending', priority: 'medium' },
    ],
  });
  if (attached.length > 0) {
    say(sessionId, `Following ${attached.join(', ')}.`);
    await tool(sessionId, 'readme', 'Edit README.md', 'edit', { file_path: 'README.md' },
      [{ type: 'diff', path: 'README.md', oldText: '# tiny\n', newText: '# tiny\nMaintainer: Ana Rivera\n' }]);
    const answer = await tool(sessionId, 'rm', 'rm obsolete.txt', 'execute', { command: 'rm obsolete.txt' },
      [{ type: 'content', content: { type: 'text', text: '' } }]);
    if (answer === 'cancelled') return 'cancelled';
    say(sessionId, answer === 'allowed' ? 'Deleted obsolete.txt.' : 'You refused, so obsolete.txt is still there.');
  } else if (/fails/.test(text)) {
    update(sessionId, {
      sessionUpdate: 'tool_call', toolCallId: 'read', title: 'Read src/calc.js', kind: 'read', status: 'completed',
      locations: [{ path: 'src/calc.js', line: 1 }],
      content: [{ type: 'content', content: { type: 'text', text: 'return a - b;' } }],
    });
    await tool(sessionId, 'fix', 'Edit src/calc.js', 'edit', { file_path: 'src/calc.js' },
      [{ type: 'diff', path: 'src/calc.js', oldText: 'return a - b;\n', newText: 'return a + b;\n' }]);
    await tool(sessionId, 'tests', 'node --test', 'execute', { command: 'node --test' },
      [{ type: 'content', content: { type: 'text', text: '# pass 2\n# fail 0' } }]);
    say(sessionId, 'Fixed: it subtracted. Two tests pass.');
  } else if (/^Rename/.test(text)) {
    const answer = await tool(sessionId, 'rename', 'Edit src/calc.js', 'edit', { file_path: 'src/calc.js' },
      [{ type: 'diff', path: 'src/calc.js', oldText: 'addNumbers', newText: 'sum' }]);
    if (answer === 'cancelled') return 'cancelled';
  } else {
    await tool(sessionId, 'rename2', 'Edit src/report.js', 'edit', { file_path: 'src/report.js' },
      [{ type: 'diff', path: 'src/report.js', oldText: 'addNumbers', newText: 'sum' }]);
    say(sessionId, 'Renamed everywhere.');
  }
  update(sessionId, { sessionUpdate: 'usage_update', used: 1200, size: 200000, cost: { amount: 0.01, currency: 'USD' } });
  return 'end_turn';
}

for await (const line of createInterface({ input: process.stdin })) {
  const message = JSON.parse(line);
  const { id, method, params } = message;
  if (method === undefined) {
    waiting.get(id)?.(message);
    waiting.delete(id);
  } else if (method === 'initialize') {
    send({ id, result: {
      protocolVersion: 1,
      agentCapabilities: reopens
        ? { loadSession: true, promptCapabilities: { embeddedContext: true }, sessionCapabilities: { list: {} } }
        : { promptCapabilities: { embeddedContext: true } },
      agentInfo,
    } });
  } else if (method === 'session/new') {
    const state = load();
    const sessionId = `scripted-${Object.keys(state).length + 1}`;
    state[sessionId] = { cwd: params.cwd, title: 'Fix the failing test', history: [], updatedAt: new Date().toISOString() };
    save(state);
    send({ id, result: { sessionId } });
    update(sessionId, { sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'review', description: 'Review the change' }] });
  } else if (method === 'session/prompt') {
    turn = { id, sessionId: params.sessionId };
    prompt(params.sessionId, params.prompt).then((stopReason) => {
      if (turn?.id === id) send({ id, result: { stopReason } });
      turn = null;
    });
  } else if (method === 'session/cancel') {
    if (turn) {
      send({ id: turn.id, result: { stopReason: 'cancelled' } });
      turn = null;
    }
  } else if (method === 'session/list') {
    const state = load();
    send({ id, result: { sessions: Object.entries(state)
      .filter(([, session]) => session.history.length > 0 && (!params.cwd || session.cwd === params.cwd))
      .map(([sessionId, session]) => ({ sessionId, cwd: session.cwd, title: session.title, updatedAt: session.updatedAt })) } });
  } else if (method === 'session/load') {
    const session = load()[params.sessionId];
    for (const said of session?.history ?? []) {
      update(params.sessionId, {
        sessionUpdate: said.role === 'user' ? 'user_message_chunk' : 'agent_message_chunk',
        content: { type: 'text', text: said.text },
      });
    }
    send({ id, result: {} });
  } else if (id !== undefined) {
    send({ id, result: {} });
  }
}
