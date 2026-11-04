// What the person sees before a session starts with a profile: each part and what the harness takes of it, each MCP server's
// command or URL and the names it receives but never their values, and an overlay's server off until allowed.
import 'package:canoa/profile/apply.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_start_dialog.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const user = LayerId(LayerKind.user);
const team = LayerId(LayerKind.overlay, 'team');

final prepared = PreparedProfile(
  environment: const {'API_TOKEN': Secret('env-secret')},
  environmentLayers: const {'API_TOKEN': user},
  contexts: const [PreparedContext('Be brief.', user, 'text')],
  skills: const [PreparedSkill('review', team, files: {'SKILL.md': [35]})],
  mcp: const [
    PreparedMcp(
      name: 'marker', layer: user, shown: 'node marker.mjs', digest: 'sha256:1', isHttp: false,
      receives: ['MARKER_TOKEN'],
      server: AcpMcpStdio(name: 'marker', command: '/usr/bin/node', args: ['marker.mjs'], env: {'MARKER_TOKEN': 'stdio-secret'}),
    ),
    PreparedMcp(
      name: 'docs', layer: team, shown: 'HTTP https://docs.example/mcp', digest: 'sha256:2', isHttp: true,
      receives: ['Authorization'], withheld: ['Authorization: reads \$UNSHARED, which none of your layers shares'],
      server: AcpMcpHttp(name: 'docs', url: 'https://docs.example/mcp'),
    ),
  ],
);

Future<void> open(WidgetTester tester, {String? harness}) async {
  tester.view.physicalSize = const Size(1400, 2400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetDevicePixelRatio);
  addTearDown(tester.view.resetPhysicalSize);
  await tester.pumpWidget(MaterialApp(home: ProfileStartDialog(agent: 'Pi', prepared: prepared, harness: harness)));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('every server is shown with what it runs and the names it receives, never a value', (tester) async {
    await open(tester, harness: 'pi');
    expect(find.textContaining('node marker.mjs'), findsOneWidget);
    expect(find.textContaining('Receives variables: MARKER_TOKEN'), findsOneWidget);
    expect(find.textContaining('Receives headers: Authorization'), findsOneWidget);
    expect(find.textContaining('Left out: Authorization'), findsOneWidget);
    expect(find.textContaining('API_TOKEN'), findsOneWidget);
    for (final secret in ['env-secret', 'stdio-secret']) {
      expect(find.textContaining(secret), findsNothing);
    }
    // Pi takes none of the context or skills, and says why.
    expect(find.byKey(const Key('profile-state-context')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-context'))).data, 'not supported');
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-skills.review'))).data, 'not supported');
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-env.API_TOKEN'))).data, 'applied');
  });

  testWidgets("an overlay's server starts off, and the person's choice is what starts", (tester) async {
    await open(tester, harness: 'claude-code');
    CheckboxListTile tile(String name) => tester.widget(find.byKey(Key('profile-start-mcp-$name')));
    expect(tile('marker').value, isTrue);
    expect(tile('docs').value, isFalse);
    expect(find.textContaining('From overlay:team: off until you allow it'), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-skills.review'))).data, 'applied');
    await tester.tap(find.byKey(const Key('profile-start-mcp-docs')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('profile-start-mcp-marker')));
    await tester.pumpAndSettle();
    expect(tile('marker').value, isFalse);
    expect(tile('docs').value, isTrue);
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-skills.review'))).data, 'applied');
  });

  testWidgets('Start gives the servers turned on, and whether to keep that', (tester) async {
    ProfileStartChoice? chosen;
    tester.view.physicalSize = const Size(1400, 2400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
      onPressed: () async => chosen = await showDialog<ProfileStartChoice>(context: context,
          builder: (_) => ProfileStartDialog(agent: 'Claude', prepared: prepared, harness: 'claude-code',
              remembered: const {'docs': 'sha256:2', 'marker': false})),
      child: const Text('go'),
    ))));
    await tester.tap(find.text('go'));
    await tester.pumpAndSettle();
    expect(find.textContaining('allowed for Claude at this definition'), findsOneWidget);
    await tester.tap(find.byKey(const Key('profile-start-remember')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('profile-start')));
    await tester.pumpAndSettle();
    expect(chosen!.on, {'docs'});
    expect(chosen!.remember, isTrue);
  });

  testWidgets('the report shows each part, its layer, what became of it and why', (tester) async {
    await tester.pumpWidget(const MaterialApp(home: ProfileReportDialog(report: ProfileReport('pi', [
      ProfileItem('mcp.marker', 'user', ItemState.unsupported, 'pi-acp accepts mcpServers and starts none of them'),
      ProfileItem('env.API_TOKEN', 'user', ItemState.applied, "the agent's process environment"),
    ]))));
    expect(find.text('Profile for Pi'), findsOneWidget);
    expect(find.text('pi-acp accepts mcpServers and starts none of them'), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('profile-state-mcp.marker'))).data, 'not supported');
  });
}
