// A stand-in for a coding agent, speaking ACP v1 on its standard streams, so integration_test/coding_task_test.dart can be run
// without spending a subscription. Each prompt of that test gets one fixed answer: tools asked about, a plan, a stop it obeys.
// What it changes on disk is not the point; what reaches the screen is. Its conversations live in the file --state= names.
// A prompt starting `Ask ` asks a question as a form, when the client takes them, shaped as Claude's adapter asks one.
// The fix turn also sends v2's plan_update, tool_call_content_chunk, notice and compaction updates, shaped as schema/v2 has
// them: the app shows them whichever version an agent speaks, and the test records each kind it sees.
// It offers a mode, a model and an effort as config options with their ACP categories, refuses the model `refuse-me` with
// -32602, and notes in its state each session request and prompt in order, under the session's `calls`.
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

// Every kind ACP has, in the order Claude's adapter sends its own. An "always" answer is this agent's to remember: it holds for
// the tool's kind for the rest of the session, which then neither asks nor tells the client.
const options = [
  { optionId: 'yes', name: 'Allow', kind: 'allow_once' },
  { optionId: 'yes-always', name: 'Always allow', kind: 'allow_always' },
  { optionId: 'no', name: 'Reject', kind: 'reject_once' },
  { optionId: 'no-always', name: 'Always reject', kind: 'reject_always' },
];
const remembered = new Map();

// What each session has chosen, by option id.
const chosen = new Map();
const configOptions = (sessionId) => {
  const now = chosen.get(sessionId) ?? { mode: 'ask', model: 'default', effort: 'low' };
  return [
    { id: 'mode', name: 'Mode', category: 'mode', type: 'select', currentValue: now.mode, options: [
      { value: 'ask', name: 'Ask first' }, { value: 'edits', name: 'Accept edits' },
    ] },
    { id: 'model', name: 'Model', category: 'model', type: 'select', currentValue: now.model, options: [
      { value: 'default', name: 'Default model' }, { value: 'fast', name: 'Fast' }, { value: 'refuse-me', name: 'Refused' },
    ] },
    { id: 'effort', name: 'Effort', category: 'thought_level', type: 'select', currentValue: now.effort, options: [
      { value: 'low', name: 'Low' }, { value: 'high', name: 'High' },
    ] },
  ];
};

function note(sessionId, call) {
  const state = load();
  if (!state[sessionId]) return;
  (state[sessionId].calls ??= []).push(call);
  save(state);
}

// Whether the client said it takes questions as forms, in initialize.
let forms = false;

// A question as Claude's adapter shapes its AskUserQuestion tool: one choice with a header, and a free-text "Other" beside it.
function question(sessionId, text) {
  const id = nextId++;
  send({
    id,
    method: 'elicitation/create',
    params: {
      mode: 'form',
      sessionId,
      message: text,
      requestedSchema: {
        type: 'object',
        properties: {
          q0: {
            type: 'string',
            title: 'Runner',
            oneOf: [
              { const: 'node', title: 'node --test', description: 'The runner package.json names' },
              { const: 'npm', title: 'npm test' },
            ],
          },
          q0_custom: { type: 'string', title: 'Other', description: 'Type your own answer, or add a note to the option you chose above (optional).' },
        },
      },
    },
  });
  return new Promise((resolve) => waiting.set(id, (answer) => resolve(answer.result ?? { action: 'cancel' })));
}

function ask(sessionId, toolCall) {
  const always = remembered.get(`${sessionId} ${toolCall.kind}`);
  if (always) return Promise.resolve(always);
  const id = nextId++;
  send({ id, method: 'session/request_permission', params: { sessionId, toolCall, options } });
  return new Promise((resolve) => waiting.set(id, (answer) => {
    const outcome = answer.result?.outcome;
    const chosen = options.find((option) => option.optionId === outcome?.optionId);
    if (outcome?.outcome === 'cancelled' || !chosen) return resolve('cancelled');
    const said = chosen.kind.startsWith('allow') ? 'allowed' : 'refused';
    if (chosen.kind.endsWith('always')) remembered.set(`${sessionId} ${toolCall.kind}`, said);
    resolve(said);
  }));
}

