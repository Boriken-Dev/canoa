// A reopened session's kept mode and its profile's mode: the report says which one the agent took.
import 'package:canoa/profile/apply.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/acp/client.dart' show AcpSessionSetup;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

const _layer = LayerId(LayerKind.user);

/// A profile that asks for [mode], as the app's launch path reports it before the session exists.
(ProfileLaunch, PreparedProfile) _profile(String mode) => (
  const ProfileLaunch(AcpSessionSetup.none, [ProfileItem('mode', 'user', ItemState.applied, 'at start')]),
  PreparedProfile(mode: Sourced(mode, _layer, null)),
);

ProfileItem _modeItem(ChatSession session) => session.profile!.items.firstWhere((item) => item.key == 'mode');

Future<ChatSession> _reopened(OptionsAgent agent, {required String kept, required String profile}) async {
  final ledger = SessionLedger(MemoryLedgerBackend());
  ledger.record(peerId: 'local:Scripted', sessionId: 'o1', entry: 'Scripted', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w');
  ledger.setMode('local:Scripted', 'o1', mode: kept, bypass: false);
  final store = SessionStore(includePreview: false, ledger: ledger);
  final client = await agent.start(scripted);
  final (launch, prepared) = _profile(profile);
  return store.reopen(ledger.entries.single, client, launch: launch, prepared: prepared, harness: 'claude');
}

void main() {
  testWidgets('the kept mode taken: the profile\'s mode is reported as replaced by it', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final session = await tester.runAsync(() => _reopened(agent, kept: 'auto', profile: 'plan')) as ChatSession;
    expect(agent.mode, 'auto');
    expect(_modeItem(session).state, ItemState.off);
  });

  testWidgets('the kept mode refused: the profile\'s mode is applied and reported as applied', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..refuseModes.add('auto');
    final session = await tester.runAsync(() => _reopened(agent, kept: 'auto', profile: 'plan')) as ChatSession;
    expect(agent.mode, 'plan', reason: 'neither mode was in effect and the report said the kept one was');
    expect(_modeItem(session).state, ItemState.applied);
  });

  testWidgets('the kept mode not offered any more: the profile\'s mode is applied and reported as applied', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.codexModes);
    final session = await tester.runAsync(() => _reopened(agent, kept: 'bypass', profile: 'plan')) as ChatSession;
    expect(agent.mode, 'plan');
    expect(_modeItem(session).state, ItemState.applied);
  });
}
