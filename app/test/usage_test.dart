// The usage control beside the mode: hidden for an agent that reports none, "No usage yet" before the first report, the bucket
// that matters most with how it refills, its colour, a menu of every bucket, and a session's spend in the transcript's header.
import 'package:canoa/chat/composer.dart' show UsageMenu;
import 'package:canoa/chat/usage.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

int get _now => DateTime.now().millisecondsSinceEpoch ~/ 1000;

Map<String, Object?> _bucket(
  String id, {
  String? label,
  String unit = 'permille',
  int? used,
  int? limit = 1000,
  Map<String, Object?> refill = const {'kind': 'unknown'},
  String status = 'ok',
  String source = 'reported',
  Map<String, Object?> models = const {'kind': 'all'},
  int? observedAt,
  String? evidence,
}) => {
  'id': id,
  'label': ?label,
  'models': models,
  'unit': unit,
  'used': ?used,
  'limit': ?limit,
  'refill': refill,
  'status': status,
  'observed_at': observedAt ?? _now,
  'source': source,
  'evidence': ?evidence,
};

Map<String, Object?> _report(
  List<Map<String, Object?>> buckets, {
  int revision = 1,
  String? account,
  Map<String, Object?>? spend,
  Map<String, Object?>? limit,
}) => {
  'supported': true,
  'sources': ['reported'],
  'account': ?account,
  'revision': revision,
  'observed_at': _now,
  'buckets': buckets,
  'limit': ?limit,
  'spend': ?spend,
};

Map<String, Object?> _window(int seconds) => {
  'kind': 'fixed_window',
  'length': [18000, 0],
  'resets_at': _now + seconds,
};

SessionStore _store() => SessionStore(includePreview: false, ledger: SessionLedger(MemoryLedgerBackend()));

Future<void> _started(WidgetTester tester, OptionsAgent agent, SessionStore store, {CanoaConfig? config}) async {
  await pumpApp(tester, store, startAgent: agent.start, config: config ?? const CanoaConfig(agents: [scripted]));
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
  await send(tester, 'Say hello');
  await settle(tester, 150);
}

Finder get _pill => find.byKey(const Key('usage'));

Finder get _label => find.descendant(of: _pill, matching: find.byType(Text));

String _shown(WidgetTester tester) => tester.widget<Text>(_label).data!;

Color? _colour(WidgetTester tester) => tester.widget<Text>(_label).style?.color;

