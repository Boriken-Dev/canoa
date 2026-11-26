// The Usage section's Limits group over a scripted host: each limit with its state, bar, reset and thresholds, a figure the host cannot
// measure said as not measured, a limit added, changed and removed as one `config/advanced_set` call, the host's refusal in its own
// words, one turn let through a stop, and the card of a refused turn. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set
// the same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/settings/advanced_config.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/usage/limit_rules.dart';
import 'package:canoa/usage/limits_group.dart';
import 'package:canoa/usage/usage_model.dart';
import 'package:canoa/usage/usage_section.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

// 2026-11-25 12:00 UTC, a Wednesday.
const _noon = 1795608000;

Map<String, Object?> _state(
  String name,
  String state, {
  String scope = 'all',
  String? of,
  String kind = 'tokens',
  int? used,
  int? limit,
  int? permille,
  String? bucket,
  int warn = 800,
  int? stop,
  String? span,
  String? why,
}) => {
  'name': name,
  'scope': scope,
  'name_of_scope': ?of,
  'figure': {'kind': kind, 'used': ?used, 'limit': ?limit, 'permille': ?permille, 'bucket': ?bucket},
  'state': state,
  'warn_permille': warn,
  'stop_permille': ?stop,
  if (span == 'week') 'window': {'start': 1795392000, 'resets_at': 1795996800} else if (span == 'day') 'window': {'start': 1795564800, 'resets_at': 1795651200},
  'why': ?why,
};

/// A host that answers `limits/state`, `limits/allow` and the two methods of the generic editor, and keeps the person's own entries.
final class _Host {
  _Host(this.rows, {Map<String, Map<String, Object?>>? own}) : own = own ?? {};

  List<Map<String, Object?>> rows;
  final Map<String, Map<String, Object?>> own;
  final calls = <(String, Map<String, Object?>)>[];
  String? refuse;

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'limits/state':
        return {'limits': rows};
      case 'limits/allow':
        return {'ok': true};
      case 'config/advanced':
        return {
          'revision': 'r${calls.length}',
          'tables': [
            {'path': ['host', 'usage', 'limits'], 'title': '[host.usage.limits]', 'shape': 'entries', 'schema': {'type': 'object', 'properties': {}}, 'own': own.isEmpty ? null : own, 'inherited': {}},
          ],
        };
      case 'config/advanced_set':
        if (refuse != null) throw ControlFailed(-32602, refuse!);
        final name = [for (final key in params['path']! as List) '$key'].last;
        if (params['value'] == null) {
          own.remove(name);
          rows = [for (final row in rows) if (row['name'] != name) row];
        } else {
          own[name] = (params['value']! as Map).cast<String, Object?>();
        }
        return {'revision': 'x', 'restart': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }

  List<Map<String, Object?>> sets() => [for (final (method, params) in calls) if (method == 'config/advanced_set') params];
}

final _rows = [
  _state('work_week', 'warn', scope: 'account', of: 'work', kind: 'usd', used: 23000000, limit: 25000000, permille: 920, stop: 1000, span: 'week'),
  _state('review_day', 'ok', scope: 'profile', of: 'review', used: 120000, limit: 1000000, permille: 120, span: 'day'),
  _state('plan', 'stop', scope: 'account', of: 'local:claude', kind: 'bucket', bucket: 'seven_day', permille: 1000, stop: 1000, used: 1000, limit: 1000),
  _state('team_money', 'unmeasured', scope: 'account', of: 'metered', kind: 'usd', limit: 10000000, why: 'this account reports no cost'),
];

Map<String, Map<String, Object?>> _own() => {
  'work_week': {'account': 'work', 'usd': 25.0, 'window': 'week', 'stop_permille': 1000},
  'review_day': {'profile': 'review', 'tokens': 1000000, 'window': 'day'},
};

Future<LimitsController> _controller(_Host host) async {
  final controller = LimitsController(host.call, AdvancedConfig(host.call));
  await controller.refresh();
  return controller;
}

