// The connect dialog: the method an agent's key proves by, the reason one cannot run here, and pairing by PIN under Noise.
import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _peer = 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

Future<void> _open(
  WidgetTester tester, {
  required Future<void> Function(ConnectTarget) onConnect,
  String? noiseWhy,
  PairByPin? pair,
  PeerMethod? method,
}) async {
  await tester.binding.setSurfaceSize(const Size(900, 1600));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
    onPressed: () => showCanoaConnectDialog(
      context,
      onConnect,
      methodUnavailable: (method) async => method == PeerMethod.noise ? noiseWhy : null,
      localPeerId: (method) async => 'sha256:own-${method.id}',
      pair: pair ?? (url, pin) async => throw StateError('no pairing here'),
      method: method,
      inBrowser: true,
    ),
    child: const Text('open'),
  ))));
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

Future<void> _choose(WidgetTester tester, PeerMethod method) async {
  await tester.tap(find.descendant(of: find.byType(SegmentedButton<PeerMethod>), matching: find.text(method.label)));
  await tester.pumpAndSettle();
}

Future<void> _fill(WidgetTester tester) async {
  await tester.enterText(find.widgetWithText(TextField, 'WebSocket WSS endpoint'), 'wss://agent.example/acp');
  await tester.enterText(find.widgetWithText(TextField, 'Pinned server peer ID'), _peer);
}

void main() {
  testWidgets('an agent typed in by hand has no method until the person names one, and the id shown is that method\'s own',
      (tester) async {
    final targets = <ConnectTarget>[];
    await _open(tester, onConnect: (target) async => targets.add(target));
    expect(find.textContaining('sha256:own-'), findsNothing);
    await _fill(tester);
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(find.text('Choose the method the host proves by: canoa-host serve prints it.'), findsOneWidget);
    expect(targets, isEmpty, reason: 'no method is assumed');
    await _choose(tester, PeerMethod.noise);
    expect(find.textContaining('sha256:own-noise'), findsOneWidget);
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(targets.single.method, PeerMethod.noise);
    expect(targets.single.peerId, _peer);
  });

  testWidgets('a method an agent\'s record or host names is prefilled, and the person can change it', (tester) async {
    final targets = <ConnectTarget>[];
    await _open(tester, onConnect: (target) async => targets.add(target), method: PeerMethod.webcrypto);
    expect(find.textContaining('sha256:own-webcrypto'), findsOneWidget);
    await _fill(tester);
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(targets.single.method, PeerMethod.webcrypto);
  });

  testWidgets('a method this browser cannot run says why, and connecting by it is refused with nothing dialled', (tester) async {
    final targets = <ConnectTarget>[];
    await _open(tester, onConnect: (target) async => targets.add(target), noiseWhy: 'no X25519 here');
    expect(find.textContaining('Pairing by PIN needs Noise, which is unavailable in this browser: no X25519 here'), findsOneWidget);
    expect(find.text('Pair'), findsNothing);
    await _choose(tester, PeerMethod.noise);
    expect(find.text('Noise is unavailable in this browser: no X25519 here'), findsWidgets);
    await _fill(tester);
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(targets, isEmpty);
    await _choose(tester, PeerMethod.webcrypto);
    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();
    expect(targets.single.method, PeerMethod.webcrypto, reason: 'only the method the person names is used');
  });

  testWidgets('a PIN pairs under Noise and fills in the id that machine proved', (tester) async {
    final asked = <(Uri, String)>[];
    await _open(tester, onConnect: (_) async {}, pair: (url, pin) async {
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

  group('on a desktop that runs agents itself', () {
    const scripted = AgentCommand(label: 'Scripted', program: 'node');

    Future<void> openLocal(WidgetTester tester) async {
      await tester.binding.setSurfaceSize(const Size(1280, 720));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () => showCanoaConnectDialog(
          context,
          (_) async {},
          onRunLocalAgent: (command, {required cwd, required title, required worktree}) async {},
          localAgents: const [scripted],
          checkInstalled: (_) async => true,
          methodUnavailable: (method) async => 'this build has no peer key to prove with',
          localPeerId: (method) async => throw UnsupportedError('no key'),
          inBrowser: false,
        ),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
    }

    testWidgets('the working directory is where the Run buttons are, in sight', (tester) async {
      await openLocal(tester);
      final workspace = find.widgetWithText(TextField, 'Agent workspace (absolute path)');
      expect(tester.getTopLeft(workspace).dy, lessThan(tester.getTopLeft(find.text('Run Scripted')).dy));
      expect(tester.getBottomLeft(workspace).dy, lessThan(720), reason: 'seen without scrolling the dialog');
      expect(workspace.hitTestable(), findsOneWidget);
    });

    testWidgets('nothing in it speaks of a browser, and what is about other machines says so', (tester) async {
      await openLocal(tester);
      await _choose(tester, PeerMethod.noise);
      final said = [
        for (final element in find.byWidgetPredicate((widget) => widget is Text || widget is SelectableText).evaluate())
          switch (element.widget) {
            Text(:final data, :final textSpan) => data ?? textSpan?.toPlainText() ?? '',
            SelectableText(:final data) => data ?? '',
            _ => '',
          },
      ];
      expect(said.where((text) => text.toLowerCase().contains('browser')), isEmpty);
      final remote = find.text('Or connect to an agent on another machine');
      expect(remote, findsOneWidget);
      expect(tester.getTopLeft(find.textContaining('fingerprint')).dy, greaterThan(tester.getTopLeft(remote).dy));
    });
  });

  testWidgets('a refused PIN says so and is cleared, since it is spent', (tester) async {
    await _open(tester, onConnect: (_) async {}, pair: (url, pin) async => throw Exception('The pairing was refused: wrong-code'));
    await tester.enterText(find.widgetWithText(TextField, 'Where it waits to pair'), 'ws://127.0.0.1:20001');
    final pin = find.widgetWithText(TextField, 'PIN');
    await tester.enterText(pin, '123456');
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();
    expect(find.textContaining('wrong-code'), findsOneWidget);
    expect(tester.widget<TextField>(pin).controller!.text, isEmpty);
  });
}
