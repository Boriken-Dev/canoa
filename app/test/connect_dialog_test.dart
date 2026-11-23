// The Add connection dialog: what is saved, the method a host's key proves by, the reason one cannot run here, pairing by PIN under
// Noise, and what a search nearby may and may not fill in.
import 'dart:io' show Directory;

import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/shell/hosts/hosts_controller.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _peer = 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

Future<void> _open(
  WidgetTester tester, {
  required Future<void> Function(Connection) onAdd,
  String? noiseWhy,
  PairByPin? pair,
  NearbySearch? nearby,
  bool inBrowser = true,
}) async {
  await tester.binding.setSurfaceSize(const Size(900, 1600));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(debugShowCheckedModeBanner: false, home: Builder(builder: (context) => TextButton(
    onPressed: () => showAddConnectionDialog(
      context,
      onAdd: onAdd,
      nearby: nearby,
      methodUnavailable: (method) async => method == PeerMethod.noise ? noiseWhy : null,
      localPeerId: (method) async => inBrowser ? 'sha256:own-${method.id}' : throw UnsupportedError('no key'),
      pair: pair ?? (url, pin) async => throw StateError('no pairing here'),
      inBrowser: inBrowser,
    ),
    child: const Text('open'),
  ))));
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

Future<void> _choose(WidgetTester tester, PeerMethod method) async {
  await tester.tap(find.descendant(of: find.byKey(const ValueKey('connection-method')), matching: find.text(method.label)));
  await tester.pumpAndSettle();
}

Future<void> _fill(WidgetTester tester, {String name = 'Work agent'}) async {
  await tester.enterText(find.byKey(const ValueKey('connection-name')), name);
  await tester.tap(find.text('Direct ACP endpoint'));
  await tester.pumpAndSettle();
  await tester.enterText(find.byKey(const ValueKey('connection-address')), 'wss://agent.example/acp');
  await tester.enterText(find.byKey(const ValueKey('connection-peer')), _peer);
}

