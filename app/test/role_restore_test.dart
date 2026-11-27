// A conversation opened again from the list says which role served it: the app asks the host's record of the session and puts the
// "served by" line back. A session no role served keeps none, and the host being unable to say leaves it as it is.
import 'package:canoa/roles/role_models.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/role_scene.dart';

RoleServedInfo _served(Object? role) => RoleServedInfo.parse({
  'turns': [
    {'no': 1, 'role': role, 'member': 'Codex///gpt-5//', 'agent': 'Codex', 'account': null, 'model': 'gpt-5', 'effort': null, 'profile': null},
  ],
  'moves': [],
});

void main() {
  test('the last turn a role served names the role and the member', () {
    final info = RoleServedInfo.parse({
      'turns': [
        {'no': 1, 'role': 'planner', 'agent': 'Claude', 'account': 'work', 'model': 'opus', 'member': 'Claude/work/opus//'},
        {'no': 2, 'role': 'planner', 'agent': 'Codex', 'model': 'gpt-5', 'member': 'Codex///gpt-5//'},
        {'no': 3, 'role': null, 'agent': 'Codex'},
      ],
    });
    expect((info.role, info.member?.agent, info.member?.model), ('planner', 'Codex', 'gpt-5'));
    expect(RoleServedInfo.parse({'turns': []}).role, isNull);
  });

  testWidgets('the served line comes back for a session the host says a role served', (tester) async {
    final roles = plannerHost();
    final store = roleStore();
    final session = await startPlanner(tester, roles, OptionsAgent(), store);
    session
      ..roleServing = null
      ..roleRestored = false;
    store.dismissRoleAsk(session);
    await tester.pump();
    expect(find.byKey(const Key('role-served-by')), findsNothing);
    roles.served_['Claude/o1'] = _served('planner');
    store.restoreRole(session);
    await until(tester, () => session.roleServing != null, 'the line to come back');
    expect(session.roleServing?.title, 'Planner');
    expect(find.text('Served by Codex · gpt-5 · Planner'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a session no role served, and a host with no record, keep no line', (tester) async {
    final roles = plannerHost();
    final store = roleStore();
    final session = await startPlanner(tester, roles, OptionsAgent(), store);
    for (final record in [_served(null), null]) {
      session
        ..roleServing = null
        ..roleRestored = false;
      if (record != null) roles.served_['Claude/o1'] = record;
      store.restoreRole(session);
      await settle(tester, 100);
      expect(session.roleServing, isNull);
    }
    await finish(tester);
  });
}
