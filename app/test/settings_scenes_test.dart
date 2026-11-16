// Settings drawn at desktop and phone width: the page with its first section, Sources as providers, Appearance with its
// preview, and Permissions with the rules made. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same
// scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_speech.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

const _config = '''
[appearance]
look = "canoa"
brightness = "dark"

[sessions]
keep = 200
keep_text = true

[agents.Mine]
program = "my-agent"
args = ["--acp", "--verbose"]
''';

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/settings_scenes_test.dart')));

  final claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
  final codex = AgentSource.hosted('Codex', harness: 'codex', local: true);
  final pi = AgentSource.hosted('Pi', harness: 'pi', local: true);

  Future<SourceCheck> check(AgentSource source) async => switch (source.label) {
    'Claude' => SourceCheck.fromStore(
      store: 'claude', title: 'Claude Code', installed: true, path: '/bin/claude', version: '2.1.3', latest: '2.2.0', updateAvailable: true),
    'Codex' => SourceCheck.fromStore(store: 'codex', title: 'Codex', installed: true, path: '/bin/codex', version: '0.166.0'),
    'Pi' => const SourceCheck(missing: ['npx', 'pi']),
    _ => const SourceCheck(),
  };

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      Future<void> open(WidgetTester tester, {String? section, PermissionRules? rules}) async {
        await openSettings(
          tester,
          config: CanoaConfig.parse(_config),
          size: size,
          section: section,
          presets: [claude, codex, pi],
          checkSource: check,
          rules: rules,
          ledger: SessionLedger(MemoryLedgerBackend()),
          agents: const ['Claude', 'Codex', 'Mine'],
          version: '0.0.1',
        );
      }

      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/settings_${name}_$label.png'));
      }

      testWidgets('the page: its sections, a search, and General with the rows applying as they change', (tester) async {
        await open(tester);
        await golden(tester, 'page');
      });

      testWidgets('Sources: each with its version, readiness and a switch', (tester) async {
        await open(tester, section: 'sources');
        await tester.tap(find.byKey(const Key('source-row Pi')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('source-row Mine')));
        await tester.pumpAndSettle();
        await golden(tester, 'sources');
      });

      testWidgets('Appearance: look, brightness and detail, and a preview of the choice', (tester) async {
        await open(tester, section: 'appearance');
        await golden(tester, 'appearance');
      });

      testWidgets('Permissions: the mode, Bypass and its log, the rules made, scoped to a source', (tester) async {
        final rules = PermissionRules(MemoryLedgerBackend(), log: MemoryRuleLog(), clock: () => DateTime.utc(2026, 11, 12, 9));
        final rule = rules.add(source: 'Claude', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test')!;
        rules.recordUse(rule, 'npm test');
        rules.add(source: 'Codex', folder: '/work/site', kind: 'edit', type: RuleType.path, value: 'src/**');
        await open(tester, section: 'permissions', rules: rules);
        await golden(tester, 'permissions');
      });

      testWidgets('Voice: the service, where it is processed, the input and a test of the microphone', (tester) async {
        final speech = FakeSpeech(
          'system',
          'System speech',
          where: const SpeechWhere(
            place: SpeechPlace.system,
            leaves: AudioLeaves.maybe,
            words: 'The operating system turns it into text. When its speech service is online, the system sends the audio to its '
                'maker; its own privacy settings say whether that is on.',
          ),
          languageChoices: const [SpeechChoice('en-US', 'English'), SpeechChoice('es-PR', 'Espanol')],
        );
        await openSettings(tester, config: CanoaConfig.parse(_config), size: size, section: 'voice', voiceBackends: [speech]);
        await tester.ensureVisible(find.byKey(const Key('voice-test')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('voice-test')));
        await tester.pump();
        speech.level(0.55);
        speech.hear('Make the sum test pass');
        await tester.pump();
        await golden(tester, 'voice');
        await tester.tap(find.byKey(const Key('voice-test-stop')));
        await tester.pumpAndSettle();
      });

      testWidgets('Voice with no speech service here', (tester) async {
        await open(tester, section: 'voice');
        await golden(tester, 'voice_none');
      });
    });
  }
}
