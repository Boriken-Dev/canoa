// An account's usage as Claude's and Codex's usage pages say it: how much is left, a countdown to the reset and a sentence on the
// pace, from what the sessions on the account last heard. Settings shows it under the account.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/accounts/usage_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

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
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/accounts_usage_test.dart')));

  test('N% left, from what is used', () {
    expect(leftText(_weekly(usedPermille: 620, resetsInSeconds: 3600), _noon), '38% left');
    expect(leftText(_weekly(usedPermille: 0, resetsInSeconds: 3600), _noon), '100% left');
    expect(leftText(const UsageBucket(id: 'x'), _noon), isNull);
  });

  test('a little left is not none and a little used is not none used', () {
    // 0.78% left reads 1%, as Antigravity's own `/usage` says it; 99.6% left reads 99%.
    expect(leftText(_weekly(usedPermille: 992, resetsInSeconds: 3600), _noon), '1% left');
    expect(leftText(_weekly(usedPermille: 4, resetsInSeconds: 3600), _noon), '99% left');
    expect(leftText(_weekly(usedPermille: 1000, resetsInSeconds: 3600), _noon), '0% left');
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

  // Antigravity's three groups as `agy` lists them: each a set of models with its own weekly and five-hour window, one of which
  // does not apply while the weekly one is used up.
  UsageBucket window(String id, String group, String label, List<String> names, {int? left, bool disabled = false, int days = 0, int hours = 0}) => UsageBucket(
    id: id,
    label: label,
    group: group,
    modelNames: names,
    limit: disabled ? null : 1000,
    remaining: disabled ? null : left,
    status: disabled ? 'disabled' : (left == 0 ? 'exhausted' : 'ok'),
    refill: UsageRefill('fixed_window', length: Duration(days: days, hours: hours), resetsAt: _noon + (days * 24 + hours) * 3600 - 600),
  );

  testWidgets('the view lists each group once with its models and the windows of it, one that does not apply said so', (tester) async {
    const gemini = ['Gemini 4 Neon', 'Gemini 3.8 Flash'];
    const claude = ['Claude Opus 5.5', 'GPT-OSS 120B'];
    final report = UsageReport(supported: true, buckets: [
      window('gemini-weekly', 'Gemini Models', 'Weekly Limit Remaining', gemini, left: 8, days: 7),
      window('gemini-5h', 'Gemini Models', 'Five Hour Limit Remaining', gemini, left: 1000, hours: 5),
      window('3p-weekly', 'Claude and GPT models', 'Weekly Limit Remaining', claude, left: 0, days: 7),
      window('3p-5h', 'Claude and GPT models', 'Five Hour Limit Remaining', claude, disabled: true, hours: 5),
      window('argon-weekly', 'Gemini 4 Argon', 'Gemini 4 Argon Weekly Limit', ['Gemini 4 Argon'], left: 0, days: 7),
    ]);
    tester.view.physicalSize = const Size(640, 440);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(debugShowCheckedModeBanner: false, home: Scaffold(body: SingleChildScrollView(child: AccountUsageView(report: report, now: _noon)))));
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_groups_desktop.png'));

    for (final group in ['Gemini Models', 'Claude and GPT models', 'Gemini 4 Argon']) {
      expect(find.byKey(Key('account-group $group')), findsOneWidget);
      expect(find.text(group), findsOneWidget);
    }
    // The group's models are said once under its name, not under each of its windows.
    expect(find.text('Models: Gemini 4 Neon, Gemini 3.8 Flash'), findsOneWidget);
    expect(find.text('Models: Claude Opus 5.5, GPT-OSS 120B'), findsOneWidget);
    expect(find.text('Models: Gemini 4 Argon'), findsOneWidget);
    expect(find.byKey(const Key('account-models gemini-5h')), findsNothing);
    // A group is above its windows, and the windows keep the report's order, not the one by what is used.
    double y(String key) => tester.getTopLeft(find.byKey(Key(key))).dy;
    expect(y('account-group Gemini Models'), lessThan(y('account-usage gemini-weekly')));
    expect(y('account-usage gemini-weekly'), lessThan(y('account-usage gemini-5h')));
    expect(y('account-usage gemini-5h'), lessThan(y('account-group Claude and GPT models')));
    expect(y('account-group Claude and GPT models'), lessThan(y('account-usage 3p-weekly')));
    expect(y('account-usage 3p-weekly'), lessThan(y('account-usage 3p-5h')));
    expect(y('account-usage 3p-5h'), lessThan(y('account-group Gemini 4 Argon')));

    expect(find.text('1% left'), findsOneWidget);
    expect(find.text('100% left'), findsOneWidget);
    expect(find.text('Disabled'), findsOneWidget);
    expect(find.byKey(const Key('account-left 3p-5h')), findsOneWidget);
    // A window that does not apply has no time of its own to reset at.
    expect(find.byKey(const Key('account-reset 3p-5h')), findsNothing);
    expect(find.text('0% left'), findsNWidgets(2));
    // The disabled window has no bar: four of the five windows have one.
    expect(find.byType(LinearProgressIndicator), findsNWidgets(4));
  });

  testWidgets('a bucket that is in no group says which models it covers under it', (tester) async {
    final report = UsageReport(supported: true, buckets: [
      UsageBucket(id: 'five_hour', label: 'Session (5 hours)', models: UsageCoverage.all, used: 120, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(hours: 5), resetsAt: _noon + 3600)),
      UsageBucket(id: 'seven_day_opus', label: 'Weekly, Opus', models: const UsageCoverage('only', ['opus']), modelNames: const ['Opus'], used: 300, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: _noon + 86400)),
      UsageBucket(id: 'oauth_apps', label: 'Weekly, OAuth apps', used: 10, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: _noon + 86400)),
    ]);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountUsageView(report: report, now: _noon)))));

    expect(find.text('Models: All models'), findsOneWidget);
    expect(find.text('Models: Opus'), findsOneWidget);
    // A bucket that says nothing of its models gets no line.
    expect(find.byKey(const Key('account-models oauth_apps')), findsNothing);
    expect(find.byKey(const Key('account-models five_hour')), findsOneWidget);
    expect(find.byKey(const Key('account-models seven_day_opus')), findsOneWidget);
  });

  // Reports as the harnesses answer them today, recorded with made-up figures: Claude's weekly window of the Fable model, which
  // the plan lists beside the plain weekly one, and Antigravity's Gemini pool that names Argon among its models, with the group of
  // its own that earlier answers gave it.
  testWidgets('the view shows the Fable window of Claude and the Argon models of Antigravity', (tester) async {
    final recorded = jsonDecode(File('test/fixtures/usage_fable_and_argon.json').readAsStringSync()) as Map<String, Object?>;
    final claude = UsageReport.parse(recorded['claude'])!;
    final agy = UsageReport.parse(recorded['antigravity'])!;

    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountUsageView(report: claude, now: _noon)))));
    expect(find.text('Weekly, Fable'), findsOneWidget);
    expect(find.text('Models: Fable'), findsOneWidget);
    expect(find.text('39% left'), findsOneWidget);
    expect(find.text('Models: All models'), findsOneWidget);

    expect([for (final section in usageSections(agy, _noon)) section.$1], ['Gemini Models', 'Gemini 4 Argon']);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountUsageView(report: agy, now: _noon)))));
    expect(find.byKey(const Key('account-group Gemini 4 Argon')), findsOneWidget);
    expect(find.text('Models: Gemini 4 Neon, Gemini 4 Argon, Gemini 3.8 Flash'), findsOneWidget);
    expect(find.text('Models: Gemini 4 Argon'), findsOneWidget);
    expect(find.text('7% left'), findsOneWidget);
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
