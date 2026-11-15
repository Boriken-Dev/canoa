// What the panel tells the person about the Canoa on this computer, in each state, against a bridge that answers as the extension's
// worker does (bindings/browser/chromium/acp-local.js).
import 'dart:async';
import 'dart:convert';

import 'package:canoa/extension/local_host.dart';
import 'package:canoa/extension/local_host_banner.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _peer = 'sha256:aa';

String _json(Map<String, Object?> map) => jsonEncode(map);

LocalHostView _view(Map<String, Object?> map) => LocalHostView.fromJson(_json(map));

final class _Bridge implements LocalHostBridge {
  _Bridge(this.next);

  LocalHostView next;
  final calls = <String>[];
  Completer<void>? holdStart;

  @override
  Future<LocalHostView> status() async {
    calls.add('status');
    return next;
  }

  @override
  Future<LocalHostView> start() async {
    calls.add('start');
    await holdStart?.future;
    return next = _view({
      'state': 'running',
      'paired': true,
      'host': {'name': 'Canoa', 'peer': _peer, 'entries': [{'name': 'claude', 'label': 'Claude'}]},
    });
  }

  @override
  Future<LocalHostView> pair(String pin) async {
    calls.add('pair $pin');
    return next = _view({
      'state': 'running',
      'paired': true,
      'host': {'name': 'Canoa', 'peer': _peer, 'entries': [{'name': 'claude', 'label': 'Claude'}]},
    });
  }

  @override
  Future<LocalHostView> unpair() async {
    calls.add('unpair');
    return next = _view({'state': 'running', 'paired': false, 'canPin': true, 'host': {'name': 'Canoa', 'peer': _peer, 'entries': []}});
  }
}

Future<void> _show(WidgetTester tester, _Bridge bridge) async {
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: LocalHostBanner(bridge: bridge))));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('the app shows the banner above the conversation when it runs as the extension, and none otherwise', (tester) async {
    final store = SessionStore();
    addTearDown(store.dispose);
    await tester.binding.setSurfaceSize(const Size(380, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(CanoaApp(store: store, localHost: _Bridge(_view({'state': 'stopped'}))));
    await tester.pump();
    await tester.pump();
    expect(find.text('Canoa on this computer is not running.'), findsOneWidget);
    expect(find.text('Start Canoa'), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();
    expect(find.byType(LocalHostBanner), findsNothing);
  });

  testWidgets('with no native host, it says Canoa is not installed and offers nothing to start', (tester) async {
    final bridge = _Bridge(_view({'state': 'not-installed'}));
    await _show(tester, bridge);
    expect(find.textContaining('Canoa is not installed on this computer'), findsOneWidget);
    expect(find.text('Start Canoa'), findsNothing);
    expect(find.text('Pair'), findsNothing);
  });

  testWidgets('a native host that does not allow this extension says so', (tester) async {
    await _show(tester, _Bridge(_view({'state': 'not-installed', 'forbidden': true})));
    expect(find.textContaining('does not know this extension'), findsOneWidget);
  });

  testWidgets('with Canoa stopped, Start Canoa starts it once and then shows it in use', (tester) async {
    final bridge = _Bridge(_view({'state': 'stopped'}))..holdStart = Completer<void>();
    await _show(tester, bridge);
    expect(find.text('Canoa on this computer is not running.'), findsOneWidget);
    await tester.tap(find.text('Start Canoa'));
    await tester.pump();
    expect(find.text('Starting…'), findsOneWidget);
    // A second click while it starts does nothing: the button is off.
    expect(tester.widget<FilledButton>(find.byType(FilledButton)).onPressed, isNull);
    bridge.holdStart!.complete();
    await tester.pumpAndSettle();
    expect(bridge.calls.where((call) => call == 'start'), hasLength(1));
    expect(find.textContaining('Using Canoa on this computer: Claude'), findsOneWidget);
  });

  testWidgets('with Canoa running and this browser not paired, a PIN pairs it', (tester) async {
    final bridge = _Bridge(_view({'state': 'running', 'paired': false, 'canPin': true, 'host': {'name': 'Canoa', 'peer': _peer, 'entries': []}}));
    await _show(tester, bridge);
    expect(find.textContaining('Pair the browser extension'), findsOneWidget);
    await tester.enterText(find.byType(TextField), ' 123456 ');
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();
    expect(bridge.calls, contains('pair 123456'));
    expect(find.textContaining('Using Canoa'), findsOneWidget);
  });

  testWidgets('a Canoa that is not the one this browser paired with asks to be paired again', (tester) async {
    await _show(tester, _Bridge(_view({'state': 'running', 'paired': false, 'changed': true, 'canPin': true, 'host': {'name': 'Canoa', 'peer': _peer, 'entries': []}})));
    expect(find.textContaining('not the Canoa this browser was paired with'), findsOneWidget);
  });

  testWidgets('a host whose proof a PIN does not pair says what to run', (tester) async {
    await _show(tester, _Bridge(_view({'state': 'running', 'paired': false, 'canPin': false, 'host': {'name': 'Canoa', 'peer': _peer, 'entries': []}})));
    expect(find.textContaining('canoa accept'), findsOneWidget);
    expect(find.byType(TextField), findsNothing);
  });

  testWidgets('a refusal is said in its words and leaves the banner as it was', (tester) async {
    final bridge = _FailingBridge();
    await _show(tester, bridge);
    await tester.tap(find.text('Start Canoa'));
    await tester.pumpAndSettle();
    expect(find.text('Canoa could not be started'), findsOneWidget);
    expect(find.text('Start Canoa'), findsOneWidget);
  });
}

final class _FailingBridge extends _Bridge {
  _FailingBridge() : super(_view({'state': 'stopped'}));

  @override
  Future<LocalHostView> start() async => throw Exception('Canoa could not be started');
}
