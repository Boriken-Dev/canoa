// A new session by role: the start screen offers the host's roles, says which member a session of the role would run on and why,
// starts that member's agent on its account and model, and binds the session to the role so the host moves it on. A refusal of the
// host (no member has usage left) is shown and stops the start.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/report.dart' show hostProfileMeta;
import 'package:canoa/roles/role_models.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/fake_roles.dart';
import 'fixtures/options_agent.dart';

const _config = CanoaConfig(
  agents: [scripted],
  host: {
    'agents': {
      'Claude': {'harness': 'claude', 'accounts': ['work']},
      'Codex': {'harness': 'codex'},
    },
  },
);

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

FakeAccountsHost _accounts() => FakeAccountsHost(accounts: [account('work', kind: 'claude-login')]);

final _claude = memberJson('Claude', account: 'work', model: 'opus', effort: 'high', serving: true);
final _codex = memberJson('Codex', model: 'gpt-5');
final _planned = memberJson('Claude', account: 'work', model: 'opus', profile: 'plan', serving: true);

FakeRolesHost _roles({String handover = 'auto'}) => FakeRolesHost(roles: [
  roleOf('planner', [_claude, _codex], title: 'Planner', handover: handover),
  roleOf('reviewer', [_codex], title: 'Reviewer'),
  roleOf('executor', [_planned], title: 'Executor'),
])
  ..resolutions['planner'] = RoleResolution.parse({'role': 'planner', 'member': _claude, 'moved': false, 'why': 'start', 'usage_known': true, 'same_harness': true, 'skipped': []})!
  ..resolutions['executor'] = RoleResolution.parse({'role': 'executor', 'member': _planned, 'moved': false, 'why': 'start', 'usage_known': true, 'same_harness': true, 'skipped': []})!
  ..resolutions['reviewer'] = RoleResolution.parse({
    'role': 'reviewer',
    'member': _codex,
    'moved': false,
    'why': 'start',
    'usage_known': false,
    'same_harness': true,
    'skipped': [
      {'member': 'Claude/work/opus/high/', 'why': 'its five-hour window is used up', 'until': 1, 'allowance': true, 'unavailable': false},
    ],
  })!;

Map<Object?, Object?> _meta(OptionsAgent agent) {
  final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
  return (initialize['params'] as Map)['_meta'] as Map<Object?, Object?>;
}

Future<void> _open(WidgetTester tester, FakeRolesHost roles, {OptionsAgent? agent, SessionStore? store}) async {
  await pumpApp(tester, store ?? _store(), linked: (agent ?? OptionsAgent()).openLink, config: _config, accountsHost: _accounts(), rolesHost: roles);
  await newSession(tester);
  await settle(tester);
}

Future<void> _choose(WidgetTester tester, String role) async {
  await tester.tap(find.byKey(const Key('role-control')));
  await frames(tester);
  await tester.tap(find.byKey(Key('role-option $role')));
  await settle(tester);
}

