// The Usage section's Limits: each account's allowances from what the hosts reported, the index and the filters sharing one
// selection, one allowance for an account two hosts reported, and a reading said to be old. Goldens are drawn in the test font;
// with CANOA_SAMPLES_DIR set the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/usage_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/settings/settings_page.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

// A Wednesday noon in the local zone, so the day names the forecast says are fixed.
final _noon = DateTime(2026, 11, 11, 12).millisecondsSinceEpoch ~/ 1000;

final class _Source implements UsageSource {
  _Source(this.name, this.entries);

  @override
  final String name;
  List<Object?> entries;
  Object? failure;

  @override
  Future<List<Object?>> limits() async {
    if (failure != null) throw failure!;
    return entries;
  }

  @override
  Future<Object?> summary(Map<String, Object?> params) async => const {};
}

Map<String, Object?> _window(String id, String label, int usedPermille, {required int hours, required int resetsIn}) => {
  'id': id,
  'label': label,
  'unit': 'permille',
  'used': usedPermille,
  'limit': 1000,
  'refill': {'kind': 'fixed_window', 'length': [hours * 3600, 0], 'resets_at': _noon + resetsIn},
  'status': usedPermille >= 1000 ? 'exhausted' : 'ok',
  'observed_at': 0,
  'source': 'reported',
};

Map<String, Object?> _entry(String account, List<Map<String, Object?>> buckets, {required int readAgo}) => {
  'account': account,
  'updated_at': _noon - readAgo,
  'observed_at': _noon - readAgo,
  'buckets': buckets,
};

final _work = [_window('session', 'Session (5 h)', 120, hours: 5, resetsIn: 3 * 3600), _window('weekly', 'Weekly', 400, hours: 168, resetsIn: 4 * 86400)];
final _personal = [_window('weekly', 'Weekly', 880, hours: 168, resetsIn: 2 * 86400)];

AccountsController _accounts() {
  final controller = AccountsController(
    host: FakeAccountsHost(accounts: [
      account('work', kind: 'claude-login', id: 'id-work'),
      account('personal', kind: 'codex-login', harness: 'codex', id: 'id-personal'),
    ]),
    config: () => const CanoaConfig(),
    change: (_) async {},
  );
  return controller;
}

Future<UsageController> _controller(List<UsageSource> sources) async {
  final accounts = _accounts();
  await accounts.refresh();
  final usage = UsageController(sources: sources, accounts: accounts, now: () => _noon);
  await usage.refresh();
  return usage;
}

