// A conversation started by role against a fake host: the planner role with Claude first and Codex second, the app on a scripted
// agent that answers for either entry.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/roles/role_models.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'app_fixture.dart';
import 'fake_accounts.dart';
import 'fake_roles.dart';
import 'options_agent.dart';

const roleConfig = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['work']},
      'Codex': {'harness': 'codex'},
    },
  },
);

SessionStore roleStore() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

FakeAccountsHost roleAccounts() => FakeAccountsHost(accounts: [account('work', kind: 'claude-login')]);

final claudeMember = memberJson('Claude', account: 'work', model: 'opus', effort: 'high', serving: true);
final codexMember = memberJson('Codex', model: 'gpt-5');

/// The host with the planner role: Claude serves, Codex is next.
FakeRolesHost plannerHost({String handover = 'auto'}) => FakeRolesHost(roles: [
  roleOf('planner', [claudeMember, codexMember], title: 'Planner', handover: handover),
])
  ..resolutions['planner'] = RoleResolution.parse({'role': 'planner', 'member': claudeMember, 'moved': false, 'why': 'start', 'usage_known': true, 'same_harness': true, 'skipped': []})!;

/// Opens a new session and chooses [role] on its start screen.
Future<void> chooseRoleOnStartScreen(WidgetTester tester, String role) async {
  await tester.tap(find.byKey(const Key('role-control')));
  await frames(tester);
  await tester.tap(find.byKey(Key('role-option $role')));
  await settle(tester);
}

/// The app on [roles] with a conversation started by the planner role and one message sent, as far as the agent's first answer.
Future<ChatSession> startPlanner(WidgetTester tester, FakeRolesHost roles, OptionsAgent agent, SessionStore store, {Size size = const Size(1280, 800)}) async {
  await pumpApp(tester, store, linked: agent.openLink, config: roleConfig, accountsHost: roleAccounts(), rolesHost: roles, size: size);
  await newSession(tester);
  await settle(tester);
  await chooseRoleOnStartScreen(tester, 'planner');
  await openFolder(tester);
  await settle(tester);
  await send(tester, 'Plan it');
  await settle(tester, 150);
  return store.selected!;
}
