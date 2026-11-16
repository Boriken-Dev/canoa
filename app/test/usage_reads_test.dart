// The Usage page asks each account's provider when it opens and on refresh, with no session run: an account with windows shows
// them, one whose provider offers none says so with the reason, one with no figures yet says that instead, and one that could not
// be asked says why. Every word on the page is selectable and copies.
@Timeout(Duration(minutes: 2))
library;

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

// A Wednesday noon in the local zone.
final _noon = DateTime(2026, 11, 11, 12).millisecondsSinceEpoch ~/ 1000;

/// A host that lists nothing from sessions and answers each account's read from [rows], counting the reads.
final class _Host implements UsageSource, UsageReader {
  final rows = <String, Object?>{};
  final reads = <String>[];

  @override
  String get name => 'Laptop';

  @override
  Future<List<Object?>> limits() async => const [];

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};

  @override
  Future<Object?> read(String name) async {
    reads.add(name);
    final row = rows[name];
    if (row is Exception) throw row;
    return row;
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

Future<(UsageController, _Host)> _open(WidgetTester tester, void Function(_Host host) setUp) async {
  final host = _Host();
  setUp(host);
  final accounts = AccountsController(
    host: FakeAccountsHost(accounts: [
      account('work', kind: 'claude-login', id: 'id-work'),
      account('key', kind: 'anthropic-api-key', id: 'id-key'),
      account('fresh', kind: 'codex-chatgpt', harness: 'codex', id: 'id-fresh'),
      account('down', kind: 'claude-login', id: 'id-down'),
    ]),
    config: () => const CanoaConfig(),
    change: (_) async {},
  );
  await accounts.refresh();
  final usage = UsageController(sources: [host], accounts: accounts, now: () => _noon);
  tester.view
    ..physicalSize = const Size(1280, 1400)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  // The page asks when it opens: no refresh is called here.
  await tester.pumpWidget(MaterialApp(
    home: Scaffold(body: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
  ));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
  await tester.pumpAndSettle();
  return (usage, host);
}

void _rows(_Host host) {
  host.rows['work'] = {
    'supported': true,
    'observed_at': _noon - 30,
    'buckets': [
      _window('five_hour', 'Session (5 hours)', 120, hours: 5, resetsIn: 3 * 3600),
      _window('seven_day', 'Weekly', 400, hours: 168, resetsIn: 4 * 86400),
    ],
  };
  host.rows['key'] = {'supported': false, 'unsupported_because': 'an API key or gateway token is billed by use and has no plan limits', 'buckets': <Object?>[]};
  host.rows['fresh'] = {'supported': true, 'buckets': <Object?>[]};
  host.rows['down'] = const ControlFailed(-32002, 'Claude refused the login (status 401): renew it, then read again');
}

void main() {
  testWidgets('opening Usage reads every account with no session and says what each came to', (tester) async {
    final (_, host) = await _open(tester, _rows);
    expect(host.reads.toSet(), {'work', 'key', 'fresh', 'down'});
    // A Claude account with windows shows them.
    expect(find.byKey(const Key('account-left five_hour')), findsOneWidget);
    expect(find.text('88% left'), findsOneWidget);
    expect(find.text('60% left'), findsOneWidget);
    expect(find.byKey(const Key('usage-hosts id-work')), findsOneWidget);
    // An API key is not offered, with the provider's reason.
    expect(find.descendant(of: find.byKey(const Key('usage-account id-key')), matching: find.text('Not offered')), findsOneWidget);
    expect(find.text('an API key or gateway token is billed by use and has no plan limits'), findsOneWidget);
    // An account the provider answered with no figures is not reported yet, which is not the same.
    expect(find.descendant(of: find.byKey(const Key('usage-account id-fresh')), matching: find.text('Not reported yet')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('usage-account id-fresh')), matching: find.text('Not offered')), findsNothing);
    expect(find.byKey(const Key('usage-note id-fresh')), findsOneWidget);
    // A read that failed says why.
    expect(find.descendant(of: find.byKey(const Key('usage-account id-down')), matching: find.text('Could not be read')), findsOneWidget);
    expect(find.text('Claude refused the login (status 401): renew it, then read again'), findsOneWidget);
  });

  testWidgets('refresh reads every account again and shows the new figures', (tester) async {
    final (usage, host) = await _open(tester, _rows);
    expect(host.reads.length, 4);
    host.rows['work'] = {
      'supported': true,
      'buckets': [_window('five_hour', 'Session (5 hours)', 500, hours: 5, resetsIn: 3600)],
    };
    host.rows['key'] = {'supported': false, 'unsupported_because': 'a different reason', 'buckets': <Object?>[]};
    await tester.tap(find.byKey(const Key('usage-refresh')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(host.reads.length, 8);
    expect(usage.loading, isFalse);
    expect(find.text('50% left'), findsOneWidget);
    expect(find.text('a different reason'), findsOneWidget);
    expect(find.text('an API key or gateway token is billed by use and has no plan limits'), findsNothing);
  });

  testWidgets('the page copies: its figures, labels, reasons and errors are selectable text', (tester) async {
    await _open(tester, _rows);
    expect(find.byType(SelectionArea), findsOneWidget);
    var copied = '';
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') copied = (call.arguments as Map)['text'] as String;
      return null;
    });
    addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));
    final region = tester.state<SelectableRegionState>(find.byType(SelectableRegion));
    region.selectAll();
    await tester.pump();
    // ignore: deprecated_member_use
    region.copySelection(SelectionChangedCause.toolbar);
    await tester.pump();
    for (final word in ['88% left', 'Weekly', 'Not offered', 'an API key or gateway token is billed by use and has no plan limits', 'Not reported yet', 'Could not be read', 'Claude refused the login (status 401): renew it, then read again']) {
      expect(copied, contains(word));
    }
  });
}