Future<void> _pump(WidgetTester tester, UsageController usage, {Size size = const Size(1280, 800), bool index = true}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: Row(children: [
        if (index) SizedBox(width: 260, child: UsageIndex(controller: usage)),
        Expanded(child: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {}, onManageAccounts: () {})),
      ]),
    ),
  ));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/usage_limits_test.dart')));

  testWidgets('the index lists All accounts, then each account under its provider, and picking one filters the view', (tester) async {
    final usage = await _controller([_Source('Laptop', [_entry('id-work', _work, readAgo: 60), _entry('id-personal', _personal, readAgo: 60)])]);
    await _pump(tester, usage);
    expect(find.byKey(const Key('usage-index all')), findsOneWidget);
    expect(find.byKey(const Key('usage-index provider claude')), findsOneWidget);
    expect(find.byKey(const Key('usage-index account id-work')), findsOneWidget);
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
    expect(find.byKey(const Key('usage-account id-personal')), findsOneWidget);

    await tester.tap(find.byKey(const Key('usage-index account id-personal')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('usage-account id-work')), findsNothing);
    expect(find.byKey(const Key('usage-account id-personal')), findsOneWidget);
    // The toolbar's filters say the same choice.
    expect(tester.widget<DropdownButton<String?>>(find.byKey(const Key('usage-filter account'))).value, 'id-personal');
    expect(tester.widget<DropdownButton<String?>>(find.byKey(const Key('usage-filter provider'))).value, 'codex');

    // A provider alone shows its accounts, and All accounts shows every one again.
    await tester.tap(find.byKey(const Key('usage-index provider claude')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
    expect(find.byKey(const Key('usage-account id-personal')), findsNothing);
    await tester.tap(find.byKey(const Key('usage-index all')));
    await tester.pumpAndSettle();
    expect(find.byType(Card), findsNWidgets(2));
  });

  testWidgets('the toolbar\'s Account filter narrows the view and widens it again', (tester) async {
    final usage = await _controller([_Source('Laptop', [_entry('id-work', _work, readAgo: 60), _entry('id-personal', _personal, readAgo: 60)])]);
    await _pump(tester, usage);
    await tester.tap(find.byKey(const Key('usage-filter account')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('work').last);
    await tester.pumpAndSettle();
    expect(find.byType(Card), findsOneWidget);
    expect(usage.account, 'id-work');
    expect(find.byKey(const Key('usage-index account id-work')).evaluate().isNotEmpty, isTrue);
    await tester.tap(find.byKey(const Key('usage-filter account')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('All accounts').last);
    await tester.pumpAndSettle();
    expect(find.byType(Card), findsNWidgets(2));
  });

  testWidgets('two hosts reporting one account show one allowance, the newest reading, and say which hosts', (tester) async {
    final newer = [_window('session', 'Session (5 h)', 700, hours: 5, resetsIn: 2 * 3600), _window('weekly', 'Weekly', 500, hours: 168, resetsIn: 4 * 86400)];
    final usage = await _controller([
      _Source('Laptop', [_entry('id-work', _work, readAgo: 600)]),
      _Source('Desktop', [_entry('id-work', newer, readAgo: 30), _entry('id-other', _personal, readAgo: 30)]),
    ]);
    await _pump(tester, usage);
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
    expect(find.byKey(const Key('usage-hosts id-work')), findsOneWidget);
    expect(find.text('Reported by Laptop, Desktop'), findsOneWidget);
    // One session bar, and it is the Desktop's newer 30% left, not the Laptop's 88%.
    expect(find.byKey(const Key('account-left session')), findsOneWidget);
    expect(find.text('30% left'), findsOneWidget);
    expect(find.text('88% left'), findsNothing);
    // An identity only one host reported stays apart, under the provider the app does not know it by.
    expect(find.byKey(const Key('usage-account id-other')), findsOneWidget);
    expect(find.text('Reported by Desktop'), findsOneWidget);
    expect(find.byKey(const Key('usage-provider Other')), findsOneWidget);
    // The Host filter limits to one host's reports.
    usage.selectHost('Laptop');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('usage-account id-other')), findsNothing);
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
  });

  testWidgets('a reading that is old says when it was read, and a fresh one says so too', (tester) async {
    final usage = await _controller([_Source('Laptop', [_entry('id-work', _work, readAgo: 3 * 3600), _entry('id-personal', _personal, readAgo: 120)])]);
    await _pump(tester, usage);
    final work = find.descendant(of: find.byKey(const Key('usage-account id-work')), matching: find.byKey(const Key('account-read weekly')));
    final personal = find.descendant(of: find.byKey(const Key('usage-account id-personal')), matching: find.byKey(const Key('account-read weekly')));
    expect(tester.widget<Text>(work).data, 'Last read 3 h ago; it may have changed since');
    expect(tester.widget<Text>(personal).data, 'Read 2 min ago');
  });

  testWidgets('a bucket that is low says so in words, not by colour alone', (tester) async {
    final usage = await _controller([_Source('Laptop', [_entry('id-personal', _personal, readAgo: 60)])]);
    await _pump(tester, usage);
    expect(find.byKey(const Key('account-state weekly')), findsOneWidget);
    expect(find.text('Running low'), findsOneWidget);
    expect(find.text('12% left'), findsOneWidget);
  });

  testWidgets('a host that cannot answer is named, and the others still show', (tester) async {
    final down = _Source('Desktop', [])..failure = 'no link';
    final usage = await _controller([_Source('Laptop', [_entry('id-work', _work, readAgo: 60)]), down]);
    await _pump(tester, usage);
    expect(find.text('Desktop did not answer: no link'), findsOneWidget);
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
  });

  testWidgets('an account no host reported is listed, and says it is not reported yet', (tester) async {
    final usage = await _controller([_Source('Laptop', [])]);
    await _pump(tester, usage);
    expect(find.byKey(const Key('usage-account id-work')), findsOneWidget);
    expect(find.byKey(const Key('usage-account id-personal')), findsOneWidget);
    expect(find.byKey(const Key('usage-hosts id-work')), findsOneWidget);
    expect(find.textContaining('Not reported yet'), findsNWidgets(2));
    expect(find.byKey(const Key('account-usage-empty')), findsNWidgets(2));
  });

  testWidgets('with no account at all the page says why nothing shows, in words', (tester) async {
    final usage = UsageController(sources: [_Source('Laptop', [])], now: () => _noon);
    await usage.refresh();
    await _pump(tester, usage);
    expect(find.byKey(const Key('usage-limits-empty')), findsOneWidget);
  });

  test('the account picker says the shortest window and the longest, as what is left', () {
    final report = UsageReport.parse({'supported': true, 'buckets': [for (final b in [..._work]) b]})!;
    expect(windowSummary(report, _noon), '5 h 88% left · week 60% left');
    expect(windowSummary(UsageReport.parse({'supported': true, 'buckets': _personal})!, _noon), 'week 12% left');
    expect(windowSummary(null, _noon), isNull);
    expect(windowSummary(const UsageReport(supported: true), _noon), isNull);
  });

  testWidgets('Settings no longer lists Usage when the app has the section, and still does when it has not', (tester) async {
    Future<void> settings({required bool section}) async {
      tester.view
        ..physicalSize = const Size(1280, 800)
        ..devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: SettingsPage(key: UniqueKey(), config: const CanoaConfig(), apply: (config) async => config, usageSection: section),
      ));
      await tester.pumpAndSettle();
    }

    await settings(section: false);
    expect(find.byKey(const Key('settings-nav-usage')), findsOneWidget);
    await settings(section: true);
    expect(find.byKey(const Key('settings-nav-usage')), findsNothing);
  });

  for (final (label, size, index) in [('desktop', const Size(1280, 800), true), ('phone', const Size(390, 844), false)]) {
    testWidgets('Usage, Limits at $label width', (tester) async {
      final usage = await _controller([
        _Source('Laptop', [_entry('id-work', _work, readAgo: 3 * 3600), _entry('id-personal', _personal, readAgo: 120)]),
      ]);
      await _pump(tester, usage, size: size, index: index);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_limits_$label.png'));
    });
  }

  testWidgets('Usage, Limits with an account no host has reported yet', (tester) async {
    final usage = await _controller([
      _Source('Laptop', [_entry('id-work', _work, readAgo: 120)]),
    ]);
    await _pump(tester, usage);
    expect(tester.takeException(), isNull);
    expect(find.byKey(const Key('usage-account id-personal')), findsOneWidget);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_limits_unreported_desktop.png'));
  });
}
