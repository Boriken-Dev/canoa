import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

/// An empty workspace for the Files surface.
final class FakeFiles implements WorkspaceFiles {
  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async =>
      HostFolderListing(path: path, parent: null, entries: const [], more: false);

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async =>
      HostFileRead(path: path, name: path.split('/').last, size: 0, kind: 'text', truncated: false);
}

/// An agent on a scripted pipe: it answers the handshake and every prompt, and [update] plays an update to the session.
final class FakeAgent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late final VerifiedAgent client;

  Future<ChatSession> start(SessionStore store, {required String cwd}) async {
    client = VerifiedAgent.fromChild(
      label: 'Fake',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        if (message['method'] == 'initialize') {
          reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (message['method'] == 'session/new') {
          reply(message['id'], {'sessionId': 's1'});
        } else if (message['method'] == 'session/prompt') {
          reply(message['id'], {'stopReason': 'end_turn'});
        } else if (message['id'] != null) {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: cwd);
    final session = store.addProviderSession(
      title: 'Fix addNumbers bug',
      agent: 'Claude Code',
      peerId: client.peerId,
      remoteSessionId: opened.id,
      cwd: cwd,
    );
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> body) => incoming.add(jsonEncode({
        'jsonrpc': '2.0',
        'method': 'session/update',
        'params': {'sessionId': 's1', 'update': body},
      }));
}

Future<void> settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 25)));
  await tester.pump();
}

