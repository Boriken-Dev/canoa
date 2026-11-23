// The Watch section of a shared session's details: what the host measured, and arming, changing and stopping a watch over a
// stand-in for the host's `sessions/activity` and `sessions/watch`.
import 'package:canoa/chat/git_changes.dart' show ChangeScope;
import 'package:canoa/chat/inspector.dart';
import 'package:canoa/chat/session_watch.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/task_agent.dart';

/// A host that answers `sessions/activity` with fixed figures and `sessions/watch` with the watch it was asked for.
final class _Host {
  final calls = <(String, Map<String, Object?>)>[];
  Map<String, Object?>? watch;
  String? refusal;

  SessionWatches get watches => SessionWatches(call);

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    if (method == 'sessions/activity') {
      return {
        'sessions': [
          {
            'session': params['session'],
            'quietMs': 12000,
            'processes': {'count': 3, 'cpuMs': 4500, 'memoryBytes': 64 * 1024 * 1024},
            'watch': watch,
          },
        ],
      };
    }
    if (refusal != null) throw ControlFailed(-32001, refusal!);
    if (params['enabled'] == false) return {'session': params['session'], 'watch': watch = null};
    final at = DateTime(2026, 11, 23, 10, 0, 0).millisecondsSinceEpoch;
    watch = {
      'armedBy': 'sha256:aa',
      'armedAt': at,
      'schedule': params.containsKey('cron') ? {'cron': params['cron'], 'utcOffsetMinutes': params['utcOffsetMinutes']} : {'everyMs': params['everyMs']},
      'prompt': params['prompt'],
      'maxPrompts': params['maxPrompts'] ?? 5,
      'promptsSent': 0,
      'nextCheckAt': at + 60000,
      'lastCheck': {'at': at - 1000, 'prompted': false, 'why': 'a turn is running'},
    };
    return {'session': params['session'], 'watch': watch};
  }

  List<Map<String, Object?>> get armed => [for (final (method, params) in calls) if (method == 'sessions/watch') params];
}

Future<void> _pumpCard(WidgetTester tester, _Host host) async {
  tester.view.physicalSize = const Size(700, 1200);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.light),
    home: Scaffold(
      body: SingleChildScrollView(
        child: SessionWatchCard(sessionId: 's1', watches: host.watches, utcOffset: const Duration(hours: -4)),
      ),
    ),
  ));
  await tester.pumpAndSettle();
}

Future<void> _tap(WidgetTester tester, Key key) async {
  await tester.ensureVisible(find.byKey(key));
  await tester.tap(find.byKey(key));
  await tester.pumpAndSettle();
}

Future<void> _type(WidgetTester tester, Key key, String text) async {
  await tester.ensureVisible(find.byKey(key));
  await tester.enterText(find.byKey(key), text);
  await tester.pump();
}

String _text(WidgetTester tester, Key key) => tester.widget<Text>(find.byKey(key)).data!;

void main() {
  testWidgets('the card shows what the host measured and arms a cron watch on the person\'s clock', (tester) async {
    final host = _Host();
    await _pumpCard(tester, host);
    expect(host.calls.single.$1, 'sessions/activity');
    expect(host.calls.single.$2, {'session': 's1'});
    expect(_text(tester, const Key('session-watch-measured')), 'Quiet for 12 s · 3 processes · CPU 4.5 s · 64.0 MB');
    expect(find.byKey(const Key('session-watch-state')), findsNothing);
    expect(find.byKey(const Key('session-watch-off')), findsNothing);

    await _tap(tester, const Key('session-watch-prompt-status'));
    await _tap(tester, const Key('session-watch-cron'));
    await _type(tester, const Key('session-watch-cron-line'), '*/10 * * * *');
    await _tap(tester, const Key('session-watch-children'));
    await _type(tester, const Key('session-watch-quiet'), '');
    await _type(tester, const Key('session-watch-cpu'), '200');
    await _type(tester, const Key('session-watch-max'), '0');
    await _tap(tester, const Key('session-watch-arm'));

    expect(host.armed.single, {
      'session': 's1',
      'cron': '*/10 * * * *',
      'utcOffsetMinutes': -240,
      'prompt': 'Status',
      'whenIdle': true,
      'whileChildren': true,
      'cpuBelowMs': 200,
      'memoryNotShrinking': false,
      'maxPrompts': 0,
      'stopOnInput': false,
    });
    expect(_text(tester, const Key('session-watch-state')), 'Watching: sends "Status" (0 of ∞ sent), next check at 10:01:00');
    expect(_text(tester, const Key('session-watch-last')), 'At 09:59:59 it sent nothing: a turn is running');
  });

  testWidgets('an interval is sent in milliseconds, a custom prompt as typed, and stopping takes the watch away', (tester) async {
    final host = _Host();
    await _pumpCard(tester, host);
    await _tap(tester, const Key('session-watch-prompt-custom'));
    await _type(tester, const Key('session-watch-custom'), 'Keep going if tests still fail');
    await _type(tester, const Key('session-watch-every'), '90');
    await _tap(tester, const Key('session-watch-stop-on-input'));
    await _tap(tester, const Key('session-watch-arm'));
    final armed = host.armed.single;
    expect(armed['everyMs'], 90000);
    expect(armed.containsKey('cron'), isFalse);
    expect(armed['quietMs'], 30000);
    expect(armed['maxPrompts'], 5);
    expect(armed['prompt'], 'Keep going if tests still fail');
    expect(armed['stopOnInput'], isTrue);
    expect(find.text('Change the watch'), findsOneWidget);

    await _tap(tester, const Key('session-watch-off'));
    expect(host.armed.last, {'session': 's1', 'enabled': false});
    expect(find.byKey(const Key('session-watch-state')), findsNothing);
    expect(find.text('Start watching'), findsOneWidget);
  });

  testWidgets('the host\'s refusal is shown in its own words', (tester) async {
    final host = _Host()..refusal = 'this key may not prompt the session';
    await _pumpCard(tester, host);
    await _tap(tester, const Key('session-watch-arm'));
    expect(_text(tester, const Key('session-watch-problem')), 'this key may not prompt the session');
    expect(find.byKey(const Key('session-watch-state')), findsNothing);
  });

  testWidgets('the details offer the Watch section only for a session the host keeps', (tester) async {
    final (_, session, store) = await showTask(tester, const Size(500, 1400));
    final host = _Host();
    Future<void> show() async {
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: SessionInspector(
            key: UniqueKey(),
            store: store,
            session: session,
            scopes: const [ChangeScope.reported],
            scope: ChangeScope.reported,
            onScope: (_) {},
            onOpenDiff: () {},
            onClose: () {},
            watches: host.watches,
          ),
        ),
      ));
      await tester.pumpAndSettle();
    }

    session.cursor = null;
    await show();
    expect(find.byKey(const Key('session-watch')), findsNothing);
    expect(host.calls, isEmpty);

    session.cursor = '7';
    await show();
    expect(find.byKey(const Key('session-watch')), findsOneWidget);
    expect(host.calls.single.$1, 'sessions/activity');
    expect(host.calls.single.$2, {'session': session.remoteSessionId});
  });
}
