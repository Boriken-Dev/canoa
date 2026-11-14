// The scripted secret flow over a Noise-paired connection to a real host: the app proves the host's static key by Noise (the Rust
// peer stands in for the page's non-extractable WebCrypto key), binds its answers by the Diffie-Hellman of the two static keys,
// and the agent's tools work end to end. A connection no Noise key proved is given no tools.
//
//   cargo build -p canoa-host --bin canoa --examples
//   CANOA_TEST_HOST=../target/debug/canoa flutter test test/noise_secret_host_test.dart
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/secret.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';
import 'fixtures/secret_host.dart' show identity, schema;
import 'fixtures/web_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

const _password = 'Tr0ub4dor.3/x+y=?qZ_9';

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

void main() {
  final skip = _hostBinary == null || exampleBinary('web_peer') == null || exampleBinary('stdio_agent') == null
      ? 'CANOA_TEST_HOST names a built canoa, with the examples built'
      : null;
  late Directory dir;
  late Process host;
  late String url;
  late String hostId;
  late WebPeerBridge browser;

  Future<ProcessResult> command(List<String> args) => Process.run(
    _hostBinary!,
    args,
    environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
  );

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-noise-secret');
    final config = File('${dir.path}/canoa.cfg');
    await config.writeAsString(
      '[host.agents.Scripted]\nprogram = ${jsonEncode(exampleBinary('stdio_agent'))}\n'
      'args = ["--scripted", "--mcp-http", "--modes=default,bypassPermissions"]\nlisten = "127.0.0.1:0"\n',
    );
    host = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path, '--method', 'noise'],
      environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home', 'XDG_DOCUMENTS_DIR': '${dir.path}/documents'},
    );
    host.stderr.transform(utf8.decoder).listen(stderr.write);
    final line = await host.stdout.transform(utf8.decoder).transform(const LineSplitter()).first.timeout(const Duration(seconds: 30));
    final entry = jsonDecode(line) as Map<String, dynamic>;
    expect(entry['method'], 'noise');
    url = entry['url'] as String;
    hostId = entry['peer'] as String;
    browser = await WebPeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    host.kill();
    await host.exitCode;
    await dir.delete(recursive: true);
  });

  Future<VerifiedAgent> connect(SessionStore store, {bool ecdsaOffer = false}) => connectVerifiedAcp(
    chatEndpoints(webSocket: url),
    expectedPeerId: hostId,
    verifyPeer: (channel, expected) async {
      final peer = await provePeer(browser, channel, expected, method: PeerMethod.noise);
      // The same proof, handed on with no Noise identity: the app then offers what the store has, a WebCrypto binding.
      return ecdsaOffer ? VerifiedPeer(peer.peerId, peer.channel) : peer;
    },
    decider: store.askPermission,
    elicitor: store.askQuestion,
    secrets: store,
  );

  test('the paired key is turned away until the person accepts it on the host', () async {
    await expectLater(connect(SessionStore()), throwsA(anything));
    final accepted = await command(['accept', await browser.localId(PeerMethod.noise)]);
    expect(accepted.exitCode, 0, reason: accepted.stderr as String);
  }, skip: skip);

  test('the scripted secret flow runs over Noise: sealed under the static keys, used once in an env, only the handle in the transcript', () async {
    final store = SessionStore();
    final client = await connect(store);
    final opened = await client.openSession(cwd: dir.path);
    expect(client.noiseSecrets, isNotNull, reason: 'the proof gave the host\'s static key, so the app binds by it');
    final session = store.addProviderSession(title: 'Deploy', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client, record: false);
    final said = StringBuffer();
    final listening = client.updates.listen((update) => said.write(update.text ?? ''));

    final asking = client.prompt(opened.id, 'Secret secret_request ${jsonEncode({'message': 'Registry login', 'schema': schema})}');
    await _until(() => session.pendingSecrets.isNotEmpty, 'the host to ask the app');
    final prompt = session.pendingSecrets.single as SecretRequestPrompt;
    final environment = store.secretEnvironment(prompt);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final collector = await environment.verifier.collector(prompt.host, AcpSecretMethod.request, prompt.request.context, prompt.request.recipient, now);
    final sealed = {'password': await collector.seal('password', _password)};
    const plain = '{"user":"ana"}';
    final auth = await collector.authenticate(environment.prover, plain, sealed, now);
    expect(auth.method, acpBindingDh);
    store.answerSecret(session, prompt, AcpSecretAccepted(plain, sealed, auth));
    await asking;
    final answer = jsonDecode(said.toString()) as Map<String, Object?>;
    expect(answer['status'], 'accepted', reason: said.toString());
    final handle = (answer['handles']! as Map)['password']! as String;
    expect(handle, startsWith('secret_'));
    expect(said.toString(), isNot(contains(_password)));

    said.clear();
    final echo = Platform.isWindows ? 'echo %REGISTRY_TOKEN%' : 'echo "\$REGISTRY_TOKEN"';
    final using = client.prompt(opened.id, 'Secret secret_use ${jsonEncode({'handle': handle, 'env': {'name': 'REGISTRY_TOKEN', 'command': echo}})}');
    await _until(() => session.pendingPermissions.isNotEmpty, 'the host to put the use to the person');
    store.answerPermission(session, allow: true);
    await using;
    final ran = jsonDecode(said.toString()) as Map<String, Object?>;
    expect(ran['status'], 'exited', reason: said.toString());
    expect(ran['stdout']! as String, contains(handle), reason: 'what the command printed came back as the handle');
    expect(said.toString(), isNot(contains(_password)));
    expect(session.messages.map((message) => message.text).join('\n'), isNot(contains(_password)));
    await listening.cancel();
    await client.close();
  }, skip: skip);

  test('a host paired by Noise refuses the ECDSA binding an app offers on its link, and gives the agent no secret tools', () async {
    final store = SessionStore()..secretIdentity = identity();
    final client = await connect(store, ecdsaOffer: true);
    expect(client.noiseSecrets, isNull);
    final opened = await client.openSession(cwd: dir.path);
    final said = StringBuffer();
    final listening = client.updates.listen((update) => said.write(update.text ?? ''));
    await client.prompt(opened.id, 'Secret secret_request ${jsonEncode({'message': 'x', 'schema': schema})}');
    await Future<void>.delayed(const Duration(milliseconds: 100));
    await listening.cancel();
    expect(said.toString(), contains('no secret tools'));
    await client.close();
  }, skip: skip);
}