function remember(sessionId, role, text, attached = []) {
  const state = load();
  state[sessionId].history.push({ role, text, attached });
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
  remember(sessionId, 'user', text, attached);
  if (/^Say /.test(text)) {
    say(sessionId, text.slice('Say '.length));
    return 'end_turn';
  }
  if (/^Ask /.test(text)) {
    if (!forms) {
      say(sessionId, 'I cannot ask: this client takes no forms.');
      return 'end_turn';
    }
    const answer = await question(sessionId, text.slice('Ask '.length));
    const chosen = answer.content ?? {};
    say(sessionId, answer.action === 'accept'
      ? `You chose ${chosen.q0 ?? 'nothing'}${chosen.q0_custom ? ` (${chosen.q0_custom})` : ''}.`
      : answer.action === 'decline' ? 'You declined to answer.' : 'The question was cancelled.');
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
    update(sessionId, {
      sessionUpdate: 'tool_call_content_chunk', toolCallId: 'read',
      content: { type: 'content', content: { type: 'text', text: '}' } },
    });
    update(sessionId, { sessionUpdate: 'notice', severity: 'info', title: 'Using the project\'s own test runner' });
    update(sessionId, {
      sessionUpdate: 'compaction_summary_chunk', compactionId: 'c1',
      content: { type: 'text', text: 'calc.js subtracts where it should add.' },
    });
    update(sessionId, { sessionUpdate: 'compaction_update', compactionId: 'c1', status: 'completed' });
    update(sessionId, {
      sessionUpdate: 'plan_update',
      plan: { type: 'items', planId: 'fix', entries: [
        { content: 'Read the code', status: 'completed', priority: 'high' },
        { content: 'Change it', status: 'in_progress', priority: 'high' },
        { content: 'Rewrite the test', status: 'cancelled', priority: 'low' },
        { content: 'Run the tests', status: 'pending', priority: 'medium' },
      ] },
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
    // A client offering v2 says it under capabilities, and v1's under clientCapabilities.
    forms = (params?.capabilities ?? params?.clientCapabilities)?.elicitation?.form !== undefined;
    send({ id, result: {
      protocolVersion: 1,
      agentCapabilities: reopens
        ? { loadSession: true, promptCapabilities: { embeddedContext: true, image: true }, sessionCapabilities: { list: {} } }
        : { promptCapabilities: { embeddedContext: true, image: true } },
      agentInfo,
    } });
  } else if (method === 'session/new') {
    const state = load();
    const sessionId = `scripted-${Object.keys(state).length + 1}`;
    state[sessionId] = { cwd: params.cwd, title: 'Fix the failing test', history: [], updatedAt: new Date().toISOString(),
      calls: ['session/new'] };
    save(state);
    send({ id, result: { sessionId, configOptions: configOptions(sessionId) } });
    update(sessionId, { sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'review', description: 'Review the change' }] });
  } else if (method === 'session/set_config_option') {
    if (params.value === 'refuse-me') {
      send({ id, error: { code: -32602, message: 'That model is not available' } });
    } else {
      chosen.set(params.sessionId, { ...(chosen.get(params.sessionId) ?? { mode: 'ask', model: 'default', effort: 'low' }),
        [params.configId]: params.value });
      note(params.sessionId, `set ${params.configId}=${params.value}`);
      send({ id, result: { configOptions: configOptions(params.sessionId) } });
    }
  } else if (method === 'session/prompt') {
    note(params.sessionId, 'session/prompt');
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
      // What went with a message comes back as its own chunk, a resource with no text of its own.
      for (const uri of said.attached ?? []) {
        update(params.sessionId, { sessionUpdate: 'user_message_chunk', content: { type: 'resource', resource: { uri, text: '' } } });
      }
    }
    send({ id, result: {} });
  } else if (id !== undefined) {
    send({ id, result: {} });
  }
}
