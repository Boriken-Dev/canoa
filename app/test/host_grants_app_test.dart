// Settings › Accounts and the program Store follow what the host grants the key the app reaches it with: the app's own host (the
// desktop, a key that holds every grant) and a host that served the page (a paired browser key) go through the same widgets, and a
// host that grants neither shows neither and is never asked for them.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_grants.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_host.dart';

/// A control channel whose host holds [grants]; it records what it was asked and answers the reads the app makes.
final class _Control implements HostLink {
  _Control(this.grants);

  final List<String> grants;
  final asked = <String>[];

  /// The params of each request, by method.
  final params = <String, List<Map<String, Object?>>>{};
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    asked.add(method);
    params.putIfAbsent(method, () => []).add((request['params'] as Map?)?.cast<String, Object?>() ?? const {});
    final Object? result = switch (method) {
      'host/grants' => {'grants': grants},
      'accounts/list' => {
        'accounts': [
          {'name': 'work', 'harness': 'claude', 'kind': 'anthropic-api-key', 'id': 'id-work', 'state': 'signed in'},
        ],
      },
      'accounts/entries' => {
        'entries': [
          {'name': 'Claude', 'harness': 'claude', 'accounts': <String>[], 'open': true},
          {'name': 'Codex', 'harness': 'codex', 'accounts': <String>[], 'open': true},
        ],
      },
      'accounts/offer' => {'restart': true},
      'host/restart' => {'restarting': true},
      'fs/list' => {'path': '', 'parent': null, 'entries': <Object?>[], 'more': false},
      'store/list' => {'stores': <Object?>[]},
      'ledger/list' => {'records': <Object?>[]},
      _ => null,
    };
    _out.add(jsonEncode(result == null
        ? {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32001, 'message': 'no $method grant'}}
        : {'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() => _out.close();
}

/// An older host: it has no `host/grants`.
final class _Old implements HostLink {
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32601, 'message': 'no such method'}}));
  }

  @override
  Future<void> close() => _out.close();
}

final _peer = 'sha256:${'a' * 64}';
const _all = ['ledger.read', 'ledger.write', 'store.read', 'store.programs', 'accounts'];

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

/// The app on its own host, whose control channel is [control].
Future<void> _desktop(WidgetTester tester, _Control control) => pumpApp(
  tester,
  _store(),
  host: linkedHost((name) => name == '@control' ? control : throw StateError('nothing starts in this test: $name')),
);

/// The app on a host that served the page, reached over [control].
Future<void> _browser(WidgetTester tester, _Control control) {
  final host = ServedHost(
    peer: _peer,
    method: PeerMethod.noise,
    entries: const [],
    control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
  );
  return pumpApp(
    tester,
    _store(),
    host: ServedSupervisor(host),
    served: ServedApp.over(host, HostControl.dialing(() async => control)),
  );
}

Future<void> _openSettings(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings').evaluate().isNotEmpty ? find.byTooltip('Settings') : find.text('Settings').first);
  await frames(tester);
}

void main() {
  group('HostGrants', () {
    HostGrants grants(_Control control) => HostGrants(() async => ControlClient(control));

    test('holds what the host says, and nothing before it has said', () async {
      final held = grants(_Control(['accounts']));
      expect(held.known, isFalse);
      expect(held.may(HostGrants.accounts), isFalse);
      await held.refresh();
      expect(held.may(HostGrants.accounts), isTrue);
      expect(held.may(HostGrants.programs), isFalse);
    });

    test('a host that cannot be reached leaves it unknown, and one with no such method grants nothing', () async {
      final unreachable = HostGrants(() async => throw StateError('down'));
      await unreachable.refresh();
      expect(unreachable.known, isFalse);
      final old = HostGrants(() async => ControlClient(_Old()));
      await old.refresh();
      expect(old.known, isTrue);
      expect(old.may(HostGrants.programs) || old.may(HostGrants.accounts), isFalse);
    });
  });

  group('offering an account to an entry', () {
    for (final (label, open) in [('on its own host', _desktop), ('on a host that served the page', _browser)]) {
      testWidgets('$label asks the host to offer it, which has the host start again', (tester) async {
        final control = _Control(_all);
        await open(tester, control);
        await _openSettings(tester);
        await tester.tap(find.byKey(const Key('settings-nav-accounts')));
        await frames(tester);
        // The entries are the host's, built-in sources included: no canoa.cfg on the app's side names Claude.
        await tester.tap(find.byKey(const Key('account-menu work')));
        await frames(tester);
        await tester.tap(find.byKey(const Key('account-offer work Claude')));
        await frames(tester);
        // An entry open to every paired key asks first.
        await tester.tap(find.byKey(const Key('offer-open-entry-confirm')));
        await frames(tester);
        await tester.pump(const Duration(seconds: 1));
        expect(control.params['accounts/offer'], [
          {'entry': 'Claude', 'accounts': ['work']},
        ]);
        if (label.contains('served')) expect(control.asked, contains('host/restart'));
      });
    }
  });

  group('the app', () {
    for (final (label, open) in [('on its own host', _desktop), ('on a host that served the page', _browser)]) {
      testWidgets('$label shows Accounts and asks for the programs when the host grants both', (tester) async {
        final control = _Control(_all);
        await open(tester, control);
        await _openSettings(tester);
        expect(find.byKey(const Key('settings-nav-accounts')), findsOneWidget);
        expect(control.asked, containsAll(['host/grants', 'accounts/list']));
      });

      testWidgets('$label shows no Accounts and never asks for them when the host grants neither', (tester) async {
        final control = _Control(['ledger.read', 'ledger.write', 'store.read']);
        await open(tester, control);
        await _openSettings(tester);
        expect(find.byKey(const Key('settings-nav-accounts')), findsNothing);
        expect(control.asked, contains('host/grants'));
        expect(control.asked, isNot(contains('accounts/list')));
      });
    }
  });
}
