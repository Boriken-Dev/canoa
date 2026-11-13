// The delegation card, the line that says what the turn waits on and a child's thread, drawn with the test font like the app's other
// goldens. With CANOA_SAMPLES_DIR set the same scenes are written with real fonts, for the UI reference.
import 'dart:io' show Directory;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegation_scene.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

Map<String, Object?> _said(String text) => {
  'jsonrpc': '2.0',
  'method': 'session/update',
  'params': {
    'sessionId': 's1',
    'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': text}},
  },
};

void main() {
  setUpAll(() {
    goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/delegation_scenes_test.dart'));
  });

  for (final (label, size) in [('desktop', const Size(1100, 900)), ('phone', const Size(390, 1100))]) {
    testWidgets('four children working, two of them done, and the line that waits on the rest, $label', (tester) async {
      final scene = await openScene(tester, size: size);
      await scene.turn();
      await scene.say(_said('I will split the share link into four parts and check them when they come back.'));
      await scene.fourChildren();
      await scene.end('c0', 'COMPLETED', at: 18);
      await scene.end('c1', 'COMPLETED', at: 21);
      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/delegation_working_$label.png'));
    });

    testWidgets('the turn done with one child failed and one stopped, $label', (tester) async {
      final scene = await openScene(tester, size: size);
      await scene.fourChildren();
      await scene.end('c0', 'COMPLETED', at: 41);
      await scene.end('c1', 'FAILED', at: 33, note: 'The source did not start: no login for it on this host.');
      await scene.end('c2', 'COMPLETED', at: 52);
      await scene.end('c3', 'CANCELED', at: 36);
      await scene.say(_said('Three of the four finished; the API part failed, so the link has no endpoint yet.'));
      scene.now = scene.now.add(const Duration(minutes: 2));
      await tester.tap(find.byKey(const Key('delegation-toggle')));
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/delegation_ended_$label.png'));
    });

    testWidgets('a child\'s thread with its pending approval and the way back, $label', (tester) async {
      final scene = await openScene(tester, size: size);
      await scene.fourChildren();
      await scene.say(childUpdate('c1', {
        'sessionUpdate': 'agent_message_chunk',
        'content': {'type': 'text', 'text': 'Reading the router before I add the endpoint.'},
      }));
      await scene.say(childUpdate('c1', {
        'sessionUpdate': 'tool_call', 'toolCallId': 'k1', 'title': 'Read src/router.ts', 'kind': 'read', 'status': 'completed',
      }));
      await scene.say(childUpdate('c1', {
        'sessionUpdate': 'tool_call', 'toolCallId': 'k2', 'title': 'Edit src/share.ts', 'kind': 'edit', 'status': 'in_progress',
      }));
      scene.host.ask(10, 'Edit src/share.ts', task: 'c1');
      await scene.settle();
      scene.store.openChild(scene.session, 'c1');
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/child_thread_$label.png'));
    });
  }
}
