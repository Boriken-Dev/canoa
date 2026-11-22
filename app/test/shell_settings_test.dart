// Settings' Shells page over a scripted host: the shells the host found are listed whether Canoa installed them or not, the host's
// own choice and each agent's are written through its `[host]` table (read again before each write, checked by the host first, the
// rest of the table kept), and what each agent is told is shown in the host's words. Names and paths are made up.
import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/settings/shell_settings.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;
import 'fixtures/store_link.dart';

HostFiles _files(FakeStoreHost host) => HostFiles(HostControl.dialing(() async => host.open()));

ShellSettings _settings(FakeStoreHost host, {Future<bool> Function()? restart, Future<void> Function()? saved}) =>
    ShellSettings(files: _files(host), restart: restart, saved: saved);

const _gitBash = {
  'id': 'git-bash',
  'label': 'Git Bash',
  'path': r'C:\Program Files\Git\bin\bash.exe',
  'family': 'bash',
  'origin': 'git-for-windows',
};
const _canoaBash = {
  'id': 'canoa-git-bash',
  'label': 'Git Bash (installed by Canoa)',
  'path': r'D:\canoa\programs\git-bash\2.56.1\git\bin\bash.exe',
  'family': 'bash',
  'origin': 'canoa',
};
const _wsl = {'id': 'wsl', 'label': 'WSL bash', 'path': r'C:\Windows\System32\bash.exe', 'family': 'bash', 'origin': 'wsl'};
const _pwsh = {'id': 'pwsh', 'label': 'PowerShell 7', 'path': r'C:\Program Files\PowerShell\7\pwsh.exe', 'family': 'powershell', 'origin': 'system'};

