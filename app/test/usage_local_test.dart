// The login each harness has where it runs is the account `local` of that harness, which the host lists with the rest: the Usage page
// shows one under each harness, asks the host for its limits by the harness it belongs to (the name alone is every harness's), and
// shows what sessions heard under the id that names the harness. Settings' lists are not made longer by it.
@Timeout(Duration(minutes: 2))
library;

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

// A Wednesday noon in the local zone.
final _noon = DateTime(2026, 11, 11, 12).millisecondsSinceEpoch ~/ 1000;

/// A host that answers each read by the account's name and the harness it was asked for, counting the reads, and lists [heard] as what
/// sessions reported.
final class _Host implements UsageSource, UsageReader {
  final rows = <String, Object?>{};
  final reads = <(String, String?)>[];
  final heard = <Object?>[];

  @override
  String get name => 'Laptop';

  @override
  Future<List<Object?>> limits() async => heard;

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};

  @override
  Future<Object?> read(String name, {String? harness}) async {
    reads.add((name, harness));
    return rows['$name/${harness ?? ''}'];
  }
}

Map<String, Object?> _window(String id, String label, int used, {required int hours, required int resetsIn}) => {
  'id': id,
  'label': label,
  'unit': 'permille',
  'used': used,
  'limit': 1000,
  'refill': {'kind': 'fixed_window', 'length': [hours * 3600, 0], 'resets_at': _noon + resetsIn},
  'status': 'ok',
  'observed_at': _noon - 30,
  'source': 'reported',
};

AccountsController _accounts() => AccountsController(
  host: FakeAccountsHost(accounts: [
    account('work', kind: 'claude-login', id: 'id-work'),
    harnessLogin('claude'),
    harnessLogin('codex'),
  ]),
  config: () => const CanoaConfig(),
  change: (_) async {},
);

Future<(UsageController, _Host)> _open(WidgetTester tester, void Function(_Host host) setUp) async {
  final host = _Host();
  setUp(host);
  final accounts = _accounts();
  await accounts.refresh();
  final usage = UsageController(sources: [host], accounts: accounts, now: () => _noon);
  tester.view
    ..physicalSize = const Size(1280, 1800)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    home: Scaffold(
      body: Row(children: [
        SizedBox(width: 260, child: UsageIndex(controller: usage)),
        Expanded(child: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
      ]),
    ),
  ));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
  await tester.pumpAndSettle();
  return (usage, host);
}

void main() {
  testWidgets('each harness has its own login on the page, read by the harness it belongs to', (tester) async {
    final (_, host) = await _open(tester, (host) {
      host.rows['work/'] = {'supported': true, 'buckets': <Object?>[]};
      host.rows['local/claude'] = {
        'supported': true,
        'account': 'local:claude',
        'buckets': [_window('five_hour', 'Session (5 hours)', 120, hours: 5, resetsIn: 3 * 3600)],
      };
      host.rows['local/codex'] = {'supported': false, 'account': 'local:codex', 'unsupported_because': 'the login Codex keeps for itself is not held here', 'buckets': <Object?>[]};
    });
    // `local` is asked once for each harness, and an account of canoa.cfg's names none.
    expect(host.reads.toSet(), {('work', null), ('local', 'claude'), ('local', 'codex')});

    // One card for each, under its harness, with the login's own title and not its name.
    for (final id in ['local:claude', 'local:codex']) {
      expect(find.byKey(Key('usage-account $id')), findsOneWidget);
      expect(find.byKey(Key('usage-index account $id')), findsOneWidget);
      expect(find.descendant(of: find.byKey(Key('usage-account $id')), matching: find.text("This machine's login")), findsOneWidget);
    }
    expect(find.text('local'), findsNothing);
    final claude = tester.getTopLeft(find.byKey(const Key('usage-provider claude'))).dy;
    final codex = tester.getTopLeft(find.byKey(const Key('usage-provider codex'))).dy;
    final claudeLogin = tester.getTopLeft(find.byKey(const Key('usage-account local:claude'))).dy;
    final codexLogin = tester.getTopLeft(find.byKey(const Key('usage-account local:codex'))).dy;
    expect(claudeLogin, greaterThan(claude));
    expect(claudeLogin, lessThan(codex));
    expect(codexLogin, greaterThan(codex));
    // Within its harness the login comes first, then the accounts of canoa.cfg.
    expect(tester.getTopLeft(find.byKey(const Key('usage-account id-work'))).dy, greaterThan(claudeLogin));

    // What each answered is its own: Claude's windows on Claude's login, Codex's reason on Codex's.
    expect(find.descendant(of: find.byKey(const Key('usage-account local:claude')), matching: find.text('88% left')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('usage-account local:codex')), matching: find.text('Not offered')), findsOneWidget);
    expect(find.text('the login Codex keeps for itself is not held here'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('usage-account local:codex')), matching: find.text('88% left')), findsNothing);
  });

  testWidgets('what sessions heard on a harness\'s login is shown on that harness\'s login only', (tester) async {
    final (usage, _) = await _open(tester, (host) {
      host.heard.add({
        'account': 'local:codex',
        'updated_at': _noon - 10,
        'observed_at': _noon - 10,
        'buckets': [_window('weekly', 'Weekly', 400, hours: 168, resetsIn: 4 * 86400)],
      });
    });
    final codex = usage.all.firstWhere((each) => each.id == 'local:codex');
    expect(codex.hosts, ['Laptop']);
    expect(codex.provider, 'codex');
    expect(codex.name, "This machine's login");
    expect(codex.report.buckets, isNotEmpty);
    final claude = usage.all.firstWhere((each) => each.id == 'local:claude');
    expect(claude.hosts, isEmpty);
    expect(claude.report.buckets, isEmpty);
    expect(find.descendant(of: find.byKey(const Key('usage-account local:codex')), matching: find.text('60% left')), findsOneWidget);
    expect(usage.accountName('local:codex'), "This machine's login");
    // No session ran on Claude's login, so it is not listed as one that reported.
    expect(find.byKey(const Key('usage-hosts local:claude')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('usage-hosts local:claude')), matching: find.textContaining('Reported by')), findsNothing);
  });

  testWidgets('the logins are listed with the accounts but are no account to offer, sign in again or remove', (tester) async {
    final accounts = _accounts();
    await accounts.refresh();
    expect([for (final each in accounts.listed) each.id], ['id-work', 'local:claude', 'local:codex']);
    // What Settings, the pools and the new-session picker read stays what canoa.cfg names.
    expect([for (final each in accounts.accounts) each.name], ['work']);
    expect(accounts.of('claude').map((each) => each.name), ['work']);
    expect(accounts.of('codex'), isEmpty);
    expect(accounts.named('local'), isNull);
    expect([for (final each in accounts.logins) (each.harness, each.id)], [('claude', 'local:claude'), ('codex', 'local:codex')]);
    expect(accounts.logins.every((each) => each.automatic), isTrue);
  });
}
