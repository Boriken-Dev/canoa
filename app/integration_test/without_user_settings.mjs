// Runs the ACP agent named after `--` and sets `_meta.claudeCode.options.settingSources` to project and local on each
// session/new, session/load and session/resume, so Claude's adapter starts Claude Code without the person's user-level
// settings, ~/.claude/CLAUDE.md or user rules, while its login still applies. For tests: no preset uses it.
//
//   node without_user_settings.mjs -- npx -y @agentclientprotocol/claude-agent-acp@0.99.1
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

const opening = new Set(['session/new', 'session/load', 'session/resume']);
const [program, ...args] = process.argv.slice(process.argv.indexOf('--') + 1);
// `npx` on Windows is a .cmd, which only a shell starts.
const agent = spawn(program, args, { stdio: ['pipe', 'inherit', 'inherit'], shell: process.platform === 'win32' });
agent.on('exit', (code) => process.exit(code ?? 1));

for await (const line of createInterface({ input: process.stdin })) {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    agent.stdin.write(`${line}\n`);
    continue;
  }
  if (opening.has(message.method) && message.params && typeof message.params === 'object') {
    const meta = (message.params._meta ??= {});
    const claudeCode = (meta.claudeCode ??= {});
    const options = (claudeCode.options ??= {});
    options.settingSources = ['project', 'local'];
  }
  agent.stdin.write(`${JSON.stringify(message)}\n`);
}
agent.stdin.end();
