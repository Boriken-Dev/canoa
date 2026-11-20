// What a page read from its host is read again when the link to the host ends and comes back (the host restarted): an account made
// meanwhile shows without a reload, and Usage lists every account, asking each provider for what it can report with no session.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';

/// A control channel of a host with [accounts]; it records what it was asked.
final class _Control implements HostLink {
  _Control(this.accounts);

  final List<String> accounts;
  final asked = <String>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    asked.add(method);
    final Object? result = switch (method) {
      'host/grants' => {
        'grants': ['accounts', 'usage', 'ledger.read'],
      },
      'accounts/list' => {
        'accounts': [
          for (final name in accounts) {'name': name, 'harness': 'claude', 'kind': 'claude-login', 'id': 'id-$name', 'state': 'signed in'},
        ],
      },
      'accounts/entries' => {'entries': <Object?>[]},
      'usage/limits' => {'accounts': <Object?>[]},
      'ledger/list' => {'records': <Object?>[]},
      _ => null,
    };
    _out.add(jsonEncode(result == null
        ? {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32001, 'message': 'no'}}
        : {'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  void hostEnds() => unawaited(_out.close());

  @override
  Future<void> close() async {
    if (!_out.isClosed) await _out.close();
  }
}

/// Dials a new [_Control] each time and keeps them.
final class _Dialer {
  final links = <_Control>[];

  Future<HostLink> dial() async {
    links.add(_Control(links.isEmpty ? const [] : const ['work']));
    return links.last;
  }
}

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

final class _Source implements UsageSource, UsageReader {
  _Source(this.entries, {this.reads = const {}, this.failReads = false});

  final List<Object?> entries;
  final Map<String, Object?> reads;
  final bool failReads;
  final readNames = <String>[];

  @override
  String get name => 'Laptop';

  @override
  Future<List<Object?>> limits() async => entries;

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};

  @override
  Future<Object?> read(String name, {String? harness}) async {
    readNames.add(name);
    if (failReads) throw StateError('the host cannot read');
    return reads[name];
  }
}

Map<String, Object?> _weekly(int observedAt) => {
  'id': 'weekly',
  'label': 'Weekly',
  'unit': 'permille',
  'used': 250,
  'limit': 1000,
  'refill': {'kind': 'unknown'},
  'status': 'ok',
  'observed_at': observedAt,
  'source': 'reported',
};

void main() {
  test('a link the host ended is made again while something listens, and says so', () async {
    final dialer = _Dialer();
    final control = HostControl.dialing(dialer.dial);
    final heard = <void>[];
    final listening = control.reconnected.listen(heard.add);
    final first = await control.ready();
    expect(dialer.links, hasLength(1));
    expect(heard, isEmpty, reason: 'the first link is not a coming back');
    dialer.links.first.hostEnds();
    await first.ended;
    for (var waited = 0; heard.isEmpty && waited < 40; waited++) {
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    expect(heard, hasLength(1));
    expect(dialer.links, hasLength(2));
    await listening.cancel();
    await control.close();
  });

  test('a link nobody waits on is not tried again by itself', () async {
    final dialer = _Dialer();
    final control = HostControl.dialing(dialer.dial);
    final first = await control.ready();
    dialer.links.first.hostEnds();
    await first.ended;
    await Future<void>.delayed(const Duration(milliseconds: 700));
    expect(dialer.links, hasLength(1));
    await control.close();
  });

  testWidgets('an account made while the host restarted shows without a reload', (tester) async {
    final dialer = _Dialer();
    final host = ServedHost(
      peer: 'sha256:${'a' * 64}',
      method: PeerMethod.noise,
      entries: const [],
      control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
    );
    await pumpApp(
      tester,
      _store(),
      host: ServedSupervisor(host),
      served: ServedApp.over(host, HostControl.dialing(dialer.dial)),
    );
    await settle(tester);
    expect(dialer.links, isNotEmpty);
    expect(dialer.links.first.asked, contains('accounts/list'));
    final before = dialer.links.length;
    dialer.links.last.hostEnds();
    await tester.pump(const Duration(seconds: 1));
    await settle(tester, 200);
    expect(dialer.links.length, greaterThan(before), reason: 'the page made a link again');
    expect(dialer.links.last.asked, containsAll(['host/grants', 'accounts/list']));
    await finish(tester);
  });

  group('Usage', () {
    UsageController build(List<UsageSource> sources, AccountsController accounts) => UsageController(sources: sources, accounts: accounts, now: () => 1000);

    Future<AccountsController> accountsOf(List<String> names) async {
      final accounts = AccountsController(
        host: FakeAccountsHost(accounts: [for (final name in names) account(name, kind: 'claude-login', id: 'id-$name')]),
        config: () => const CanoaConfig(),
        change: (_) async {},
      );
      await accounts.refresh();
      return accounts;
    }

    test('lists an account no session has run on, as not reported', () async {
      final usage = build([_Source(const [])], await accountsOf(['work']));
      await usage.refresh();
      expect(usage.all.map((each) => each.name), ['work']);
      expect(usage.all.single.hosts, isEmpty);
      expect(usage.all.single.report.buckets, isEmpty);
    });

    test('shows the limits a provider reports on demand, and leaves an account its provider cannot read as not reported', () async {
      final source = _Source(const [], reads: {
        'work': {
          'account': 'id-work',
          'observed_at': 900,
          'buckets': [_weekly(900)],
        },
        'idle': {'account': 'id-idle', 'supported': false, 'unsupported_because': 'learns usage only in a turn', 'buckets': <Object?>[]},
      });
      final usage = build([source], await accountsOf(['work', 'idle']));
      await usage.refresh();
      final work = usage.all.firstWhere((each) => each.name == 'work');
      expect(work.report.buckets.single.id, 'weekly');
      expect(work.hosts, ['Laptop']);
      final idle = usage.all.firstWhere((each) => each.name == 'idle');
      expect(idle.hosts, isEmpty);
      expect(idle.report.buckets, isEmpty);
      expect(source.readNames, ['work', 'idle']);
    });

    test('a host that cannot read leaves what its sessions reported', () async {
      final source = _Source([
        {
          'account': 'id-work',
          'updated_at': 5,
          'observed_at': 5,
          'buckets': [_weekly(5)],
        },
      ], failReads: true);
      final usage = build([source], await accountsOf(['work']));
      await usage.refresh();
      expect(usage.all.single.report.buckets.single.id, 'weekly');
    });
  });
}