Future<void> _add(WidgetTester tester) async {
  await tester.ensureVisible(find.byKey(const ValueKey('connection-add')));
  await tester.tap(find.byKey(const ValueKey('connection-add')));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/connect_dialog_test.dart')));

  testWidgets('a connection has no method until the person names one, and the id shown is that method\'s own', (tester) async {
    final added = <Connection>[];
    await _open(tester, onAdd: (connection) async => added.add(connection));
    expect(find.textContaining('sha256:own-'), findsNothing);
    await _fill(tester);
    await _add(tester);
    expect(find.text('Choose the method the host proves by: canoa serve prints it.'), findsOneWidget);
    expect(added, isEmpty, reason: 'no method is assumed');
    await _choose(tester, PeerMethod.noise);
    expect(find.textContaining('sha256:own-noise'), findsOneWidget);
    await _add(tester);
    expect(added.single.method, 'noise');
    expect(added.single.peer, _peer);
    expect(added.single.kind, ConnectionKind.acp);
    expect(added.single.name, 'Work agent');
  });

  testWidgets('a name, a well-formed address and a well-formed id are required, and each says what is wrong', (tester) async {
    final added = <Connection>[];
    await _open(tester, onAdd: (connection) async => added.add(connection));
    await _choose(tester, PeerMethod.webcrypto);
    await _add(tester);
    expect(find.text('Name it: 1 to 64 characters.'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('connection-name')), 'Host');
    await _add(tester);
    expect(find.textContaining('Enter an address'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('connection-address')), 'https://user:secret@host:1/');
    await _add(tester);
    expect(find.text('An address holds no user name or password.'), findsOneWidget, reason: 'nothing secret is saved');
    await tester.enterText(find.byKey(const ValueKey('connection-address')), 'https://host:7800/');
    await _add(tester);
    expect(find.textContaining('Enter the verified server peer ID'), findsOneWidget);
    expect(added, isEmpty);
    await tester.enterText(find.byKey(const ValueKey('connection-peer')), _peer);
    await _add(tester);
    expect(added.single.kind, ConnectionKind.canoa, reason: 'a Canoa host is the default kind');
    expect(added.single.address, 'https://host:7800/');
  });

  testWidgets('a method this browser cannot run says why, and adding by it is refused with nothing saved', (tester) async {
    final added = <Connection>[];
    await _open(tester, onAdd: (connection) async => added.add(connection), noiseWhy: 'no X25519 here');
    expect(find.textContaining('Pairing by PIN needs Noise, which is unavailable in this browser: no X25519 here'), findsOneWidget);
    expect(find.text('Pair'), findsNothing);
    await _choose(tester, PeerMethod.noise);
    expect(find.text('Noise is unavailable in this browser: no X25519 here'), findsWidgets);
    await _fill(tester);
    await _add(tester);
    expect(added, isEmpty);
    await _choose(tester, PeerMethod.webcrypto);
    await _add(tester);
    expect(added.single.method, 'webcrypto', reason: 'only the method the person names is used');
  });

  testWidgets('a name already saved is refused in the dialog\'s own words', (tester) async {
    await _open(tester, onAdd: (_) async => throw const FormatException('A connection named Work agent is already saved.'));
    await _fill(tester);
    await _choose(tester, PeerMethod.noise);
    await _add(tester);
    expect(find.text('A connection named Work agent is already saved.'), findsOneWidget);
    expect(find.text('Add connection'), findsOneWidget, reason: 'the dialog stays for the person to rename it');
  });

  testWidgets('a PIN pairs under Noise and fills in the id that machine proved', (tester) async {
    final asked = <(Uri, String)>[];
    await _open(tester, onAdd: (_) async {}, pair: (url, pin) async {
      asked.add((url, pin));
      return _peer;
    });
    await tester.enterText(find.widgetWithText(TextField, 'Where it waits to pair'), 'ws://127.0.0.1:20001');
    await tester.enterText(find.widgetWithText(TextField, 'PIN'), '123456');
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();
    expect(asked.single, (Uri.parse('ws://127.0.0.1:20001'), '123456'));
    expect(find.widgetWithText(TextField, _peer), findsOneWidget);
    expect(find.textContaining('Paired under Noise'), findsOneWidget);
    expect(find.textContaining('sha256:own-noise'), findsOneWidget, reason: 'Noise is now the method chosen');
  });

  testWidgets('a refused PIN says so and is cleared, since it is spent', (tester) async {
    await _open(tester, onAdd: (_) async {}, pair: (url, pin) async => throw Exception('The pairing was refused: wrong-code'));
    await tester.enterText(find.widgetWithText(TextField, 'Where it waits to pair'), 'ws://127.0.0.1:20001');
    final pin = find.widgetWithText(TextField, 'PIN');
    await tester.enterText(pin, '123456');
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();
    expect(find.textContaining('wrong-code'), findsOneWidget);
    expect(tester.widget<TextField>(pin).controller!.text, isEmpty);
  });

  group('on a desktop', () {
    testWidgets('it has no pairing by PIN, and nothing in it speaks of a browser', (tester) async {
      await _open(tester, onAdd: (_) async {}, inBrowser: false, noiseWhy: 'this build has no peer key to prove with');
      await _choose(tester, PeerMethod.noise);
      expect(find.textContaining('PIN'), findsNothing);
      expect(find.text('Pair'), findsNothing);
      final said = [
        for (final element in find.byWidgetPredicate((widget) => widget is Text || widget is SelectableText).evaluate())
          switch (element.widget) {
            Text(:final data, :final textSpan) => data ?? textSpan?.toPlainText() ?? '',
            SelectableText(:final data) => data ?? '',
            _ => '',
          },
      ];
      expect(said.where((text) => text.toLowerCase().contains('browser')), isEmpty);
    });
  });

  group('nearby', () {
    const claim = NearbyCandidate(
      label: 'Claude on the desk',
      url: 'ws://192.168.1.5:7801',
      peer: _peer,
      method: 'noise',
      offers: ['agent'],
    );

    testWidgets('with no host to ask, it says looking nearby needs one', (tester) async {
      await _open(tester, onAdd: (_) async {});
      expect(find.byKey(const ValueKey('nearby-needs-host')), findsOneWidget);
      expect(find.byKey(const ValueKey('nearby-look')), findsNothing);
    });

    testWidgets('a candidate is listed unticked, ticking it only fills the fields, and nothing is saved until Add', (tester) async {
      final added = <Connection>[];
      await _open(tester, onAdd: (connection) async => added.add(connection), nearby: () async => const NearbyResult([claim]));
      await tester.tap(find.byKey(const ValueKey('nearby-look')));
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('nearby ws://192.168.1.5:7801'));
      expect(row, findsOneWidget);
      expect(tester.widget<CheckboxListTile>(row).value, isFalse, reason: 'nothing is ticked for the person');
      expect(find.textContaining('Its claim:'), findsOneWidget, reason: 'what it says is a claim, and is shown as one');
      expect(added, isEmpty);
      expect(tester.widget<TextField>(find.byKey(const ValueKey('connection-address'))).controller!.text, isEmpty);

      await tester.ensureVisible(row);
      await tester.tap(row);
      await tester.pumpAndSettle();
      expect(tester.widget<CheckboxListTile>(row).value, isTrue);
      expect(tester.widget<TextField>(find.byKey(const ValueKey('connection-address'))).controller!.text, 'ws://192.168.1.5:7801');
      expect(tester.widget<TextField>(find.byKey(const ValueKey('connection-peer'))).controller!.text, _peer);
      expect(added, isEmpty, reason: 'ticking saves nothing');
      await _add(tester);
      expect(added.single.name, 'Claude on the desk');
      expect(added.single.method, 'noise');
    });

    testWidgets('a search that could not run on the link says why, and an empty one says nothing announced itself', (tester) async {
      await _open(tester, onAdd: (_) async {}, nearby: () async => const NearbyResult([], problem: 'The link could not be searched: no multicast'));
      await tester.tap(find.byKey(const ValueKey('nearby-look')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('nearby-problem')), findsOneWidget);
      expect(find.textContaining('no multicast'), findsOneWidget);
    });
  });

  testWidgets('golden: the dialog with two candidates found nearby, none ticked', (tester) async {
    final other = NearbyCandidate(label: 'Codex in the lab', url: 'ws://192.168.1.9:7802', peer: 'sha256:${'b' * 64}', method: 'webcrypto');
    await _open(tester, onAdd: (_) async {}, inBrowser: false, nearby: () async => NearbyResult([
      const NearbyCandidate(label: 'Claude on the desk', url: 'ws://192.168.1.5:7801', peer: _peer, method: 'noise', offers: ['agent']),
      other,
    ]));
    await tester.tap(find.byKey(const ValueKey('nearby-look')));
    await tester.pumpAndSettle();
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/dialog_add_connection_desktop.png'));
  });
}