Future<void> _pump(WidgetTester tester, LimitsController controller, {Size size = const Size(1100, 760), String language = 'en'}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(AppTextScope(
    text: AppText(language: language),
    child: MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(
        body: SingleChildScrollView(
          padding: const EdgeInsets.all(16),
          child: LimitsGroup(controller: controller, accounts: const ['work', 'metered'], profiles: const ['default', 'review'], clock: () => _noon),
        ),
      ),
    ),
  ));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/usage_limits_group_test.dart')));

  test('a limit is read from what limits/state lists', () {
    final row = LimitRow.fromJson(_rows.first);
    expect((row.name, row.scope, row.scopeName, row.kind, row.state), ('work_week', LimitScope.account, 'work', LimitKind.usd, LimitState.warn));
    expect((row.used, row.limit, row.permille, row.warnPermille, row.stopPermille), (23000000, 25000000, 920, 800, 1000));
    expect(row.resetsAt, isNotNull);
    final away = LimitRow.fromJson(_rows.last);
    expect((away.state, away.why), (LimitState.unmeasured, 'this account reports no cost'));
    expect(away.permille, isNull);
  });

  test('an entry is made from the form and read back into it', () {
    expect(limitEntry(scope: LimitScope.account, scopeName: 'work', kind: LimitKind.usd, figure: '25.5', window: 'week', stop: 1000), {
      'account': 'work',
      'usd': 25.5,
      'window': 'week',
      'stop_permille': 1000,
    });
    expect(limitEntry(scope: LimitScope.all, kind: LimitKind.bucket, figure: ' any ', window: 'week', warn: 700), {'bucket': 'any', 'warn_permille': 700});
    expect(limitEntry(scope: LimitScope.profile, scopeName: 'review', kind: LimitKind.tokens, figure: '5000000', window: 'day'), {
      'profile': 'review',
      'tokens': 5000000,
      'window': 'day',
    });
    final read = readEntry({'account': 'work', 'usd': 25.0, 'window': 'week', 'stop_permille': 1000});
    expect((read.scope, read.scopeName, read.kind, read.window, read.warn, read.stop), (LimitScope.account, 'work', LimitKind.usd, 'week', null, 1000));
  });

  testWidgets('each limit shows its state, bar, figure, reset and thresholds, and a figure not measured says why', (tester) async {
    final host = _Host(_rows, own: _own());
    await _pump(tester, await _controller(host));
    for (final (name, state) in [('work_week', 'Near the limit'), ('review_day', 'Within the limit'), ('plan', 'Stopped'), ('team_money', 'Not measured')]) {
      expect(find.byKey(Key('limit-row $name')), findsOneWidget, reason: name);
      expect(tester.widget<Text>(find.byKey(Key('limit-state $name'))).data, state, reason: name);
    }
    expect(tester.widget<Text>(find.byKey(const Key('limit-figure work_week'))).data, '92% · 23.00 of 25.00 USD');
    expect(tester.widget<Text>(find.byKey(const Key('limit-thresholds work_week'))).data, 'Warns from 80%, stops from 100%');
    expect(tester.widget<Text>(find.byKey(const Key('limit-thresholds review_day'))).data, 'Warns from 80%, never stops');
    expect(tester.widget<LinearProgressIndicator>(find.byKey(const Key('limit-bar work_week'))).value, closeTo(0.92, 0.001));
    expect(find.byKey(const Key('limit-resets work_week')), findsOneWidget);
    // Not measured is never drawn as 0 percent: no bar, no figure, and the reason.
    expect(find.byKey(const Key('limit-bar team_money')), findsNothing);
    expect(find.byKey(const Key('limit-figure team_money')), findsNothing);
    expect(tester.widget<Text>(find.byKey(const Key('limit-why team_money'))).data, 'Not measured: this account reports no cost');
    // A limit the person's own file holds is edited here; the allowance of the host's own login has none in this file.
    expect(find.byKey(const Key('limit-edit work_week')), findsOneWidget);
    expect(find.byKey(const Key('limit-edit plan')), findsNothing);
    // A stop offers one turn through.
    expect(find.byKey(const Key('limit-allow plan')), findsOneWidget);
    expect(find.byKey(const Key('limit-allow work_week')), findsNothing);
    expect(tester.takeException(), isNull);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_limits_group_desktop.png'));
  });

  testWidgets('at phone width the same rows stack', (tester) async {
    await _pump(tester, await _controller(_Host(_rows, own: _own())), size: const Size(390, 1500));
    expect(tester.takeException(), isNull);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_limits_group_phone.png'));
  });

  testWidgets('with no limit nothing warns or stops, and the page says so', (tester) async {
    await _pump(tester, await _controller(_Host([])));
    expect(find.byKey(const Key('limits-empty')), findsOneWidget);
    expect(find.byKey(const Key('limit-add')), findsOneWidget);
  });

  testWidgets('a limit is added with its form as one call, a refusal is shown in the host\'s words, and a name is not reused', (tester) async {
    final host = _Host(_rows, own: _own());
    await _pump(tester, await _controller(host));
    await tester.tap(find.byKey(const Key('limit-add')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('limit-form')), findsOneWidget);

    // Nothing is sent for a name the host would refuse, or one already used.
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('limit-problem')), findsOneWidget);
    await tester.enterText(find.byKey(const Key('limit-name')), 'work_week');
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(tester.widget<Text>(find.byKey(const Key('limit-problem'))).data, 'A limit called work_week exists.');
    expect(host.sets(), isEmpty);

    await tester.enterText(find.byKey(const Key('limit-name')), 'month_money');
    await tester.enterText(find.byKey(const Key('limit-figure')), '40');
    await tester.enterText(find.byKey(const Key('limit-stop')), '900');
    await tester.enterText(find.byKey(const Key('limit-warn')), '950');
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(tester.widget<Text>(find.byKey(const Key('limit-problem'))).data, 'A threshold is a whole number of thousandths, and the stop is at least the warning.');
    await tester.enterText(find.byKey(const Key('limit-warn')), '700');
    host.refuse = 'host.usage.limits.month_money: usd needs a window';
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(tester.widget<Text>(find.byKey(const Key('limit-problem'))).data, 'host.usage.limits.month_money: usd needs a window');

    host.refuse = null;
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(host.sets().last['path'], ['host', 'usage', 'limits', 'month_money']);
    expect(host.sets().last['value'], {'usd': 40, 'window': 'week', 'warn_permille': 700, 'stop_permille': 900});
    expect(find.byKey(const Key('limit-form')), findsNothing);
  });

  testWidgets('a limit is changed from its own entry and removed after asking', (tester) async {
    final host = _Host(_rows, own: _own());
    await _pump(tester, await _controller(host));
    await tester.tap(find.byKey(const Key('limit-edit review_day')));
    await tester.pumpAndSettle();
    // The form holds what the file holds, and a name is not asked again.
    expect(find.byKey(const Key('limit-name')), findsNothing);
    expect(tester.widget<TextField>(find.byKey(const Key('limit-figure'))).controller!.text, '1000000');
    expect(tester.widget<DropdownButton<LimitScope>>(find.byKey(const Key('limit-scope'))).value, LimitScope.profile);
    await tester.enterText(find.byKey(const Key('limit-figure')), '2000000');
    await tester.tap(find.byKey(const Key('limit-save')));
    await tester.pumpAndSettle();
    expect(host.sets().single['value'], {'profile': 'review', 'tokens': 2000000, 'window': 'day'});

    await tester.tap(find.byKey(const Key('limit-remove work_week')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('confirm-remove-no')));
    await tester.pumpAndSettle();
    expect(host.sets(), hasLength(1), reason: 'keeping it changes nothing');
    await tester.tap(find.byKey(const Key('limit-remove work_week')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('confirm-remove-yes')));
    await tester.pumpAndSettle();
    expect(host.sets().last, {'path': ['host', 'usage', 'limits', 'work_week'], 'value': null, 'revision': isA<String>()});
    expect(find.byKey(const Key('limit-row work_week')), findsNothing);
  });

  testWidgets('one turn is let through a stop by name, and the person is told', (tester) async {
    final host = _Host(_rows, own: _own());
    await _pump(tester, await _controller(host));
    await tester.tap(find.byKey(const Key('limit-allow plan')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.$1 == 'limits/allow').single.$2, {'name': 'plan'});
    expect(tester.widget<Text>(find.byKey(const Key('limits-told'))).data, 'The next turn will go through the limit plan.');
  });

  testWidgets('the words are the app\'s language', (tester) async {
    await _pump(tester, await _controller(_Host(_rows, own: _own())), language: 'es');
    expect(tester.widget<Text>(find.byKey(const Key('limit-state plan'))).data, 'Detenido');
    expect(tester.widget<Text>(find.byKey(const Key('limit-why team_money'))).data, 'Sin medir: esta cuenta no informa de ningún coste');
    expect(find.text('Añadir límite'), findsOneWidget);
  });

  testWidgets('a host that refuses the read says so and shows no limit', (tester) async {
    Future<Object?> refuse(String method, Map<String, Object?> params) async => throw ControlFailed(-32001, 'This key may not read the limits.');
    final controller = LimitsController(refuse, AdvancedConfig(refuse));
    await controller.refresh();
    await _pump(tester, controller);
    expect(tester.widget<Text>(find.byKey(const Key('limits-problem'))).data, 'This key may not read the limits.');
  });

  testWidgets('the Usage page carries the group under the warning, and the line that says the warning is every limit\'s default', (tester) async {
    final accounts = AccountsController(host: FakeAccountsHost(accounts: const []), config: () => const CanoaConfig(), change: (_) async {});
    final usage = UsageController(sources: const [], accounts: accounts, now: () => _noon);
    final host = _Host(_rows, own: _own());
    final limits = await _controller(host);
    tester.view
      ..physicalSize = const Size(1100, 2400)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {}, limits: limits)),
    ));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('limits-group')), findsOneWidget);
    expect(find.byKey(const Key('limits-warn-default')), findsOneWidget);
    // A page with no limits controller (a host that grants none) has no group.
    await tester.pumpWidget(MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: UsagePage(controller: usage, warnPermille: 800, onWarnChanged: (_) {})),
    ));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('limits-group')), findsNothing);
  });

  group('the card of a refused turn', () {
    final meta = {
      'stopReason': 'refusal',
      '_meta': {
        'dev.boriken.canoa/limit': {'limit': 'work_week', 'scope': 'account', 'used_permille': 1040, 'resets_at': _noon + 4 * 86400, 'figure': 'usd'},
      },
    };

    test('is read from the answer\'s _meta', () {
      final stop = LimitStop.of(meta)!;
      expect((stop.limit, stop.usedPermille, stop.resetsAt), ('work_week', 1040, _noon + 4 * 86400));
      expect(LimitStop.of({'stopReason': 'end_turn'}), isNull);
      expect(LimitStop.of({'_meta': {'dev.boriken.canoa/limit': 'x'}}), isNull);
    });

    testWidgets('names the limit and the reset, and offers to continue once and to open the limits', (tester) async {
      var continued = 0, opened = 0;
      tester.view
        ..physicalSize = const Size(700, 190)
        ..devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(AppTextScope(
        text: AppText.base,
        child: MaterialApp(
          theme: themeFor(LookId.canoa, Brightness.dark),
          home: Scaffold(body: Padding(padding: const EdgeInsets.all(16), child: LimitStopCard(stop: LimitStop.of(meta)!, onContinue: () => continued++, onOpenLimits: () => opened++))),
        ),
      ));
      await tester.pumpAndSettle();
      expect(find.text('The limit work_week stopped this turn'), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('limit-stop-figure'))).data, '104% used · Resets 2026-11-29 00:00 (UTC)'.replaceAll('2026-11-29 00:00', limitTime(_noon + 4 * 86400)));
      await tester.tap(find.byKey(const Key('limit-stop-continue')));
      await tester.tap(find.byKey(const Key('limit-stop-open')));
      expect((continued, opened), (1, 1));
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/usage_limit_stop_card_desktop.png'));
    });
  });
}