void main() {
  testWidgets('the start screen lists the roles, says which member would serve and why, and gives the draft its agent', (tester) async {
    final roles = _roles();
    final store = _store();
    await _open(tester, roles, store: store);
    expect(find.byKey(const Key('role-control')), findsOneWidget);
    expect(find.text('Role'), findsWidgets);
    await _choose(tester, 'planner');
    expect(roles.resolves.single.role, 'planner');
    expect(find.byKey(const Key('role-resolved')), findsOneWidget);
    expect(find.text('Claude · work · opus · high: first with usage'), findsOneWidget);
    final draft = store.selected!;
    expect(draft.role, 'planner');
    expect(draft.source?.label, 'Claude');
    expect(draft.account, 'work');
    expect((draft.roleServing?.member.model, draft.roleServing?.member.effort), ('opus', 'high'));
    await finish(tester);
  });

  testWidgets('a role whose member has no usage report says so, and the members it left out are in the tooltip', (tester) async {
    await _open(tester, _roles());
    await _choose(tester, 'reviewer');
    expect(find.text('Codex · gpt-5: first with usage (usage not reported)'), findsOneWidget);
    final tooltip = tester.widget<Tooltip>(find.ancestor(of: find.byKey(const Key('role-resolved')), matching: find.byType(Tooltip)).first);
    expect(tooltip.message, 'Claude/work/opus/high/: its five-hour window is used up');
    await finish(tester);
  });

  testWidgets('sending starts the member named by the host on its account and model, and binds the session to the role', (tester) async {
    final roles = _roles();
    final agent = OptionsAgent();
    final store = _store();
    await _open(tester, roles, agent: agent, store: store);
    await _choose(tester, 'planner');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Plan it');
    await settle(tester, 150);
    expect(agent.starts, ['Claude']);
    expect(_meta(agent)[accountMeta], 'work');
    expect(_meta(agent)[modelMeta], 'opus');
    final bound = roles.binds.single;
    expect((bound.role, bound.member, bound.entry, bound.session), ('planner', 'Claude/work/opus/high/', 'Claude', 'o1'));
    expect(bound.cwd, isNotEmpty);
    final session = store.selected!;
    expect(session.roleServing?.title, 'Planner');
    expect(session.roleServing?.member.model, 'opus');
    // Asked again when it starts, since the usage may have moved since the screen showed it.
    expect(roles.resolves.map((each) => each.role), ['planner', 'planner']);
    await finish(tester);
  });

  testWidgets('the host profile of the member goes to the host when the role starts the session, and none is named when there is none', (tester) async {
    final agent = OptionsAgent();
    await _open(tester, _roles(), agent: agent);
    await _choose(tester, 'executor');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Do it');
    await settle(tester, 150);
    expect(_meta(agent)[hostProfileMeta], 'plan');
    await finish(tester);
  });

  testWidgets('a member with no profile names none', (tester) async {
    final agent = OptionsAgent();
    await _open(tester, _roles(), agent: agent);
    await _choose(tester, 'planner');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Plan it');
    await settle(tester, 150);
    expect(_meta(agent).containsKey(hostProfileMeta), isFalse);
    await finish(tester);
  });

  testWidgets('a host with no member left refuses in its words, which the start screen shows and the start repeats', (tester) async {
    final roles = _roles()..refusal = 'Claude: used up until 15:04. Codex: not signed in.';
    final agent = OptionsAgent();
    await _open(tester, roles, agent: agent);
    await _choose(tester, 'planner');
    expect(find.byKey(const Key('role-refused')), findsOneWidget);
    expect(find.text('Claude: used up until 15:04. Codex: not signed in.'), findsOneWidget);
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Plan it');
    await settle(tester, 150);
    expect(agent.starts, isEmpty, reason: 'no agent starts when no member can serve');
    expect(roles.binds, isEmpty);
    await finish(tester);
  });

  testWidgets('No role takes the choice back', (tester) async {
    final roles = _roles();
    final store = _store();
    await _open(tester, roles, store: store);
    await _choose(tester, 'planner');
    expect(store.selected!.role, 'planner');
    await tester.tap(find.byKey(const Key('role-control')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('role-option-none')));
    await settle(tester);
    expect(store.selected!.role, isNull);
    expect(find.byKey(const Key('role-resolved')), findsNothing);
    await finish(tester);
  });

  testWidgets('an account picked by hand ends the role the draft was given', (tester) async {
    final store = _store();
    await _open(tester, _roles(), store: store);
    await _choose(tester, 'planner');
    expect(store.selected!.role, 'planner');
    store.setAccount(store.selected!, 'work');
    await settle(tester);
    expect(store.selected!.role, isNull);
    expect(store.selected!.roleServing, isNull);
    expect(find.byKey(const Key('role-resolved')), findsNothing);
    await finish(tester);
  });

  testWidgets('a running conversation shows its role in the picker, and Pick a member by hand ends the hold of the role on it', (tester) async {
    final roles = _roles();
    final store = _store();
    await _open(tester, roles, store: store);
    await _choose(tester, 'planner');
    await openFolder(tester);
    await settle(tester);
    await send(tester, 'Plan it');
    await settle(tester, 150);
    final session = store.selected!;
    await tester.tap(find.byKey(const Key('option-model')));
    await frames(tester);
    expect(find.byKey(const Key('role-serves')), findsOneWidget);
    expect(find.text('Planner is served by Claude · work · opus · high'), findsOneWidget);
    await tester.tap(find.byKey(const Key('role-by-hand')));
    await settle(tester);
    expect(roles.unbinds.single, (entry: 'Claude', session: 'o1'));
    expect(session.role, isNull);
    expect(session.roleServing, isNull);
    expect(find.byKey(const Key('role-serves')), findsNothing);
    await finish(tester);
  });

  testWidgets('a host with no role shows no control', (tester) async {
    await _open(tester, FakeRolesHost());
    expect(find.byKey(const Key('role-control')), findsNothing);
    await finish(tester);
  });
}
