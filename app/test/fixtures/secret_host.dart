// A host on a proven connection that asks the app for secrets, and the app around it: what the secret tests and scenes share.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UpdateRecorder;
import 'package:coqui_acp/secret_unprotected.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
final hostProver = ecdsaP256Prover(List<int>.generate(32, (i) => i + 101));
final appProver = ecdsaP256Prover(List<int>.generate(32, (i) => i + 1));

const password = 'hunter2-correct-horse';
const token = 'tok-9f8e7d6c5b4a';
const schema = '{"type":"object","properties":{"user":{"type":"string","title":"User name"},'
    '"password":{"type":"string","title":"Password","writeOnly":true}},"required":["user","password"]}';
const showSchema = '{"type":"object","properties":{"label":{"type":"string","title":"Label"},"key":{"type":"string","title":"API key","writeOnly":true}}}';

SecretIdentity identity() => (
  prover: appProver,
  verifier: AcpSecretVerifier(own: appProver.id),
  offer: const AcpSecretsCapability(bindings: [acpBindingEcdsa]),
);

/// The host: a proven peer on a channel that answers what the app asks, and sends its own requests to it.
final class SecretHost implements WebChannel {
  final input = StreamController<String>.broadcast(sync: true);
  final sent = <Map<String, dynamic>>[];
  @override
  WebChannelKind get kind => WebChannelKind.webSocket;
  @override
  Stream<String> get messages => input.stream;

  @override
  Future<void> send(String message) async {
    final request = Map<String, dynamic>.from(jsonDecode(message) as Map);
    sent.add(request);
    final id = request['id'];
    if (id == null || request['method'] == null) return;
    input.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': switch (request['method']) {
      'initialize' => {'protocolVersion': 2, 'capabilities': {'session': <String, Object?>{}}},
      'session/new' => {'sessionId': 's1'},
      _ => <String, Object?>{},
    }}));
  }

  @override
  Future<void> close() async => input.close();

  void ask(Object id, String method, Map<String, Object?> params) =>
      input.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params}));

  Map<String, dynamic>? answerTo(Object id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
  Map<String, dynamic>? get initialize => sent.where((message) => message['method'] == 'initialize').firstOrNull;

  /// A connection to this host that no conversation is attached to.
  Future<VerifiedAgent> open(SessionStore store) async {
    final client = await VerifiedAgent.open(
      this,
      expectedPeerId: hostProver.id.toString(),
      verifyPeer: (proven, expected) async => VerifiedPeer(expected, proven),
      decider: store.askPermission,
      elicitor: store.askQuestion,
      secrets: store,
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    return client;
  }

  Future<(ChatSession, VerifiedAgent)> connect(SessionStore store) async {
    final client = await open(store);
    final session = store.addProviderSession(title: 'Deploy', agent: 'Claude', peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    return (session, client);
  }
}

Future<void> settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 300)));
  await tester.pump();
}

Future<(SecretHost, ChatSession, SessionStore)> shown(WidgetTester tester, {bool withIdentity = true, Size size = const Size(1100, 1800), UpdateRecorder? recorder}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final store = SessionStore(recorder: recorder);
  if (withIdentity) store.secretIdentity = identity();
  final host = SecretHost();
  final (session, _) = await tester.runAsync(() => host.connect(store)) as (ChatSession, VerifiedAgent);
  await tester.pumpWidget(CanoaApp(store: store));
  await tester.pump();
  return (host, session, store);
}

/// What the host sends for a request: its own receiver, bound to its identity for the app.
Future<Map<String, Object?>> requestParams(String id) async {
  final context = AcpSecretContext(sessionId: 's1', requestId: id, message: 'The deploy needs your registry login', schema: schema);
  final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, hostProver, appProver.id, now);
  receivers[id] = receiver;
  return AcpSecretRequestParams(context: context, recipient: receiver.recipient).toJson();
}

final receivers = <String, AcpSecretReceiver>{};

/// Waits, in real time, until [finder] matches: a form is drawn once the card has checked the host's key.
Future<void> until(WidgetTester tester, Finder finder) async {
  for (var i = 0; i < 100 && finder.evaluate().isEmpty; i++) {
    await settle(tester);
  }
}
