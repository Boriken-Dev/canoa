// The login each harness has where it runs is the account `local` of that harness, which the host lists with the rest: the Usage page
// shows one under each harness, asks the host for its limits by the harness it belongs to (the name alone is every harness's), and
// shows what sessions heard under the id that names the harness. Settings' lists are not made longer by it.
@Timeout(Duration(minutes: 2))
library;

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:material_ui/material_ui.dart';
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

/// The accounts the host lists: canoa.cfg's `work` and the login of each harness in [logins], which the host lists only for a harness that is
/// signed in on its computer.
AccountsController _accounts(List<String> logins) => AccountsController(
  host: FakeAccountsHost(accounts: [
    account('work', kind: 'claude-login', id: 'id-work'),
    for (final harness in logins) harnessLogin(harness),
  ]),
  config: () => const CanoaConfig(),
  change: (_) async {},
);

Future<(UsageController, _Host)> _open(
  WidgetTester tester,
  void Function(_Host host) setUp, {
  List<String> logins = const ['claude', 'codex'],
}) async {
  final host = _Host();
  setUp(host);
  final accounts = _accounts(logins);
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

  testWidgets('a harness the host lists no login for has no card and is not asked', (tester) async {
    final (usage, host) = await _open(tester, (host) {
      host.rows['work/'] = {'supported': true, 'buckets': <Object?>[]};
      host.rows['local/claude'] = {'supported': true, 'account': 'local:claude', 'buckets': <Object?>[]};
    }, logins: const ['claude']);
    expect(host.reads.toSet(), {('work', null), ('local', 'claude')});
    expect(find.byKey(const Key('usage-account local:codex')), findsNothing);
    expect(find.byKey(const Key('usage-index account local:codex')), findsNothing);
    expect(usage.all.where((each) => each.id == 'local:codex'), isEmpty);
  });

  testWidgets('a login that is a key of the environment says so, naming the variable, and has its own card', (tester) async {
    final accounts = AccountsController(
      host: FakeAccountsHost(accounts: [harnessLogin('claude'), harnessLogin('codex', via: 'environment key (OPENAI_API_KEY)')]),
      config: () => const CanoaConfig(),
      change: (_) async {},
    );
    await accounts.refresh();
    final usage = UsageController(sources: [_Host()], accounts: accounts, now: () => _noon);
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    expect(usage.all.firstWhere((each) => each.id == 'local:codex').name, 'Environment key (OPENAI_API_KEY)');
    expect(usage.all.firstWhere((each) => each.id == 'local:claude').name, "This machine's login");
  });

  testWidgets('a login the provider could not look for is listed with its reason, and one it found has no such note', (tester) async {
    const reason = 'the keyring did not answer in time';
    final accounts = AccountsController(
      host: FakeAccountsHost(accounts: [harnessLogin('claude', note: reason), harnessLogin('codex')]),
      config: () => const CanoaConfig(),
      change: (_) async {},
    );
    await accounts.refresh();
    expect([for (final each in accounts.logins) each.id], ['local:claude', 'local:codex']);
    expect(accounts.logins.first.note, reason);
    expect(accounts.logins.last.note, isNull);
    final host = _Host()
      ..rows['local/claude'] = {'supported': true, 'account': 'local:claude', 'buckets': <Object?>[]}
      ..rows['local/codex'] = {'supported': true, 'account': 'local:codex', 'buckets': <Object?>[]};
    final usage = UsageController(sources: [host], accounts: accounts, now: () => _noon);
    tester.view
      ..physicalSize = const Size(1280, 1800)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(body: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
    ));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(usage.all.firstWhere((each) => each.id == 'local:claude').loginNote, reason);
    expect(find.byKey(const Key('usage-account local:claude')), findsOneWidget);
    expect(find.text('Not checked whether it is signed in: $reason'), findsOneWidget);
    expect(find.byKey(const Key('usage-login-note local:claude')), findsOneWidget);
    // The login that was found says nothing of the sort, and the account is listed as any other.
    expect(find.byKey(const Key('usage-account local:codex')), findsOneWidget);
    expect(find.byKey(const Key('usage-login-note local:codex')), findsNothing);
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
    final accounts = _accounts(const ['claude', 'codex']);
    await accounts.refresh();
    expect([for (final each in accounts.listed) each.id], ['id-work', 'local:claude', 'local:codex']);
    // What Settings, the pools and the new-session picker read stays what canoa.cfg names.
    expect([for (final each in accounts.accounts) each.name], ['work']);
    expect(accounts.of('claude_accounts').map((each) => each.name), ['work']);
    expect(accounts.of('codex_accounts'), isEmpty);
    expect(accounts.named('local'), isNull);
    expect([for (final each in accounts.logins) (each.agent, each.id)], [('claude', 'local:claude'), ('codex', 'local:codex')]);
    expect(accounts.logins.every((each) => each.automatic), isTrue);
  });

  testWidgets('unlisted or legacy codex usage groups under codex instead of Other', (tester) async {
    final (usage, _) = await _open(tester, (host) {
      host.heard.addAll([
        {
          'account': 'codex:local',
          'updated_at': _noon - 10,
          'observed_at': _noon - 10,
          'buckets': [_window('weekly', 'Weekly', 250, hours: 168, resetsIn: 4 * 86400)],
        },
        {
          'account': 'codex',
          'updated_at': _noon - 20,
          'observed_at': _noon - 20,
          'buckets': [_window('session', 'Session', 150, hours: 5, resetsIn: 3600)],
        },
        {
          'account': 'id-remote-codex',
          'provider': 'codex_accounts',
          'updated_at': _noon - 30,
          'observed_at': _noon - 30,
          'buckets': [_window('weekly', 'Weekly', 350, hours: 168, resetsIn: 5 * 86400)],
        },
        {
          'account': 'id-other',
          'updated_at': _noon - 40,
          'observed_at': _noon - 40,
          'buckets': [_window('weekly', 'Weekly', 100, hours: 168, resetsIn: 2 * 86400)],
        },
      ]);
    });
    expect(find.byKey(const Key('usage-provider codex')), findsOneWidget);
    expect(find.byKey(const Key('usage-index provider codex')), findsOneWidget);
    expect(find.byKey(const Key('usage-provider Other')), findsOneWidget);
    for (final id in ['codex:local', 'codex', 'id-remote-codex']) {
      final entry = usage.all.firstWhere((each) => each.id == id);
      expect(entry.provider, 'codex');
      expect(entry.provider, isNot(otherProvider));
    }
    expect(usage.all.firstWhere((each) => each.id == 'codex:local').name, "This machine's login");
    expect(usage.all.firstWhere((each) => each.id == 'id-other').provider, otherProvider);
    expect(usage.accountName('codex:local'), "This machine's login");
  });
}
