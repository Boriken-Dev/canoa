// The Usage section's Tokens and Cost: the range, totals, each account's part, the daily chart and its table, and the breakdowns,
// over a fixture standing in for a host's `usage/summary`. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same
// scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/chat/inspector.dart';
import 'package:canoa/chat/git_changes.dart' show ChangeScope;
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/usage/chart.dart';
import 'package:canoa/usage/spend_model.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;
import 'fixtures/task_agent.dart';

final _now = DateTime.utc(2026, 11, 16, 12).millisecondsSinceEpoch ~/ 1000;
int _at(int month, int day) => DateTime.utc(2026, month, day, 9).millisecondsSinceEpoch ~/ 1000;

/// One recorded turn; a null counter was not reported.
final class _Line {
  const _Line(this.time, this.account, this.harness, this.model, this.session, {this.input, this.output, this.cacheRead, this.cacheWrite, this.cost});

  final int time;
  final String account, harness, session;
  final String? model;
  final int? input, output, cacheRead, cacheWrite, cost;
}

final _lines = [
  _Line(_at(11, 15), 'id-work', 'claude', 'opus', 's1', input: 1000, output: 200, cacheRead: 5000, cacheWrite: 300, cost: 1500000),
  _Line(_at(11, 10), 'id-work', 'claude', 'haiku', 's2', input: 400, output: 100),
  _Line(_at(11, 16), 'id-personal', 'codex', 'gpt', 's3', input: 50, output: 10, cost: 20000),
  _Line(_at(11, 1), 'ghost', 'claude', null, 's4', input: 70, output: 5),
  _Line(_at(8, 20), 'id-work', 'claude', 'opus', 's5', input: 9, output: 1),
];

/// What a host does with `usage/summary`: the lines in range and for the filters, summed by the group asked.
final class _Host implements UsageSource {
  _Host(this.name, [this.lines = const []]);

  @override
  final String name;
  final List<_Line> lines;
  Object? failure;
  final asked = <Map<String, Object?>>[];

  @override
  Future<List<Object?>> limits() async => const [];

  @override
  Future<Object?> summary(Map<String, Object?> params) async {
    asked.add(params);
    if (failure != null) throw failure!;
    final from = params['from'] as int? ?? 0, to = params['to'] as int? ?? 1 << 40;
    final kept = [
      for (final line in lines)
        if (line.time >= from &&
            line.time < to &&
            (params['account'] == null || params['account'] == line.account) &&
            (params['provider'] == null || params['provider'] == line.harness) &&
            (params['model'] == null || params['model'] == line.model) &&
            (params['session'] == null || params['session'] == line.session))
          line,
    ];
    String? key(_Line line) => switch (params['group_by']) {
      'account' => line.account,
      'provider' => line.harness,
      'model' => line.model,
      'session' => line.session,
      _ => DateTime.fromMillisecondsSinceEpoch(line.time * 1000, isUtc: true).toIso8601String().substring(0, 10),
    };
    Map<String, Object?> sum(Iterable<_Line> some, [Object? key = #none]) {
      int? add(int? Function(_Line) of) {
        final values = some.map(of).nonNulls;
        return values.isEmpty ? null : values.fold<int>(0, (a, b) => a + b);
      }

      return {
        if (key != #none) 'key': key,
        'records': some.length,
        'sessions': some.map((line) => line.session).toSet().length,
        'input': ?add((line) => line.input),
        'output': ?add((line) => line.output),
        'cache_read': ?add((line) => line.cacheRead),
        'cache_write': ?add((line) => line.cacheWrite),
        'cost_micro_usd': ?add((line) => line.cost),
      };
    }

    final groups = <String?, List<_Line>>{};
    for (final line in kept) {
      groups.putIfAbsent(key(line), () => []).add(line);
    }
    return {
      'from': from,
      'to': to,
      'group_by': params['group_by'],
      'timezone': 'UTC',
      'total': sum(kept),
      'groups': [for (final MapEntry(:key, :value) in groups.entries) sum(value, key)],
    };
  }
}

Future<UsageController> _controller(List<UsageSource> sources, UsageView view) async {
  final accounts = AccountsController(
    host: FakeAccountsHost(accounts: [account('work', kind: 'claude-login', id: 'id-work'), account('personal', kind: 'codex-login', harness: 'codex', id: 'id-personal')]),
    config: () => const CanoaConfig(),
    change: (_) async {},
  );
  await accounts.refresh();
  final usage = UsageController(sources: sources, accounts: accounts, now: () => _now);
  usage.setView(view);
  if (view != UsageView.limits) await usage.refreshSpend();
  return usage;
}

Future<void> _pump(WidgetTester tester, UsageController usage, {Size size = const Size(1280, 900), bool index = true}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: Row(children: [
        if (index) SizedBox(width: 260, child: UsageIndex(controller: usage)),
        Expanded(child: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
      ]),
    ),
  ));
  await tester.pumpAndSettle();
}