void main() {
  group('the control', () {
    testWidgets('is hidden when the agent reports no usage', (tester) async {
      final agent = OptionsAgent();
      await _started(tester, agent, _store());
      expect(_pill, findsNothing);
      expect(agent.methods, isNot(contains('_coqui_acp/usage')), reason: 'nothing is asked of an agent that did not advertise it');
      await finish(tester);
    });

    testWidgets('is hidden when the agent answers that it is not supported', (tester) async {
      final agent = OptionsAgent()
        ..usage = {
          'supported': false,
          'unsupported_because': 'this harness reports no usage',
          'sources': <Object?>[],
          'revision': 0,
          'observed_at': 0,
          'buckets': <Object?>[],
        };
      await _started(tester, agent, _store());
      expect(_pill, findsNothing);
      await finish(tester);
    });

    testWidgets('says "No usage yet" when it is supported and nothing is reported', (tester) async {
      final agent = OptionsAgent()..usage = _report([]);
      await _started(tester, agent, _store());
      expect(_shown(tester), 'No usage yet');
      await finish(tester);
    });

    testWidgets('asks to be told of changes when it starts the connection', (tester) async {
      final agent = OptionsAgent()..usage = _report([]);
      await _started(tester, agent, _store());
      expect(agent.usageOptIn, {
        'dev.boriken.coqui.acp/usage': {'notify': true},
      });
      await finish(tester);
    });

    testWidgets('sits right after the mode', (tester) async {
      final agent = OptionsAgent()..usage = _report([]);
      await _started(tester, agent, _store());
      final mode = tester.getTopLeft(find.byKey(const Key('option-mode'))).dx;
      final usage = tester.getTopLeft(_pill).dx;
      final add = tester.getTopLeft(find.byKey(const Key('composer-plus'))).dx;
      expect(usage, greaterThan(mode));
      expect(usage, lessThan(add));
      await finish(tester);
    });

    testWidgets('shows a percentage with the time to the reset of a fixed window', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 620, refill: _window(4830))]);
      await _started(tester, agent, _store());
      expect(_shown(tester), '5 hours · 62% · resets in 1 h 20 m');
      await finish(tester);
    });

    testWidgets('shows tokens used of the limit', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([_bucket('weekly', label: 'Weekly', unit: 'tokens', used: 120000, limit: 400000, refill: _window(90030))]);
      await _started(tester, agent, _store());
      expect(_shown(tester), 'Weekly · 120k / 400k tokens · resets in 1 d 1 h');
      await finish(tester);
    });

    testWidgets('shows money used of the limit, and a balance that never refills says nothing of a reset', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([
          _bucket('credit', label: 'Credit', unit: 'micro_usd', used: 1200000, limit: 5000000, refill: {'kind': 'none'}),
        ]);
      await _started(tester, agent, _store());
      expect(_shown(tester), r'Credit · $1.20 / $5.00');
      await finish(tester);
    });

    testWidgets('shows the rate of a token bucket', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([
          _bucket(
            'tpm',
            label: 'Tokens',
            unit: 'tokens',
            used: 12000,
            limit: 40000,
            refill: {'kind': 'token_bucket', 'amount': 40000, 'per': [60, 0], 'max': 40000},
          ),
        ]);
      await _started(tester, agent, _store());
      expect(_shown(tester), 'Tokens · 12k / 40k tokens · 40k tokens/min');
      await finish(tester);
    });

    testWidgets('shows the bucket with the highest fraction, and an exhausted one before it', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([
          _bucket('five_hour', label: '5 hours', used: 300, refill: _window(600)),
          _bucket('seven_day', label: 'Week', used: 710, refill: _window(100000)),
        ]);
      await _started(tester, agent, _store());
      expect(_shown(tester), startsWith('Week · 71%'));
      agent.pushUsage(
        _report([
          _bucket('five_hour', label: '5 hours', used: 1000, status: 'exhausted', refill: _window(600)),
          _bucket('seven_day', label: 'Week', used: 710, refill: _window(100000)),
        ], revision: 2),
      );
      await settle(tester);
      expect(_shown(tester), startsWith('5 hours · 100%'));
      await finish(tester);
    });

    testWidgets('is the colour of an error once exhausted, a status near the limit, and plain below', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 500, refill: _window(600))]);
      await _started(tester, agent, _store());
      final scheme = Theme.of(tester.element(_pill)).colorScheme;
      expect(_colour(tester), isNull);
      agent.pushUsage(_report([_bucket('five_hour', label: '5 hours', used: 800, refill: _window(600))], revision: 2));
      await settle(tester);
      expect(_colour(tester), scheme.tertiary, reason: 'from [usage] warn_permille, 800 by default');
      agent.pushUsage(
        _report([_bucket('five_hour', label: '5 hours', used: 1000, status: 'exhausted', refill: _window(600))], revision: 3),
      );
      await settle(tester);
      expect(_colour(tester), scheme.error);
      await finish(tester);
    });

    testWidgets('shows a status from the configured fraction', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 500, refill: _window(600))]);
      await _started(tester, agent, _store(), config: const CanoaConfig(agents: [scripted], warnPermille: 400));
      expect(_colour(tester), Theme.of(tester.element(_pill)).colorScheme.tertiary);
      await finish(tester);
    });

    testWidgets('a window that has ended says it was reset and is not yet reported, with no colour of its own', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([_bucket('five_hour', label: '5 hours', used: 1000, status: 'exhausted', refill: _window(-30))]);
      await _started(tester, agent, _store());
      expect(_shown(tester), '5 hours · reset, not yet reported');
      expect(_colour(tester), isNull);
      await finish(tester);
    });

    testWidgets('keeps the newest report by revision, whatever order they arrive in', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 100)]);
      await _started(tester, agent, _store());
      agent.pushUsage(_report([_bucket('five_hour', label: '5 hours', used: 400)], revision: 5));
      await settle(tester);
      expect(_shown(tester), contains('40%'));
      agent.pushUsage(_report([_bucket('five_hour', label: '5 hours', used: 200)], revision: 4));
      await settle(tester);
      expect(_shown(tester), contains('40%'));
      await finish(tester);
    });

    testWidgets('is read again when a turn ends', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 100)]);
      await _started(tester, agent, _store());
      final asked = agent.methods.where((method) => method == '_coqui_acp/usage').length;
      await send(tester, 'Say again');
      await settle(tester, 150);
      expect(agent.methods.where((method) => method == '_coqui_acp/usage').length, greaterThan(asked));
      await finish(tester);
    });
  });

  group('the menu', () {
    Future<void> open(WidgetTester tester) async {
      await tester.tap(_pill);
      await frames(tester);
    }

    testWidgets('lists every bucket with its figure and refill, most pressing first', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([
          _bucket('five_hour', label: '5 hours', used: 300, refill: _window(630)),
          _bucket('seven_day', label: 'Week', used: 710, refill: _window(100030)),
          _bucket('credit', label: 'Credit', unit: 'micro_usd', used: 1200000, limit: 5000000, refill: {'kind': 'none'}),
        ], account: 'work');
      await _started(tester, agent, _store());
      await open(tester);
      final menu = find.byType(UsageMenu);
      expect(menu, findsOneWidget);
      const week = 'Week · 71% · resets in 1 d 3 h';
      const five = '5 hours · 30% · resets in 10 m';
      for (final line in [week, five, r'Credit · $1.20 / $5.00']) {
        expect(find.descendant(of: menu, matching: find.text(line)), findsOneWidget, reason: line);
      }
      Finder inMenu(String line) => find.descendant(of: menu, matching: find.text(line));
      expect(tester.getTopLeft(inMenu(week)).dy, lessThan(tester.getTopLeft(inMenu(five)).dy));
      expect(find.text('Account: work'), findsOneWidget);
      await finish(tester);
    });

    testWidgets('marks a bucket inferred, and says what it covers and how old it is', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report([
          _bucket(
            'inferred',
            label: 'Limit',
            limit: null,
            status: 'exhausted',
            source: 'inferred',
            evidence: 'RESOURCE_EXHAUSTED',
            models: {
              'kind': 'only',
              'models': ['opus'],
            },
            observedAt: _now - 240,
          ),
        ]);
      await _started(tester, agent, _store());
      expect(_shown(tester), 'Limit · limit reached');
      await open(tester);
      expect(find.text('inferred from "RESOURCE_EXHAUSTED", not reported by the agent'), findsOneWidget);
      expect(find.text('covers opus'), findsOneWidget);
      expect(find.text('as of 4 min ago'), findsOneWidget);
      await finish(tester);
    });

    testWidgets('shows the report the host infers for an agent that reports nothing', (tester) async {
      // Shaped as canoa makes it: source inferred throughout, no label, no figures, coverage and refill unknown.
      final agent = OptionsAgent()
        ..usage = {
          ..._report([
            {
              'id': 'inferred',
              'models': {'kind': 'unknown'},
              'unit': 'tokens',
              'refill': {'kind': 'unknown'},
              'status': 'exhausted',
              'observed_at': _now - 30,
              'source': 'inferred',
              'evidence': 'Error 429',
            },
          ], account: 'agy'),
          'sources': ['inferred'],
        };
      await _started(tester, agent, _store());
      expect(_shown(tester), contains('limit reached'));
      expect(_colour(tester), isNotNull);
      await open(tester);
      expect(find.text('inferred from "Error 429", not reported by the agent'), findsOneWidget);
      expect(find.text('Account: agy'), findsOneWidget);
      await finish(tester);
    });

    testWidgets('says "No usage yet" for an inferring host that has seen no limit', (tester) async {
      final agent = OptionsAgent()..usage = {..._report([]), 'sources': ['inferred']};
      await _started(tester, agent, _store());
      expect(_shown(tester), 'No usage yet');
      await finish(tester);
    });

    testWidgets('says a window past its end was reset, not yet reported', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 900, refill: _window(-5))]);
      await _started(tester, agent, _store());
      await open(tester);
      expect(find.descendant(of: find.byType(UsageMenu), matching: find.text('5 hours · reset, not yet reported')), findsOneWidget);
      await finish(tester);
    });

    testWidgets('says nothing is reported yet when it holds no bucket', (tester) async {
      final agent = OptionsAgent()..usage = _report([]);
      await _started(tester, agent, _store());
      await open(tester);
      expect(find.byKey(const Key('usage-empty')), findsOneWidget);
      await finish(tester);
    });

    testWidgets('keeps its text selectable', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 300)]);
      await _started(tester, agent, _store());
      await open(tester);
      expect(find.descendant(of: find.byType(UsageMenu), matching: find.byType(SelectionArea)), findsOneWidget);
      await finish(tester);
    });

    testWidgets('names the last refusal', (tester) async {
      final agent = OptionsAgent()
        ..usage = _report(
          [_bucket('five_hour', label: '5 hours', used: 1000, status: 'exhausted', refill: _window(5000))],
          limit: {
            'bucket': 'five_hour',
            'resets_at': _now + 5000,
            'retry_after': [90, 0],
            'source': 'reported',
            'text': 'You have hit your usage limit.',
            'observed_at': _now,
          },
        );
      await _started(tester, agent, _store());
      await open(tester);
      expect(find.textContaining('You have hit your usage limit. · retry after 1 m'), findsOneWidget);
      await finish(tester);
    });
  });

  group('the spend', () {
    Map<String, Object?> spend({int? cost}) => {
      'session_id': 'o1',
      'input': 1200,
      'output': 340,
      'cache_read': 9000,
      'cache_write': 800,
      'cost_micro_usd': ?cost,
      'by_model': <Object?>[],
    };

    testWidgets("joins the transcript's header with its tokens and cost", (tester) async {
      final agent = OptionsAgent()..usage = _report([], spend: spend(cost: 48500));
      await _started(tester, agent, _store());
      expect(find.text(r'1.5k tokens · $0.05'), findsOneWidget);
      await finish(tester);
    });

    testWidgets('leaves an absent cost absent', (tester) async {
      final agent = OptionsAgent()..usage = _report([], spend: spend());
      await _started(tester, agent, _store());
      expect(find.text('1.5k tokens'), findsOneWidget);
      await finish(tester);
    });

    testWidgets('follows a notification naming the session', (tester) async {
      final agent = OptionsAgent()..usage = _report([_bucket('five_hour', label: '5 hours', used: 100)]);
      await _started(tester, agent, _store());
      expect(find.textContaining('tokens'), findsNothing);
      agent.pushUsage(_report([_bucket('five_hour', label: '5 hours', used: 100)], revision: 2, spend: spend(cost: 2000000)), session: 'o1');
      await settle(tester);
      expect(find.text(r'1.5k tokens · $2.00'), findsOneWidget);
      await finish(tester);
    });
  });

  group('the text', () {
    test('counts are compact', () {
      expect([0, 950, 1000, 1500, 120000, 2400000, 3000000000].map(compactCount), ['0', '950', '1k', '1.5k', '120k', '2.4M', '3G']);
    });

    test('money rounds to cents', () {
      expect([0, 4999, 5000, 1200000, 5000000].map(dollars), [r'$0.00', r'$0.00', r'$0.01', r'$1.20', r'$5.00']);
    });

    test('a span is its two largest units', () {
      expect([30, 60, 600, 4800, 7200, 90000, 100000].map(spanText), ['30 s', '1 m', '10 m', '1 h 20 m', '2 h', '1 d 1 h', '1 d 3 h']);
    });

    test('a bucket gives its level against the threshold', () {
      final now = _now;
      UsageBucket at(int used, {String status = 'ok'}) => UsageBucket(id: 'b', used: used, limit: 1000, status: status);
      expect(levelOf(at(799), now), UsageLevel.calm);
      expect(levelOf(at(800), now), UsageLevel.warn);
      expect(levelOf(at(799), now, warnPermille: 500), UsageLevel.warn);
      expect(levelOf(at(100, status: 'warning'), now), UsageLevel.warn, reason: 'the agent said so');
      expect(levelOf(at(1000, status: 'exhausted'), now), UsageLevel.danger);
    });
  });
}
