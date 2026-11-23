// An account's usage as Claude's and Codex's usage pages say it: how much is left, a countdown to the reset and a sentence on the
// pace, from what the sessions on the account last heard. Settings shows it under the account.
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/accounts/usage_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

// A Wednesday noon in the local zone, so the day names the forecast says are fixed.
final _noon = DateTime(2026, 11, 11, 12).millisecondsSinceEpoch ~/ 1000;

UsageBucket _weekly({required int usedPermille, required int resetsInSeconds, int lengthDays = 7}) => UsageBucket(
  id: 'weekly',
  label: 'Weekly',
  used: usedPermille,
  limit: 1000,
  refill: UsageRefill('fixed_window', length: Duration(days: lengthDays), resetsAt: _noon + resetsInSeconds),
);

void main() {
  test('N% left, from what is used', () {
    expect(leftText(_weekly(usedPermille: 620, resetsInSeconds: 3600), _noon), '38% left');
    expect(leftText(_weekly(usedPermille: 0, resetsInSeconds: 3600), _noon), '100% left');
    expect(leftText(const UsageBucket(id: 'x'), _noon), isNull);
  });

  test('the figures of a key say where they came from, and a plan window says nothing of it', () {
    UsageBucket from(String source, {String unit = 'tokens'}) => UsageBucket(id: 'x', unit: UsageUnit(unit), used: 5, limit: 10, source: source);
    expect(sourceText(from('response_headers')), "From the provider's rate-limit headers");
    expect(sourceText(from('gateway', unit: 'micro_usd')), "From the gateway's key information");
    expect(sourceText(from('recorded')), 'Counted by Canoa from the sessions on this key');
    expect(sourceText(from('reported')), isNull);
  });

  test('a countdown to the reset', () {
    expect(resetText(_weekly(usedPermille: 100, resetsInSeconds: 3 * 3600 + 20 * 60), _noon), 'Resets in 3 h 20 m');
    expect(resetText(_weekly(usedPermille: 100, resetsInSeconds: -5), _noon), 'Reset, not yet reported');
  });

  test('the forecast says when the allowance runs out at the pace so far, and only where there is a pace', () {
    // Half used after 3.5 of 7 days: it lasts exactly to the reset.
    final steady = _weekly(usedPermille: 500, resetsInSeconds: 3 * 86400 + 12 * 3600);
    expect(forecastText(steady, _noon), 'On pace to last until it resets');
    // 40% used after 2 of 7 days: empty 3 days on, Saturday noon.
    expect(forecastText(_weekly(usedPermille: 400, resetsInSeconds: 5 * 86400), _noon), "At this pace you'll run out Saturday afternoon");
    // 80% used after 2 days: empty at midnight, which the day after says as tomorrow night.
    expect(forecastText(_weekly(usedPermille: 800, resetsInSeconds: 5 * 86400), _noon), "At this pace you'll run out tomorrow night");
    // Within the hour of the day, the time is said.
    final soon = _weekly(usedPermille: 900, resetsInSeconds: 12 * 3600, lengthDays: 1);
    expect(forecastText(soon, _noon), startsWith("At this pace you'll run out at "));
    // A window that has just begun, nothing used, an exhausted bucket and an unknown refill say nothing of a pace.
    expect(forecastText(_weekly(usedPermille: 50, resetsInSeconds: 7 * 86400 - 600), _noon), isNull);
    expect(forecastText(_weekly(usedPermille: 0, resetsInSeconds: 3 * 86400), _noon), isNull);
    expect(forecastText(const UsageBucket(id: 'x', used: 5, limit: 10), _noon), isNull);
  });

  testWidgets('the view shows each bucket with its figure, bar, reset and forecast, and the empty case in words', (tester) async {
    final report = UsageReport(supported: true, account: 'id-work', buckets: [
      _weekly(usedPermille: 800, resetsInSeconds: 5 * 86400),
      UsageBucket(
        id: 'session',
        label: 'Session',
        used: 100,
        limit: 1000,
        refill: UsageRefill('fixed_window', length: const Duration(hours: 5), resetsAt: _noon + 4 * 3600),
      ),
    ]);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: AccountUsageView(report: report, now: _noon))));
    expect(find.text('20% left'), findsOneWidget);
    expect(find.text('90% left'), findsOneWidget);
    expect(find.text('Resets in 5 d'), findsOneWidget);
    expect(find.text('Resets in 4 h'), findsOneWidget);
    expect(find.byKey(const Key('account-pace weekly')), findsOneWidget);
    expect(find.byType(LinearProgressIndicator), findsNWidgets(2));
    await tester.pumpWidget(const MaterialApp(home: Scaffold(body: AccountUsageView(report: null))));
    expect(find.byKey(const Key('account-usage-empty')), findsOneWidget);
  });

  testWidgets('Settings shows the usage the sessions on an account last heard, by the account\'s id', (tester) async {
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login', id: 'id-work')]);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final report = UsageReport(supported: true, account: 'id-work', buckets: [
      UsageBucket(id: 'weekly', label: 'Weekly', used: 250, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: now + 86400)),
    ]);
    final controller = AccountsController(
      host: host,
      config: () => const CanoaConfig(),
      change: (_) async {},
      usage: (id) => id == 'id-work' ? report : null,
    );
    tester.view.physicalSize = const Size(900, 1400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('account-usage-toggle work')));
    await tester.pump();
    expect(find.byKey(const Key('account-left weekly')), findsOneWidget);
    expect(find.text('75% left'), findsOneWidget);
  });
}
