// The default workspace: `[sessions] workspace` as a path template, its variables, the one-segment rule for a value, what
// canoa.cfg refuses, and where Documents is on each platform.
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/workspace/documents.dart';
import 'package:canoa/workspace/scratch.dart';
import 'package:canoa/workspace/workspace_template.dart';
import 'package:flutter_test/flutter_test.dart';

const _session = '3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40';
final _now = DateTime(2026, 11, 11, 15, 30, 5);

WorkspaceValues _values({
  String documents = '/home/ana/Documents',
  String home = '/home/ana',
  String session = _session,
  String handler = 'claude',
  String source = 'Claude',
  String host = 'box1',
  String fqdn = '',
}) => WorkspaceValues(
  documents: documents, home: home, session: session, now: _now, handler: handler, source: source, host: host, fqdn: fqdn,
);

String _expand(String template, [WorkspaceValues? values]) => expandWorkspace(template, values ?? _values());

Matcher _formatProblem(List<String> words) =>
    throwsA(isA<FormatException>().having((e) => e.message, 'message', allOf([for (final word in words) contains(word)])));

void main() {
  group('the template', () {
    test('the default is Documents/Canoa/scratch/<session>', () {
      expect(_expand(defaultWorkspace), '/home/ana/Documents/Canoa/scratch/$_session');
      expect(CanoaConfig.parse('').sessions.workspace, defaultWorkspace);
    });

    test('a set template expands each variable', () {
      expect(_expand(r'${home}/work/${date}/${session}'), '/home/ana/work/2026-11-11/$_session');
      expect(_expand(r'${documents}/${handler}/${source}/${host}/${session}'),
          '/home/ana/Documents/claude/Claude/box1/$_session');
      expect(_expand(r'/s/${time}/${datetime}/${session}'), '/s/153005/2026-11-11T153005/$_session');
      expect(_expand(r'${home}\w\${session}'), '/home/ana/w/$_session', reason: 'a backslash in a template separates too');
    });

    test('the separator is the platform\'s', () {
      expect(expandWorkspace(defaultWorkspace, _values(documents: r'E:\people\ana\Documents'), separator: r'\'),
          r'E:\people\ana\Documents\Canoa\scratch\' + _session);
      expect(expandWorkspace(defaultWorkspace, _values(documents: r'C:\'), separator: r'\'), r'C:\Canoa\scratch\' + _session);
    });

    test(r'a part the variables leave empty is dropped, as ${fqdn} is on the desktop', () {
      expect(_expand(r'${documents}/Canoa/scratch/${fqdn}/${session}'), '/home/ana/Documents/Canoa/scratch/$_session');
      expect(_expand(r'${documents}/Canoa/scratch/${fqdn}/${session}', _values(fqdn: 'example.com')),
          '/home/ana/Documents/Canoa/scratch/example.com/$_session');
    });

    test('a value holding a separator or .. stays one segment', () {
      expect(_expand(r'${documents}/${source}/${session}', _values(source: 'a/b')), '/home/ana/Documents/a_b/$_session');
      expect(_expand(r'${documents}/${source}/${session}', _values(source: r'a\b')), '/home/ana/Documents/a_b/$_session');
      for (final hostile in ['..', '../..', r'..\..\x', 'a/../b', '.', '...', ' ', '']) {
        final out = _expand(r'${documents}/Canoa/${source}/${session}', _values(source: hostile));
        final below = out.substring('/home/ana/Documents/Canoa/'.length).split('/');
        expect(below.where((part) => part == '..' || part == '.'), isEmpty, reason: 'for "$hostile": $out');
        expect(below.last, _session, reason: out);
        expect(below.length, lessThanOrEqualTo(2), reason: 'at most the one segment for "$hostile": $out');
      }
    });

    test('control and reserved characters are replaced', () {
      expect(safeSegment('a:b*c?d"e<f>g|h\u0001i\u007f'), 'a_b_c_d_e_f_g_h_i_');
      expect(safeSegment('CON'), '_CON');
      expect(safeSegment('nul.txt'), '_nul.txt');
      expect(safeSegment('name. '), 'name');
      expect(safeSegment('.hidden'), '_hidden');
      expect(safeSegment('x' * 300).length, 100);
      expect(safeSegment('Claude Code'), 'Claude Code');
    });

    test('an unknown variable, an unclosed one or none for the session is refused', () {
      expect(workspaceProblem(r'${documents}/${nope}/${session}'), contains(r'unknown variable ${nope}'));
      expect(workspaceProblem(r'${documents}/Canoa'), contains(r'no ${session}'));
      expect(workspaceProblem(r'${documents}/${session'), isNotNull);
      expect(workspaceProblem(''), isNotNull);
      expect(workspaceProblem(r'${documents}/${session}'), isNull);
      expect(() => expandWorkspace(r'${documents}/Canoa', _values()), throwsFormatException);
    });
  });

  group('canoa.cfg', () {
    test('round-trips the key and writes nothing for the default', () {
      const template = r'${home}/scratch/${date}/${session}';
      final config = CanoaConfig.parse('[sessions]\nworkspace = "$template"\n');
      expect(config.sessions.workspace, template);
      expect(CanoaConfig.parse(config.encode()).sessions.workspace, template);
      expect(config.encode(), contains('workspace = '));
      expect(const CanoaConfig().encode(), isNot(contains('workspace')));
      const windows = CanoaConfig(sessions: LedgerSettings(workspace: r'C:\Scratch\${session}'));
      expect(CanoaConfig.parse(windows.encode()).sessions.workspace, r'C:\Scratch\${session}');
    });

    test('refuses an unknown variable or none for the session, naming the key', () {
      expect(() => CanoaConfig.parse('[sessions]\nworkspace = "\${documents}/\${nope}/\${session}"\n'),
          _formatProblem(['sessions.workspace', 'nope']));
      expect(() => CanoaConfig.parse('[sessions]\nworkspace = "\${documents}/Canoa"\n'),
          _formatProblem(['sessions.workspace', 'session']));
      expect(() => CanoaConfig.parse('[sessions]\nworkspace = 3\n'), _formatProblem(['sessions.workspace']));
    });
  });

  group('Documents', () {
    late Directory home;
    setUp(() => home = Directory.systemTemp.createTempSync('canoa-home'));
    tearDown(() => home.deleteSync(recursive: true));

    test('on Linux it follows XDG_DOCUMENTS_DIR, else user-dirs.dirs, else ~/Documents', () {
      final env = {'HOME': home.path};
      expect(documentsFolder(system: 'linux', environment: env), '${home.path}/Documents');
      File('${home.path}/.config/user-dirs.dirs')
        ..createSync(recursive: true)
        ..writeAsStringSync('# comment\nXDG_DESKTOP_DIR="\$HOME/Desktop"\nXDG_DOCUMENTS_DIR="\$HOME/Papers"\n');
      expect(documentsFolder(system: 'linux', environment: env), '${home.path}/Papers');
      expect(documentsFolder(system: 'linux', environment: {...env, 'XDG_DOCUMENTS_DIR': '/data/docs'}), '/data/docs');
      expect(documentsFolder(system: 'linux', environment: {...env, 'XDG_DOCUMENTS_DIR': ''}), '${home.path}/Papers',
          reason: 'an empty variable is unset');
      expect(documentsFolder(system: 'linux', environment: {...env, 'XDG_CONFIG_HOME': '${home.path}/none'}),
          '${home.path}/Documents');
    });

    test('on macOS it is ~/Documents, and XDG_DOCUMENTS_DIR is not read', () {
      expect(documentsFolder(system: 'macos', environment: {'HOME': '/Users/ana', 'XDG_DOCUMENTS_DIR': '/x'}),
          '/Users/ana/Documents');
    });

    test('on Windows it is the known folder, else the profile\'s Documents', () {
      expect(documentsFolder(system: 'windows', environment: {'USERPROFILE': r'E:\people\ana'},
          knownFolder: () => r'D:\OneDrive\Documents'), r'D:\OneDrive\Documents');
      expect(documentsFolder(system: 'windows', environment: {'USERPROFILE': r'E:\people\ana'}, knownFolder: () => null),
          r'E:\people\ana\Documents');
      expect(documentsFolder(system: 'windows', environment: {}, knownFolder: () => null), isNull);
    });

    test('on this Windows the known folder is asked of the system', () {
      final found = documentsFolder();
      expect(found, isNotNull);
      expect(Directory(found!).existsSync(), isTrue, reason: found);
    }, skip: Platform.isWindows ? false : 'asks the Windows shell');
  });

  group('scratchFolder', () {
    test('makes the default under the given Documents, named for the session', () {
      expect(scratchFolder(defaultWorkspace, _session, documents: '/d', separator: '/'), '/d/Canoa/scratch/$_session');
    });

    test('names the handler and source of the session\'s source', () {
      const claude = AgentSource.preset(AgentCommand(label: 'Claude', program: 'claude-agent-acp'));
      const hosted = AgentSource.hosted('lab', harness: 'codex', title: 'The lab');
      const other = AgentSource.hosted('plain');
      const antigravity = AgentSource.preset(AgentCommand(label: 'Antigravity', program: 'agy'));
      const pi = AgentSource.preset(AgentCommand(label: 'Pi', program: 'pi-acp'));
      const mine = AgentSource.command(AgentCommand(label: 'My Agent', program: 'mine'));
      const template = r'${documents}/${handler}/${source}/${session}';
      expect(scratchFolder(template, _session, source: claude, documents: '/d', separator: '/'), '/d/claude/Claude/$_session');
      expect(scratchFolder(template, _session, source: hosted, documents: '/d', separator: '/'), '/d/codex/lab/$_session');
      expect(handlerOf(other), 'plain');
      expect([for (final source in [claude, hosted, antigravity, pi, mine]) handlerOf(source)],
          ['claude', 'codex', 'antigravity', 'pi', 'My Agent'], reason: 'the provider id, else the label');
      expect(handlerOf(const AgentSource.hosted('grok-lab', harness: 'grok')), 'grok');
    });

    test('makes and removes a folder, saying whether it was new', () async {
      final base = Directory.systemTemp.createTempSync('canoa-scratch');
      addTearDown(() => base.deleteSync(recursive: true));
      final folder = scratchFolder(defaultWorkspace, _session, documents: base.path)!;
      expect(Directory(folder).existsSync(), isFalse);
      expect(await makeFolder(folder), isTrue);
      expect(Directory(folder).existsSync(), isTrue);
      expect(await makeFolder(folder), isFalse);
      await removeEmptyFolder(folder);
      expect(Directory(folder).existsSync(), isFalse);
    });
  });

  test('a session id is a version 4 UUID, and each is new', () {
    final ids = {for (var i = 0; i < 50; i++) newSessionUuid()};
    expect(ids, hasLength(50));
    for (final id in ids) {
      expect(id, matches(RegExp(r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$')));
    }
  });
}
