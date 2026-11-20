// The hosting dialog against a host it cannot tell from the real one: a key is paired or removed only by the person's
// click on a dialog that shows the whole id, an agent added by hand reaches canoa.cfg, and a PIN is shown until it is used.
import 'dart:async';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/hosting_panel.dart';
import 'package:canoa/providers/provider_catalog.dart' show providerCatalog;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_host.dart';

const _waiting = 'sha256:1111111111111111111111111111111111111111111111111111111111111111';
const _paired = 'sha256:2222222222222222222222222222222222222222222222222222222222222222';

/// A method as the dialog shows it beside a host id.
final _methodShown = RegExp(r'^ \(\w+\)$');

final class _Harness {
  final tool = FakeTool();
  late final supervisor = HostSupervisor(tool);
  final saved = <CanoaConfig>[];
  String? refuseWith;

  _Harness() {
    tool.answer = (args) => switch (args.first) {
      'peers' => const HostOutput(
        0,
        '{"peer":"$_waiting","state":"waiting","source":"127.0.0.1:50000"}\n'
            '{"peer":"$_paired","state":"accepted","source":null}\n',
        '',
      ),
      _ => const HostOutput(0, '', ''),
    };
  }

  Future<void> show(WidgetTester tester, {CanoaConfig config = const CanoaConfig()}) async {
    await tester.binding.setSurfaceSize(const Size(900, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: HostingPanel(
            supervisor: supervisor,
            config: config,
            onSave: (changed) async {
              if (refuseWith != null) return refuseWith;
              saved.add(changed);
              return null;
            },
            freePort: () async => 7811,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  List<List<String>> ranWith(String command) => tool.ran.where((args) => args.first == command).toList();
}

void main() {
  testWidgets('a waiting key is paired only after the person compares its whole id and says so', (tester) async {
    final host = _Harness();
    await host.show(tester);
    expect(find.text(_waiting), findsOneWidget);
    expect(find.text('Waiting (from 127.0.0.1:50000)'), findsOneWidget);

    await tester.tap(find.text('Pair…'));
    await tester.pumpAndSettle();
    expect(find.text('Pair this key?'), findsOneWidget);
    expect(find.text(_waiting), findsNWidgets(2), reason: 'the dialog shows the whole id to compare');
    expect(host.ranWith('accept'), isEmpty, reason: 'nothing is accepted by opening the dialog');
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(host.ranWith('accept'), isEmpty);

    await tester.tap(find.text('Pair…'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Pair'));
    await tester.pumpAndSettle();
    expect(host.ranWith('accept'), [
      ['accept', '--', _waiting],
    ]);
  });

  testWidgets('removing a key shows its id and waits for the person', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.byTooltip('Remove this key'));
    await tester.pumpAndSettle();
    expect(find.text('Remove this key?'), findsOneWidget);
    expect(find.text(_paired), findsNWidgets(2));
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(host.ranWith('revoke'), isEmpty);
    await tester.tap(find.byTooltip('Remove this key'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Remove'));
    await tester.pumpAndSettle();
    expect(host.ranWith('revoke'), [
      ['revoke', '--', _paired],
    ]);
  });

  testWidgets('an agent added by hand goes to canoa.cfg on a loopback port of its own', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'bad name');
    await tester.enterText(fields.at(1), 'node "my agent.mjs" --flag');
    // What an entry's name may hold is the host's to say, and its words are shown.
    host.refuseWith = 'an entry name has 1 to 64 letters, digits, hyphens or underscores';
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.text('an entry name has 1 to 64 letters, digits, hyphens or underscores'), findsOneWidget);
    expect(host.saved, isEmpty);
    host.refuseWith = null;

    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final again = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(again.at(0), 'mine');
    await tester.enterText(again.at(1), 'node "my agent.mjs" --flag');
    await tester.enterText(again.at(2), 'relative/dir');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.text('The workspace must be a whole path.'), findsOneWidget);
    await tester.enterText(again.at(2), '/home/me/src');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();

    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry, {
      'program': 'node',
      'args': ['my agent.mjs', '--flag'],
      'listen': '127.0.0.1:7811',
      'working_roots': ['/home/me/src'],
    });
    expect(find.text('mine'), findsOneWidget);
  });

  testWidgets("an agent is given the reader of its quota from the ones the host offers, and is offered none by a host that lists none", (tester) async {
    addTearDown(providerCatalog.clear);
    providerCatalog.clear();
    final bare = _Harness();
    await bare.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('usage-reader')), findsNothing);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    providerCatalog.adopt({
      'providers': const <Object?>[],
      'usage_readers': [
        {'id': 'antigravity', 'origin': 'bundled'},
        {'id': 'team_meter', 'origin': 'team_ext'},
      ],
    });
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'mine');
    await tester.enterText(fields.at(1), 'node agent.mjs');
    await tester.tap(find.byKey(const Key('usage-reader')));
    await tester.pumpAndSettle();
    expect(find.text('As its command says'), findsWidgets);
    expect(find.text('antigravity'), findsOneWidget);
    await tester.tap(find.text('team_meter (team_ext)').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry['usage'], 'team_meter');
  });

  testWidgets('an agent may be kept to a folder on a share, but not to a device or a bare server', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'mine');
    await tester.enterText(fields.at(1), 'node agent.mjs');
    for (final refused in [r'\\.\pipe\x', r'\\fileserver']) {
      await tester.enterText(fields.at(2), refused);
      await tester.tap(find.text('Add'));
      await tester.pumpAndSettle();
      expect(find.text('The workspace must be a whole path.'), findsOneWidget, reason: refused);
      expect(host.saved, isEmpty);
    }
    await tester.enterText(fields.at(2), r'\\fileserver\projects\app');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry['working_roots'], [r'\\fileserver\projects\app']);
  });

  testWidgets('what canoa finds wrong with a change is shown, and nothing is listed as hosted', (tester) async {
    final host = _Harness()..refuseWith = 'host entry mine: program not found';
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'mine');
    await tester.enterText(fields.at(1), 'nothing-here');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.text('host entry mine: program not found'), findsOneWidget);
    expect(find.text('None yet.'), findsOneWidget);
  });

  testWidgets('stopping hosting an entry asks first, then saves canoa.cfg without it', (tester) async {
    final host = _Harness();
    final config = const CanoaConfig().withHostAgent(name: 'claude', program: 'npx', listen: '127.0.0.1:7801');
    await host.show(tester, config: config);
    await tester.tap(find.byTooltip('Stop hosting claude'));
    await tester.pumpAndSettle();
    expect(host.saved, isEmpty);
    await tester.tap(find.widgetWithText(FilledButton, 'Stop hosting'));
    await tester.pumpAndSettle();
    expect(host.saved.single.hostEntries, isEmpty);
  });

  testWidgets('a PIN is shown until a program uses it, then who paired', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Show a PIN'));
    await tester.pump();
    host.tool.children.single.out.add('{"pin":"482913","peer":"sha256:host","url":"ws://127.0.0.1:9100"}');
    // The waiting spinner never settles.
    await tester.pump();
    await tester.pump();
    expect(find.text('482913'), findsOneWidget);
    expect(find.text('Dial ws://127.0.0.1:9100'), findsOneWidget);
    expect(find.textContaining(_methodShown), findsNothing, reason: 'a host that does not say its method shows none');
    host.tool.children.single.out.add('{"paired":"$_paired"}');
    await tester.pump();
    await tester.pump();
    expect(find.text('482913'), findsNothing);
    expect(find.text('Paired:'), findsOneWidget);
  });

  testWidgets('a PIN for the browser extension says where to type it and gives no address to dial', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Pair the browser extension'));
    await tester.pump();
    expect(host.tool.started.single, ['pair', '--native', '--until-stdin-closes']);
    host.tool.children.single.out.add('{"pin":"482913","peer":"sha256:host","method":"noise","native":true}');
    await tester.pump();
    await tester.pump();
    expect(find.text('482913'), findsOneWidget);
    expect(find.text('Type it in the Canoa extension, in your browser'), findsOneWidget);
    expect(find.textContaining('Dial '), findsNothing);
    host.tool.children.single.out.add('{"paired":"$_paired"}');
    await tester.pump();
    await tester.pump();
    expect(find.text('Paired:'), findsOneWidget);
  });

  testWidgets('closing the dialog ends a PIN still showing', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Show a PIN'));
    await tester.pump();
    host.tool.children.single.out.add('{"pin":"482913","peer":"sha256:host","url":"ws://127.0.0.1:9100"}');
    // The waiting spinner never settles.
    await tester.pump();
    await tester.pump();
    await tester.pumpWidget(const SizedBox());
    expect(host.tool.children.single.killed, isTrue);
  });

  testWidgets('the host id a browser pins is shown once the host says it', (tester) async {
    final host = _Harness();
    unawaited(host.supervisor.apply(configPath: '/c/canoa.cfg'));
    await host.show(tester);
    host.tool.children.single.out.add('{"name":"claude","kind":"agent","url":"ws://127.0.0.1:7801","peer":"$_paired"}');
    await tester.pumpAndSettle();
    expect(find.text('Hosting, on this computer only.'), findsOneWidget);
    expect(find.text(_paired), findsNWidgets(2), reason: 'the host id, and the same key listed as paired');
    expect(find.textContaining(_methodShown), findsNothing, reason: 'a host that does not say its method shows none');
  });

  testWidgets('with nothing to host the host is still up, and its id is shown', (tester) async {
    final host = _Harness();
    unawaited(host.supervisor.apply(configPath: '/c/canoa.cfg'));
    await host.show(tester);
    host.tool.children.single.out.add('{"ready":true,"peer":"$_paired","method":"noise"}');
    await tester.pumpAndSettle();
    expect(find.text('Nothing to host: add an agent.'), findsOneWidget);
    expect(find.text('A browser pins this host as:'), findsOneWidget);
  });

  testWidgets('another host on this home is said in its words, with a way to try again', (tester) async {
    final host = _Harness();
    unawaited(host.supervisor.apply(configPath: '/c/canoa.cfg'));
    await host.show(tester);
    host.tool.children.single
      ..err.add('canoa: canoa (process 12) already hosts this home, /h: stop it')
      ..end(statusHeld);
    await tester.pumpAndSettle();
    expect(find.textContaining('canoa (process 12) already hosts this home'), findsOneWidget);
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(host.tool.started, hasLength(2));
  });

  testWidgets('the method the host proves by is shown beside its id', (tester) async {
    final host = _Harness();
    unawaited(host.supervisor.apply(configPath: '/c/canoa.cfg'));
    await host.show(tester);
    host.tool.children.single.out.add(
      '{"name":"claude","kind":"agent","url":"ws://127.0.0.1:7801","peer":"$_paired","method":"noise"}',
    );
    await tester.pumpAndSettle();
    expect(host.supervisor.peerMethod, 'noise');
    expect(find.text(' (noise)'), findsOneWidget);
  });

  testWidgets('a PIN shows the method the host proves by', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Show a PIN'));
    await tester.pump();
    host.tool.children.single.out.add(
      '{"pin":"482913","peer":"sha256:host","method":"webcrypto","url":"ws://127.0.0.1:9100"}',
    );
    // The waiting spinner never settles.
    await tester.pump();
    await tester.pump();
    expect(find.text('sha256:host'), findsOneWidget);
    expect(find.text(' (webcrypto)'), findsOneWidget);
  });
}
