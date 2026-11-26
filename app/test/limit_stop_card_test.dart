// A turn the host refuses for a limit ends with the stop reason `refusal`: the conversation asks the host how the limit named in the
// host's words stands, and shows the card of the limit that is stopping it, with Continue once and Open limits. An agent's own
// refusal, or a limit that is not stopping, shows none. Over an agent that refuses and a scripted answer to `limits/state`.
@Timeout(Duration(minutes: 2))
library;

import 'package:canoa/usage/limit_rules.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

final class _Host implements LimitsLink {
  _Host(this.rows);

  List<Map<String, Object?>> rows;
  final allowed = <(String, String?)>[];
  String? refuse;
  var asked = 0;

  @override
  Future<List<LimitRow>> state() async {
    asked++;
    return [for (final row in rows) LimitRow.fromJson(row)];
  }

  @override
  Future<String?> allow(String name, {String? session}) async {
    allowed.add((name, session));
    return refuse;
  }
}

Map<String, Object?> _row(String name, String state, int permille) => {
  'name': name,
  'scope': 'all',
  'figure': {'kind': 'tokens', 'used': permille, 'limit': 1000, 'permille': permille},
  'state': state,
  'warn_permille': 800,
  'stop_permille': 1000,
  'window': {'start': 1795392000, 'resets_at': 1795996800},
};

const _notice = 'Limit week_work stopped this turn: 104 percent used (26.00 of 25.00 USD) this week (UTC); it resets in 4 days.';

void main() {
  Future<(SessionStore, OptionsAgent, _Host)> open(WidgetTester tester, {String said = _notice, List<Map<String, Object?>>? rows}) async {
    final store = SessionStore();
    final agent = OptionsAgent()..refusal = said;
    final host = _Host(rows ?? [_row('week_work', 'stop', 1040), _row('other', 'ok', 100)]);
    await pumpApp(tester, store, linked: agent.openLink);
    store
      ..limits = host
      ..onOpenLimits = null;
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    return (store, agent, host);
  }

  testWidgets('a refused turn shows the card of the limit the host names, and Continue once lets one turn through and sends it again', (tester) async {
    final (store, agent, host) = await open(tester);
    var opened = 0;
    store.onOpenLimits = () => opened++;
    await send(tester, 'Write the report');
    expect(host.asked, 1);
    expect(find.byKey(const Key('limit-stop-card week_work')), findsOneWidget);
    expect(find.text('The limit week_work stopped this turn'), findsOneWidget);
    expect(find.byKey(const Key('limit-stop-figure')), findsOneWidget);

    await tester.tap(find.byKey(const Key('limit-stop-open')));
    expect(opened, 1);

    // The host lets one turn through, and the message that was refused goes again; the card is gone once a turn is sent.
    agent.refusal = null;
    await tester.tap(find.byKey(const Key('limit-stop-continue')));
    await settle(tester);
    expect(host.allowed, [('week_work', store.selected!.remoteSessionId)]);
    expect(find.byKey(const Key('limit-stop-card week_work')), findsNothing);
    final prompts = [for (final request in agent.requests) if (request['method'] == 'session/prompt') request];
    expect(prompts, hasLength(2));
    expect('${prompts.last['params']}', contains('Write the report'));
    await finish(tester);
  });

  testWidgets('the card goes when the person sends the next message', (tester) async {
    final (store, agent, _) = await open(tester);
    await send(tester, 'Write the report');
    expect(find.byKey(const Key('limit-stop-card week_work')), findsOneWidget);
    agent.refusal = null;
    await send(tester, 'Never mind');
    expect(store.limitStop(store.selected!), isNull);
    expect(find.byKey(const Key('limit-stop-card week_work')), findsNothing);
    await finish(tester);
  });

  testWidgets('an agent\'s own refusal shows no card', (tester) async {
    final (_, _, host) = await open(tester, said: 'I will not do that.');
    await send(tester, 'Do something');
    expect(host.asked, 0, reason: 'the host\'s words name no limit, so it is not asked');
    expect(find.byKey(const Key('limit-stop-card week_work')), findsNothing);
    await finish(tester);
  });

  testWidgets('a limit that is not stopping shows no card', (tester) async {
    final (_, _, calm) = await open(tester, rows: [_row('week_work', 'warn', 900)]);
    await send(tester, 'Write the report');
    expect(calm.asked, 1);
    expect(find.byType(Card), findsNothing);
    await finish(tester);
  });

  testWidgets('a host that refuses to let a turn through says so and keeps the card', (tester) async {
    final (store, agent, host) = await open(tester);
    host.refuse = 'only the person running this host lets a turn through a limit';
    await send(tester, 'Write the report');
    agent.refusal = null;
    await tester.tap(find.byKey(const Key('limit-stop-continue')));
    await settle(tester);
    expect(store.limitStop(store.selected!), isNotNull);
    await finish(tester);
  });
}
