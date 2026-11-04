// A conversation behind the one chosen finishes its turn, in the app's own window, and the desktop shows a notification for it.
// It is for a person to see: the test holds the window open for CANOA_TEST_HOLD seconds (default 20) while the notification is
// up, and passes when choosing the notification in that time opens the conversation, or when nobody chose it.
//
//   flutter test integration_test/notification_test.dart -d windows   (or -d linux, which needs a notification service)
//
// It sends nothing anywhere: the agent is in memory, and the notification names only "Behind" and its agent, "Notifier test".
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/attention/notifier.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

final int _hold = int.tryParse(Platform.environment['CANOA_TEST_HOLD'] ?? '') ?? 20;

Future<(StreamController<String>, ChatSession)> _session(SessionStore store, String title) async {
  final incoming = StreamController<String>();
  final client = VerifiedAgent.fromChild(
    label: 'Notifier test',
    messages: incoming.stream,
    close: () async {},
    send: (line) async {
      final message = jsonDecode(line) as Map<String, dynamic>;
      final id = message['id'];
      if (id == null || message['method'] == null) return;
      incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (message['method']) {
        'initialize' => {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}},
        'session/new' => {'sessionId': title},
        _ => <String, Object?>{},
      }}));
    },
  );
  await client.initialize();
  await client.openSession(cwd: Directory.systemTemp.path);
  final session = store.addProviderSession(title: title, agent: 'Notifier test', peerId: client.peerId, remoteSessionId: title);
  store.attachVerifiedClient(session.id, client, record: false);
  return (incoming, session);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('a finished turn behind is notified on this desktop, and choosing it opens the conversation', (tester) async {
    final store = SessionStore(includePreview: false);
    final (agent, behind) = await _session(store, 'Behind');
    final (_, front) = await _session(store, 'Front');
    store.select(front.id);
    final notifier = platformNotifier();
    await tester.pumpWidget(CanoaApp(store: store, notifier: notifier));
    await tester.pumpAndSettle();
    expect(await notifier.available, isTrue);

    void update(Map<String, Object?> body) =>
        agent.add(jsonEncode({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 'Behind', 'update': body}}));
    update({'sessionUpdate': 'state_update', 'state': 'running'});
    update({'sessionUpdate': 'state_update', 'state': 'idle', 'stopReason': 'end_turn'});
    debugPrint('NOTIFIED: look for "Behind" from Canoa for $_hold seconds');
    final until = DateTime.now().add(Duration(seconds: _hold));
    while (DateTime.now().isBefore(until) && store.selectedId != behind.id) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    debugPrint(store.selectedId == behind.id ? 'CHOSEN: the notification opened Behind' : 'NOT CHOSEN in $_hold seconds');
    expect(behind.unread || store.selectedId == behind.id, isTrue);
  }, skip: !Platform.isWindows && !Platform.isLinux, timeout: const Timeout(Duration(minutes: 5)));
}