String _tile(WidgetTester tester, String label) =>
    tester.widgetList<Text>(find.descendant(of: find.byKey(Key('spend-total $label')), matching: find.byType(Text))).map((text) => text.data).join('|');

String _chart(WidgetTester tester) => tester.widget<UsageBarChart>(find.byType(UsageBarChart)).description;

Future<void> _setRange(WidgetTester tester, SpendRange range) async {
  await tester.ensureVisible(find.byKey(Key('spend-range ${range.name}')));
  await tester.tap(find.byKey(Key('spend-range ${range.name}')));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/usage_spend_test.dart')));

  testWidgets('the range changes the totals, the chart and its table together', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _pump(tester, usage);
    expect(usage.range, SpendRange.week);
    expect(_tile(tester, 'Sessions'), 'Sessions|3');
    expect(_tile(tester, 'Input (uncached)'), 'Input (uncached)|1,450');
    expect(find.byKey(const Key('spend-day 2026-11-10')), findsOneWidget);
    expect(find.byKey(const Key('spend-day 2026-11-01')), findsNothing);
    expect(_chart(tester), 'Tokens by day, 8 days');

    await _setRange(tester, SpendRange.month);
    expect(_tile(tester, 'Sessions'), 'Sessions|4');
    expect(_tile(tester, 'Input (uncached)'), 'Input (uncached)|1,520');
    expect(find.byKey(const Key('spend-day 2026-11-01')), findsOneWidget);
    expect(_chart(tester), 'Tokens by day, 31 days');

    await _setRange(tester, SpendRange.quarter);
    expect(_tile(tester, 'Sessions'), 'Sessions|5');
    expect(find.byKey(const Key('spend-day 2026-08-20')), findsOneWidget);

    await _setRange(tester, SpendRange.day);
    expect(_tile(tester, 'Sessions'), 'Sessions|1', reason: 'only the turn on the 16th');
    expect(tester.widget<Text>(find.byKey(const Key('spend-zone'))).data, '2026-11-15 to 2026-11-16 · days are in UTC');
  });

  testWidgets('a counter no agent reported reads Not reported, not zero', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _pump(tester, usage);
    expect(_tile(tester, 'Cache read'), 'Cache read|5,000');
    usage.selectAccount('id-personal');
    await tester.pumpAndSettle();
    expect(_tile(tester, 'Cache read'), 'Cache read|Not reported');
    expect(_tile(tester, 'Cache write'), 'Cache write|Not reported');
    expect(_tile(tester, 'Output'), 'Output|10');
    expect(find.text('Not reported'), findsWidgets);
  });

  testWidgets('an account no list names reads Unknown, and a known one reads by name', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _setRangeAfter(tester, usage, SpendRange.month);
    expect(find.descendant(of: find.byKey(const Key('spend-account ghost')), matching: find.text('Unknown')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('spend-account id-work')), matching: find.text('work')), findsOneWidget);
    expect(usage.accountName(''), 'Unknown');
    expect(usage.accountName(null), 'Unknown');
  });

  testWidgets('a narrow window stacks the chart over its table, and a wide one sets them side by side', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _pump(tester, usage, size: const Size(390, 844), index: false);
    expect(find.byKey(const Key('spend-daily narrow')), findsOneWidget);
    expect(find.byKey(const Key('spend-daily wide')), findsNothing);
    expect(tester.takeException(), isNull);
    // Four columns of tokens never fit beside the chart; the cost's one does.
    await _pump(tester, usage);
    expect(find.byKey(const Key('spend-daily narrow')), findsOneWidget);
    usage.setView(UsageView.cost);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('spend-daily wide')), findsOneWidget);
  });

  testWidgets('the cost is only what agents reported, says so, and leaves an unreported total unestimated', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.cost);
    await _pump(tester, usage);
    expect(_tile(tester, 'Reported cost'), 'Reported cost|\$1.52');
    expect(tester.widget<Text>(find.byKey(const Key('spend-note'))).data, contains('nothing is estimated'));
    expect(tester.widget<Text>(find.byKey(const Key('spend-note'))).data, contains('subscription fees are not included'));
    usage.selectAccount('id-work');
    await tester.pumpAndSettle();
    expect(_tile(tester, 'Reported cost'), 'Reported cost|\$1.50');
    // The 10th's turn reported no cost: its day says so.
    expect(tester.widget<Text>(find.byKey(const Key('spend-day 2026-11-10 Reported cost'))).data, 'Not reported');
    usage.selectAccount(null);
    usage.selectModel('haiku');
    await tester.pumpAndSettle();
    expect(_tile(tester, 'Reported cost'), 'Reported cost|Not reported');
  });

  testWidgets('a row drills in, keeps the range, and its chip lets go', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _pump(tester, usage, size: const Size(1280, 2400));
    await _setRange(tester, SpendRange.month);
    await tester.ensureVisible(find.byKey(const Key('spend-breakdown model')));
    await tester.tap(find.byKey(const Key('spend-breakdown model')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('spend-row opus')), findsOneWidget);
    await tester.tap(find.byKey(const Key('spend-row opus')));
    await tester.pumpAndSettle();
    expect(usage.model, 'opus');
    expect(usage.range, SpendRange.month);
    expect(find.byKey(const Key('spend-chip model')), findsOneWidget);
    expect(_tile(tester, 'Sessions'), 'Sessions|1');
    await tester.tap(find.descendant(of: find.byKey(const Key('spend-chip model')), matching: find.byIcon(Icons.clear)));
    await tester.pumpAndSettle();
    expect(usage.model, isNull);
    expect(_tile(tester, 'Sessions'), 'Sessions|4');

    // A day drills into that day, as a custom range.
    await tester.ensureVisible(find.byKey(const Key('spend-breakdown day')));
    await tester.tap(find.byKey(const Key('spend-breakdown day')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('spend-row 2026-11-10')));
    await tester.pumpAndSettle();
    expect(usage.range, SpendRange.custom);
    expect(_tile(tester, 'Sessions'), 'Sessions|1');
    expect(_tile(tester, 'Input (uncached)'), 'Input (uncached)|400');
  });

  testWidgets('a breakdown sorts by the column tapped, and by name', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines)], UsageView.tokens);
    await _setRangeAfter(tester, usage, SpendRange.month, size: const Size(1280, 2000));
    List<String> order() => [
      for (final row in tester.widget<DataTable>(find.byKey(const Key('spend-breakdown-table'))).rows)
        (row.key! as ValueKey<String>).value.substring('spend-row '.length),
    ];
    expect(order(), ['id-work', 'ghost', 'id-personal'], reason: 'the most input first');
    Future<void> byName() async {
      final header = find.descendant(of: find.byKey(const Key('spend-breakdown-table')), matching: find.text('Account'));
      await tester.ensureVisible(header);
      await tester.tap(header);
      await tester.pumpAndSettle();
    }

    await byName();
    // By the name shown: personal, work, then Unknown last when descending reversed.
    expect(order().first, 'id-personal');
    await byName();
    expect(order().first, 'id-work');
  });

  testWidgets('a host that answers nothing is listed as not covered, and the rest still count', (tester) async {
    final down = _Host('Desktop')..failure = 'no link';
    final usage = await _controller([_Host('Laptop', _lines), down], UsageView.tokens);
    await _pump(tester, usage);
    expect(find.text('Desktop is not covered: it did not answer (no link)'), findsOneWidget);
    expect(_tile(tester, 'Sessions'), 'Sessions|3');
  });

  testWidgets('the view switch and the filters ask the host only for sums, never for records', (tester) async {
    final host = _Host('Laptop', _lines);
    final usage = await _controller([host], UsageView.limits);
    await _pump(tester, usage);
    expect(host.asked, isEmpty, reason: 'Limits needs no sums');
    await tester.tap(find.text('Tokens'));
    await tester.pumpAndSettle();
    expect(host.asked.map((each) => each['group_by']), containsAll(['day', 'account']));
    expect(host.asked.every((each) => each['from'] is int && each['to'] is int), isTrue);
  });

  testWidgets('two hosts add up, and the Host breakdown lists each', (tester) async {
    final usage = await _controller([_Host('Laptop', _lines), _Host('Desktop', [_lines[2]])], UsageView.tokens);
    usage.setBreakdown(Breakdown.host);
    await usage.refreshSpend();
    await _pump(tester, usage);
    expect(_tile(tester, 'Sessions'), 'Sessions|4');
    expect(find.byKey(const Key('spend-row Laptop')), findsOneWidget);
    expect(find.byKey(const Key('spend-row Desktop')), findsOneWidget);
  });

  testWidgets('the session details show that session\'s tokens apart from its context', (tester) async {
    final (agent, session, store) = await showTask(tester, const Size(500, 900));
    agent.usage(200, 1000);
    final asked = <String>[];
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SessionInspector(
          store: store,
          session: session,
          scopes: const [ChangeScope.reported],
          scope: ChangeScope.reported,
          onScope: (_) {},
          onOpenDiff: () {},
          onClose: () {},
          sessionSpend: (id) async {
            asked.add(id);
            return const SpendRow(records: 2, sessions: 1, input: 1200, output: 300);
          },
        ),
      ),
    ));
    await tester.pumpAndSettle();
    expect(asked, [session.remoteSessionId]);
    expect(find.text('20% used'), findsOneWidget);
    expect(find.byKey(const ValueKey('tokens Input (uncached)')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const ValueKey('tokens Input (uncached)'))).data, '1,200');
    expect(tester.widget<Text>(find.byKey(const ValueKey('tokens Output'))).data, '300');
    expect(tester.widget<Text>(find.byKey(const ValueKey('tokens Cache read'))).data, 'Not reported');
  });

  for (final (label, size, index) in [('desktop', const Size(1280, 1500), true), ('phone', const Size(390, 2000), false)]) {
    for (final view in [UsageView.tokens, UsageView.cost]) {
      testWidgets('Usage, ${view.label} at $label width', (tester) async {
        final usage = await _controller([_Host('Laptop', _lines)], view);
        await _setRangeAfter(tester, usage, SpendRange.month, size: size, index: index);
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_${view.name}_$label.png'));
      });
    }
  }
}

Future<void> _setRangeAfter(WidgetTester tester, UsageController usage, SpendRange range, {Size size = const Size(1280, 900), bool index = true}) async {
  usage.setRange(range);
  await usage.refreshSpend();
  await _pump(tester, usage, size: size, index: index);
}
