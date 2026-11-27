// A conversation a role serves goes on in place when the host moves it: the same tab, the new session id on the next member's
// entry, the transcript kept, and the line says when and from whom. A conversation that is not the one moved is left alone, and a
// move the app cannot follow leaves it where it was and says why.
import 'package:canoa/roles/role_models.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_roles.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/role_scene.dart';

RoleMoved _moved({String toEntry = 'Codex', String toSession = 'o7', String fromSession = 'o1', List<Object?> losses = const []}) => RoleMoved.parse({
  'role': 'planner',
  'from': {'entry': 'Claude', 'session': fromSession},
  'to': {'entry': toEntry, 'session': toSession},
  'replayed': false,
  'losses': losses,
})!;

/// Claude has run out and Codex serves now, as the host lists it after the move.
void _afterMove(FakeRolesHost roles) {
  roles.roles[0] = roleOf('planner', [
    memberJson('Claude', account: 'work', model: 'opus', effort: 'high', whyNot: {'member': 'Claude/work/opus/high/', 'why': 'five-hour window used up', 'until': 1796126640, 'allowance': true, 'unavailable': false}),
    memberJson('Codex', model: 'gpt-5', serving: true),
  ], title: 'Planner');
}

/// What the person said in [session], as its transcript holds it.
List<String> _said(ChatSession session) => [for (final each in session.messages) if (each.role == MessageRole.user) each.text];

void main() {
  testWidgets('the conversation goes on in the same tab on the new session, with its transcript, and the line says what moved', (tester) async {
    final roles = plannerHost();
    final agent = OptionsAgent();
    final store = roleStore();
    final session = await startPlanner(tester, roles, agent, store);
    expect(session.remoteSessionId, 'o1');
    expect(_said(session), contains('Plan it'));
    final tabs = store.sessions.where((each) => !each.isDraft).length;
    _afterMove(roles);

    roles.move(_moved(losses: [
      {'kind': 'thinking', 'count': 3},
      'tool_as_note',
    ]));
    await until(tester, () => session.remoteSessionId == 'o7', 'the conversation to follow the host');
    await settle(tester, 100);

    expect(store.selected, same(session), reason: 'the same conversation, not a new one');
    expect(store.sessions.where((each) => !each.isDraft).length, tabs, reason: 'no new tab');
    expect(agent.starts, ['Claude', 'Codex'], reason: 'the next member\'s agent was started');
    expect(session.agent, 'Codex');
    expect(session.source?.label, 'Codex');
    expect(session.account, isNull, reason: 'Codex\'s member names no account');
    expect(_said(session), contains('Plan it'), reason: 'the transcript is kept');
    expect(find.text('Plan it'), findsWidgets);
    expect(session.roleServing?.member.agent, 'Codex');
    expect(session.roleServing?.movedFrom?.agent, 'Claude');
    expect(find.textContaining('Planner moved from Claude · work · opus · high to Codex · gpt-5 at '), findsOneWidget);
    expect(find.textContaining('· 2 did not carry'), findsOneWidget);
    expect(session.roleProblem, isNull);
    await finish(tester);
  });

  testWidgets('a move of some other session is not followed', (tester) async {
    final roles = plannerHost();
    final agent = OptionsAgent();
    final store = roleStore();
    final session = await startPlanner(tester, roles, agent, store);
    _afterMove(roles);
    roles.move(_moved(fromSession: 'elsewhere'));
    await settle(tester, 150);
    expect(session.remoteSessionId, 'o1');
    expect(agent.starts, ['Claude']);
    expect(session.roleServing?.movedAt, isNull);
    await finish(tester);
  });

  testWidgets('a move to an entry this computer does not run leaves the conversation where it was and says why', (tester) async {
    final roles = plannerHost();
    final agent = OptionsAgent();
    final store = roleStore();
    final session = await startPlanner(tester, roles, agent, store);
    _afterMove(roles);
    roles.move(_moved(toEntry: 'Elsewhere'));
    await settle(tester, 200);
    expect(session.remoteSessionId, 'o1');
    expect(session.agent, 'Claude');
    expect(session.roleProblem?.key, 'role.problem.follow');
    expect(find.byKey(const Key('role-problem')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a new agent that will not start leaves it with the agent it had, and says why', (tester) async {
    final roles = plannerHost();
    final agent = OptionsAgent();
    final store = roleStore();
    final session = await startPlanner(tester, roles, agent, store);
    _afterMove(roles);
    agent.failNextStarts = 1;
    roles.move(_moved());
    await until(tester, () => session.roleProblem != null, 'the failure to be said');
    await settle(tester, 100);
    expect(session.remoteSessionId, 'o1');
    expect(session.agent, 'Claude');
    expect(_said(session), contains('Plan it'));
    expect(session.roleServing?.member.agent, 'Claude', reason: 'the line still names the member it runs on');
    expect(find.byKey(const Key('role-problem')), findsOneWidget);
    await finish(tester);
  });
}
