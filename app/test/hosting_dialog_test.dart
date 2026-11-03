// The hosting dialog against a host it cannot tell from the real one: a key is paired or removed only by the person's
// click on a dialog that shows the whole id, an agent added by hand reaches Canoa.cfg, and a PIN is shown until it is used.
import 'dart:async';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/hosting_dialog.dart';
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
          body: HostingDialog(
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

  testWidgets('an agent added by hand goes to Canoa.cfg on a loopback port of its own', (tester) async {
    final host = _Harness();
    await host.show(tester);
    await tester.tap(find.text('Add an agent'));
    await tester.pumpAndSettle();
    final fields = find.descendant(of: find.byType(AlertDialog).last, matching: find.byType(TextField));
    await tester.enterText(fields.at(0), 'bad name');
    await tester.enterText(fields.at(1), 'node "my agent.mjs" --flag');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.textContaining('A name is 1 to 64'), findsOneWidget);
    expect(host.saved, isEmpty);

    await tester.enterText(fields.at(0), 'mine');
    await tester.enterText(fields.at(2), 'relative/dir');
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.text('The folder must be a whole path.'), findsOneWidget);
    await tester.enterText(fields.at(2), '/home/me/src');
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

  testWidgets('what canoa-host finds wrong with a change is shown, and nothing is listed as hosted', (tester) async {
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

  testWidgets('stopping hosting an entry asks first, then saves Canoa.cfg without it', (tester) async {
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
    unawaited(host.supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
    await host.show(tester);
    host.tool.children.single.out.add('{"name":"claude","kind":"agent","url":"ws://127.0.0.1:7801","peer":"$_paired"}');
    await tester.pumpAndSettle();
    expect(find.text('Hosting, on this computer only.'), findsOneWidget);
    expect(find.text(_paired), findsNWidgets(2), reason: 'the host id, and the same key listed as paired');
    expect(find.textContaining(_methodShown), findsNothing, reason: 'a host that does not say its method shows none');
  });

  testWidgets('the method the host proves by is shown beside its id', (tester) async {
    final host = _Harness();
    unawaited(host.supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
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
