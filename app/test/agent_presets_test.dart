// Claude and Codex run through the host this app carries, with no Node: what the presets say, how they are started, what
// a source check looks for, and the harness their agent names itself as.
import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/apply.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter_test/flutter_test.dart';

AgentCommand _preset(String label) => agentPresets.firstWhere((preset) => preset.label == label);

void main() {
  test('Claude and Codex are the host\'s own agent command, and need only their own tool', () {
    for (final (label, harness) in [('Claude', 'claude'), ('Codex', 'codex')]) {
      final preset = _preset(label);
      expect(preset.hosts, isTrue, reason: label);
      expect(preset.args.take(2), ['agent', harness], reason: label);
      expect([preset.program, ...preset.args], isNot(contains('npx')), reason: label);
      expect(preset.requires, [harness], reason: label);
      expect(commandNeeds(preset), [harness], reason: '$label needs no program of its own on PATH');
      expect(preset.programPaths.values, [harness], reason: label);
    }
    expect(_preset('Claude').programPaths.keys, ['COQUI_ACP_CLAUDE_EXECUTABLE']);
    expect(_preset('Codex').programPaths.keys, ['COQUI_ACP_CODEX_EXECUTABLE']);
    expect(_preset('Claude').args, contains('--allow-client-plugins'), reason: 'a profile\'s skills are a plugin');
    expect(_preset('Codex').args, isNot(contains('--allow-client-plugins')));
  });

  test('a host\'s own command runs as this app with its flag, or as the canoa the environment names', () {
    final claude = _preset('Claude');
    List<String> line((String, List<String>) run) => [run.$1, ...run.$2];
    expect(line(invocation(claude, executable: r'C:\Canoa\CanoaUI.exe')),
        [r'C:\Canoa\CanoaUI.exe', '--canoa-child', ...claude.args]);
    expect(line(invocation(claude, executable: r'C:\Canoa\CanoaUI.exe', hostProgram: '/build/canoa')),
        ['/build/canoa', ...claude.args]);
    expect(line(invocation(claude, executable: '/app', hostProgram: '')), ['/app', '--canoa-child', ...claude.args]);
    const other = AgentCommand(label: 'Mine', program: 'my-agent', args: ['--acp']);
    expect(line(invocation(other, executable: '/app', hostProgram: '/build/canoa')), ['my-agent', '--acp']);
  });

  test('a source check looks only for the tool a host\'s own command drives', () async {
    final asked = <String>[];
    final check = await checkCommand(_preset('Claude'), (program) async {
      asked.add(program);
      return program == 'claude' ? '/usr/bin/claude' : null;
    });
    expect(asked, ['claude']);
    expect(check.ready, isTrue);
  });

  test('the translators the host runs are Claude Code and Codex, for the ledger and for profiles', () {
    final names = HarnessNames();
    expect(names.label(reportedName: 'coqui-acp-claude', entry: 'Claude'), 'Claude Code');
    expect(names.label(reportedName: 'coqui-acp-codex', entry: 'Codex'), 'Codex');
    expect(harnessIdOf(reportedName: 'coqui-acp-claude', entry: 'hosted'), 'claude-code');
    expect(harnessIdOf(reportedName: 'coqui-acp-codex', entry: 'hosted'), 'codex');
  });
}