Map<String, Object?> _report({String? setting, Map<String, Object?>? chosen, String? problem}) => {
  'shells': [_canoaBash, _gitBash, _wsl, _pwsh],
  'setting': setting,
  'default': ?chosen,
  'problem': ?problem,
  'agents': [
    {
      'entry': 'Claude',
      'harness': 'claude',
      'shell': 'git-bash',
      'own': true,
      'id': 'git-bash',
      'env': ['CLAUDE_CODE_GIT_BASH_PATH'],
      'notice': null,
    },
    {
      'entry': 'Codex',
      'harness': 'codex',
      'shell': setting,
      'own': false,
      if (chosen != null) 'id': chosen['id'],
      'env': <String>[],
      'notice': 'Codex has no setting that names its shell: it looks for pwsh or powershell on the PATH on Windows and uses the account\'s login shell elsewhere',
    },
  ],
};

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/shell_settings_test.dart')));

  group('the shells of the host', () {
    test('are listed with where each comes from, the host\'s choice and what each agent is told', () async {
      final host = FakeStoreHost([])..shellReport = _report(setting: 'git-bash', chosen: _gitBash);
      final settings = _settings(host);
      await settings.load();
      final report = settings.report;
      expect([for (final shell in report.shells) shell.id], ['canoa-git-bash', 'git-bash', 'wsl', 'pwsh']);
      expect(report.shells.first.origin, 'canoa', reason: 'a copy the Store installed is listed as such');
      expect(report.shells[2].origin, 'wsl');
      expect(report.chosen?.id, 'git-bash');
      expect(report.agents.first.env, ['CLAUDE_CODE_GIT_BASH_PATH']);
      expect(report.agents.first.own, isTrue);
      expect(report.agents.last.own, isFalse);
      expect(report.agents.last.notice, contains('no setting'));
      expect(settings.problem, isNull);
      settings.dispose();
    });

    test('a key the host does not let list them has none and says so', () async {
      final host = FakeStoreHost([]);
      final settings = _settings(host);
      host.failures['shell/list'] = const ControlFailed(-32001, 'this key has no grant');
      await settings.load();
      expect(settings.report.shells, isEmpty);
      expect(settings.problem, contains('may not list'));
      expect(settings.loaded, isFalse);
      settings.dispose();
    });

    test('the host\'s shell is written to [host], keeping the rest of it, and taken out again', () async {
      final host = FakeStoreHost([])
        ..shellReport = _report()
        ..hostTable = {
          'origins': ['https://x.example'],
          'agents': {'Claude': {'harness': 'claude'}},
        };
      var saved = 0;
      final settings = _settings(host, saved: () async => saved++);
      await settings.load();
      expect(await settings.chooseForHost('git-bash'), isNull);
      expect(host.hostTable, {
        'origins': ['https://x.example'],
        'agents': {'Claude': {'harness': 'claude'}},
        'shell': 'git-bash',
      });
      expect(host.params['config/check'], isNotEmpty, reason: 'the host checked it first');
      expect(settings.restartPending, isTrue);
      expect(saved, 1);
      expect(await settings.chooseForHost(null), isNull);
      expect(host.hostTable!.containsKey('shell'), isFalse);
      host.hostTable = {'shell': 'pwsh'};
      expect(await settings.chooseForHost(null), isNull);
      expect(host.hostTable, isNull, reason: 'an empty [host] is removed');
      settings.dispose();
    });

    test('an agent\'s own shell is written to its entry only, and an entry the file lacks is refused', () async {
      final host = FakeStoreHost([])
        ..shellReport = _report()
        ..hostTable = {
          'shell': 'pwsh',
          'agents': {
            'Claude': {'harness': 'claude'},
            'Codex': {'harness': 'codex'},
          },
        };
      final settings = _settings(host);
      await settings.load();
      expect(await settings.chooseForAgent('Claude', 'git-bash'), isNull);
      expect(host.hostTable!['agents'], {
        'Claude': {'harness': 'claude', 'shell': 'git-bash'},
        'Codex': {'harness': 'codex'},
      });
      expect(host.hostTable!['shell'], 'pwsh', reason: 'the host\'s own choice is not touched');
      expect(await settings.chooseForAgent('Claude', null), isNull);
      expect((host.hostTable!['agents'] as Map)['Claude'], {'harness': 'claude'});
      expect(await settings.chooseForAgent('Nobody', 'wsl'), contains('no agent called Nobody'));
      settings.dispose();
    });

    test('a refusal of the host comes back in its words and nothing is written', () async {
      final host = FakeStoreHost([])
        ..shellReport = _report()
        ..hostTable = {'origins': ['https://x.example']};
      final settings = _settings(host);
      await settings.load();
      host.checkRefusal = 'that would leave canoa.cfg refused: host entry Claude: the shell nothing was not found';
      expect(await settings.chooseForHost('nothing'), contains('was not found'));
      expect(host.params['config/set'], isNull);
      expect(settings.restartPending, isFalse);
      settings.dispose();
    });

    test('the host restarts once, and what is pending clears only when it did', () async {
      final host = FakeStoreHost([])
        ..shellReport = _report()
        ..hostTable = {};
      var allowed = false, restarts = 0;
      final settings = _settings(host, restart: () async {
        restarts++;
        return allowed;
      });
      await settings.load();
      await settings.chooseForHost('wsl');
      await settings.restartHost();
      expect(settings.restartPending, isTrue, reason: 'declined: still pending');
      allowed = true;
      await settings.restartHost();
      expect(restarts, 2);
      expect(settings.restartPending, isFalse);
      settings.dispose();
    });
  });

  group('the Shells page', () {
    Future<(FakeStoreHost, ShellSettings)> open(WidgetTester tester, {Map<String, Object?>? report, Future<bool> Function()? restart}) async {
      final host = FakeStoreHost([])
        ..shellReport = report ?? _report(setting: 'git-bash', chosen: _gitBash)
        ..hostTable = {
          'shell': 'git-bash',
          'agents': {
            'Claude': {'harness': 'claude', 'shell': 'git-bash'},
            'Codex': {'harness': 'codex'},
          },
        };
      final settings = _settings(host, restart: restart);
      await tester.binding.setSurfaceSize(const Size(900, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(debugShowCheckedModeBanner: false, home: Scaffold(body: SingleChildScrollView(child: Padding(padding: const EdgeInsets.all(16), child: ShellsSection(settings: settings))))));
      await tester.pumpAndSettle();
      addTearDown(settings.dispose);
      return (host, settings);
    }

    testWidgets('says which shell the host uses and how each agent is told its own, or why it cannot be', (tester) async {
      await open(tester);
      expect(find.descendant(of: find.byKey(const Key('shell-host')), matching: find.textContaining(r'Git Bash: C:\Program Files\Git\bin\bash.exe')), findsOneWidget);
      expect(
        find.descendant(of: find.byKey(const Key('shell-agent-Claude')), matching: find.textContaining('told to claude as CLAUDE_CODE_GIT_BASH_PATH')),
        findsOneWidget,
      );
      expect(find.descendant(of: find.byKey(const Key('shell-agent-Codex')), matching: find.textContaining('Codex has no setting that names its shell')), findsOneWidget);
      expect(find.byKey(const Key('shells-restart-note')), findsNothing);
    });

    testWidgets('choosing a shell for the host writes it and says the host reads it when it restarts', (tester) async {
      var restarted = 0;
      final (host, _) = await open(tester, restart: () async {
        restarted++;
        return true;
      });
      await tester.tap(find.byKey(const Key('shell-host-menu')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('PowerShell 7').last);
      await tester.pumpAndSettle();
      expect(host.hostTable!['shell'], 'pwsh');
      expect(find.byKey(const Key('shells-restart-note')), findsOneWidget);
      await tester.tap(find.byKey(const Key('shells-restart')));
      await tester.pumpAndSettle();
      expect(restarted, 1);
      expect(find.byKey(const Key('shells-restart-note')), findsNothing);
    });

    testWidgets('choosing one for an agent writes its own entry, and "the host\'s shell" takes it out', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('shell-agent-menu-Codex')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('WSL bash').last);
      await tester.pumpAndSettle();
      expect((host.hostTable!['agents'] as Map)['Codex'], {'harness': 'codex', 'shell': 'wsl'});
      await tester.tap(find.byKey(const Key('shell-agent-menu-Claude')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('The host\'s shell').last);
      await tester.pumpAndSettle();
      expect((host.hostTable!['agents'] as Map)['Claude'], {'harness': 'claude'});
    });

    testWidgets('a host that found no shell points to the Store', (tester) async {
      await open(tester, report: {'shells': <Object?>[], 'agents': <Object?>[]});
      expect(find.byKey(const Key('shells-none')), findsOneWidget);
      expect(find.textContaining('Programs'), findsOneWidget);
    });

    testWidgets('the page as the person sees it', (tester) async {
      await open(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/shells_settings_desktop.png'));
    });
  });
}
