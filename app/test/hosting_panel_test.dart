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
import 'fixtures/providers_fixture.dart';

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

  /// Opens Add an agent, which the host's description of an entry draws, and types [values] into its boxes by member name.
  Future<void> addDialog(WidgetTester tester, Map<String, String> values) async {
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    for (final MapEntry(:key, :value) in values.entries) {
      final box = find.byKey(Key('agent-field-$key'));
      await tester.ensureVisible(box);
      await tester.enterText(box, value);
    }
  }

  Future<void> addIt(WidgetTester tester) async {
    await tester.ensureVisible(find.text('Add').last);
    await tester.tap(find.text('Add').last);
    await tester.pumpAndSettle();
  }

  testWidgets('an agent added by hand goes to canoa.cfg on a loopback port of its own, with the keys the host says an entry takes', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await addDialog(tester, {'name': 'bad name', 'program': 'node', 'args': 'my agent.mjs\n--flag'});
    // What an entry's name may hold is the host's to say, and its words are shown.
    host.refuseWith = 'an entry name has 1 to 64 letters, digits, hyphens or underscores';
    await addIt(tester);
    expect(find.text('an entry name has 1 to 64 letters, digits, hyphens or underscores'), findsOneWidget);
    expect(host.saved, isEmpty);
    host.refuseWith = null;

    await addDialog(tester, {'name': 'mine', 'program': 'node', 'args': 'my agent.mjs\n--flag', 'working_roots': 'relative/dir'});
    await addIt(tester);
    expect(find.text('The workspace must be a whole path.'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('agent-field-working_roots')), '/home/me/src');
    await addIt(tester);

    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry, {
      'program': 'node',
      'args': ['my agent.mjs', '--flag'],
      'listen': '127.0.0.1:7811',
      'working_roots': ['/home/me/src'],
    });
    expect(find.text('mine'), findsOneWidget);
  });

  testWidgets('what the host describes of an entry is what the dialog offers: a name is needed, and a program or a harness', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await addDialog(tester, {});
    expect(find.text('What it runs'), findsOneWidget, reason: "the host's sections are drawn");
    expect(find.text('Where it works'), findsOneWidget);
    for (final member in ['name', 'program', 'args', 'working_roots', 'accounts', 'profiles']) {
      expect(find.byKey(Key('agent-field-$member')), findsOneWidget, reason: member);
    }
    expect(find.byKey(const Key('agent-field-harness')), findsOneWidget, reason: 'a choice of the harnesses the host lists');
    expect(find.byKey(const Key('agent-advanced')), findsOneWidget, reason: 'the rarer keys are folded');
    await addIt(tester);
    expect(find.text('Name is needed'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('agent-field-name')), 'mine');
    await addIt(tester);
    expect(find.text('Name a program or a harness.'), findsOneWidget);
    expect(host.saved, isEmpty);

    // A harness the host lists is chosen and written under its own key, with what an entry that runs one takes.
    await tester.tap(find.byKey(const Key('agent-field-harness')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Codex').last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('agent-field-accounts')), 'work\nhome');
    await addIt(tester);
    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry, {'harness': 'codex', 'accounts': ['work', 'home'], 'listen': '127.0.0.1:7811'});
  });

  testWidgets('a host that has not described an entry cannot have one added, and says so', (tester) async {
    addTearDown(providerCatalog.clear);
    providerCatalog.clear();
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('agent-entry-unknown')), findsOneWidget);
    expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Add').last).onPressed, isNull);
  });

  testWidgets('an agent is given the reader of its quota from the ones the host offers', (tester) async {
    addTearDown(listBundledProviders);
    final answer = bundledProvidersJson();
    providerCatalog.adopt({
      ...answer,
      'usage_readers': [
        {'id': 'antigravity', 'origin': 'bundled'},
        {'id': 'team_meter', 'origin': 'team_ext'},
      ],
    });
    final host = _Harness();
    await host.show(tester);
    await addDialog(tester, {'name': 'mine', 'program': 'node', 'args': 'agent.mjs'});
    await tester.ensureVisible(find.byKey(const Key('usage-reader')));
    await tester.tap(find.byKey(const Key('usage-reader')));
    await tester.pumpAndSettle();
    expect(find.text('As its command says'), findsWidgets);
    expect(find.text('antigravity'), findsOneWidget);
    await tester.tap(find.text('team_meter (team_ext)').last);
    await tester.pumpAndSettle();
    await addIt(tester);
    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry['usage'], 'team_meter');
  });

  testWidgets('an agent may be kept to a folder on a share, but not to a device or a bare server', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await addDialog(tester, {'name': 'mine', 'program': 'node', 'args': 'agent.mjs'});
    for (final refused in [r'\\.\pipe\x', r'\\fileserver']) {
      final box = find.byKey(const Key('agent-field-working_roots'));
      await tester.ensureVisible(box);
      await tester.enterText(box, refused);
      await addIt(tester);
      expect(find.text('The workspace must be a whole path.'), findsOneWidget, reason: refused);
      expect(host.saved, isEmpty);
    }
    await tester.enterText(find.byKey(const Key('agent-field-working_roots')), r'\\fileserver\projects\app');
    await addIt(tester);
    final entry = (host.saved.single.host!['agents'] as Map)['mine'] as Map;
    expect(entry['working_roots'], [r'\\fileserver\projects\app']);
  });

  testWidgets('what canoa finds wrong with a change is shown, and nothing is listed as hosted', (tester) async {
    final host = _Harness()..refuseWith = 'host entry mine: program not found';
    await host.show(tester);
    await addDialog(tester, {'name': 'mine', 'program': 'nothing-here'});
    await addIt(tester);
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
