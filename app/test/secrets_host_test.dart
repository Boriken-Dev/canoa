// The Secrets section's code against the real host, as a library in this process: an agent asks for a secret through the host's own tools,
// the person's answer says how long to keep it, and the app's own code then lists it, shows it on request, sets another by hand through
// the host's sealed form, and forgets them. CANOA_TEST_LIBRARY names a built canoa_ffi library; CANOA_TEST_HOST a built canoa, which
// stands for what runs the host's own commands. The scripted agent is canoa-host's `stdio_agent` example. Without CANOA_TEST_LIBRARY
// these tests are skipped.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/accounts/accounts_host.dart' show SignInAsk;
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/secrets/secrets_controller.dart';
import 'package:canoa/secrets/secrets_host.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/forms/schema_fields.dart' show SecretSource;
import 'package:canoa_native/canoa_native.dart';
import 'package:coqui_acp/secret.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';

final String? _library = switch (Platform.environment['CANOA_TEST_LIBRARY']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};
final String? _host = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

/// What the person's app answers a form the host sealed to it: [plain] as JSON text and the secret [values] by field.
Future<AcpSecretAccepted> _answer(SignInAsk asking, String plain, Map<String, String> values) async {
  final environment = asking.environment;
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  final collector = await environment.verifier.collector(environment.expected, AcpSecretMethod.request, asking.request.context, asking.request.recipient, now);
  final sealed = {for (final entry in values.entries) entry.key: await collector.seal(entry.key, entry.value)};
  final auth = await collector.authenticate(environment.prover, plain, sealed, now);
  return AcpSecretAccepted(plain, sealed, auth);
}

void main() {
  final skip = _library == null ? 'CANOA_TEST_LIBRARY names a built canoa_ffi library' : null;

  group('in this process', skip: skip, () {
    late Directory dir;
    late String cfg;
    late CanoaNative native;
    late LibraryHostTool tool;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-secrets');
      cfg = '${dir.path}${Platform.pathSeparator}canoa.cfg';
      native = CanoaNative.open(path: _library);
      tool = LibraryHostTool(native, home: '${dir.path}/home', program: _host);
    });
    tearDown(() async {
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may still hold a file a moment after a stop.
      }
    });

    test('a secret an agent asked for and the person kept is listed, shown on request and forgotten; a second request is answered unasked; one is set by hand', () async {
      final agent = exampleBinary('stdio_agent');
      if (agent == null) fail(exampleMissing('stdio_agent')!);
      await File(cfg).writeAsString('${const CanoaConfig().encode()}\n[host.agents.Scripted]\nprogram = ${jsonEncode(agent)}\nargs = ["--scripted", "--mcp-http", "--modes=default,bypassPermissions"]\nlisten = "127.0.0.1:0"\n');
      final supervisor = HostSupervisor(tool);
      await supervisor.apply(configPath: cfg);
      await _until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final link = supervisor.openLink('Scripted');
      final secret = (link as HostLinkSecret).takeLinkSecret();
      expect(secret, isNotNull);
      final store = SessionStore();
      final client = VerifiedAgent.fromChild(
        label: 'Scripted', messages: link.messages, send: link.send, close: link.close,
        decider: store.askPermission, elicitor: store.askQuestion, secrets: store, linkSecrets: await linkSecretIdentity(secret!),
      );
      await client.initialize();
      final opened = await client.openSession(cwd: dir.path);
      final session = store.addProviderSession(title: 'Deploy', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
      store.attachVerifiedClient(session.id, client, record: false);
      final said = StringBuffer();
      final listening = client.updates.listen((update) => said.write(update.text ?? ''));
      const typed = 'Tr0ub4dor.3/x+y=?qZ_9';
      const schema = '{"type":"object","properties":{"deploy_token":{"type":"string","writeOnly":true}},"required":["deploy_token"]}';

      // The host asks, and its form carries the question of how long to keep it: the person chooses until Canoa closes.
      final asking = client.prompt(opened.id, 'Secret secret_request ${jsonEncode({'message': 'The deploy needs a token', 'schema': schema})}');
      await _until(() => session.pendingSecrets.isNotEmpty, 'the host to ask the app');
      final prompt = session.pendingSecrets.single as SecretRequestPrompt;
      expect(prompt.request.context.schema, contains('_keep'), reason: 'the form offers how long to keep it');
      expect(prompt.request.context.schema, contains('Until Canoa closes'));
      final environment = store.secretEnvironment(prompt);
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final collector = await environment.verifier.collector(prompt.host, AcpSecretMethod.request, prompt.request.context, prompt.request.recipient, now);
      final sealed = {'deploy_token': await collector.seal('deploy_token', typed)};
      const plain = '{"_keep":"close"}';
      store.answerSecret(session, prompt, AcpSecretAccepted(plain, sealed, await collector.authenticate(environment.prover, plain, sealed, now)));
      await asking;
      final first = jsonDecode(said.toString()) as Map<String, Object?>;
      expect(first['status'], 'accepted', reason: said.toString());
      expect(said.toString(), isNot(contains(typed)));

      // The app's own code: the table lists it, with no value.
      final control = HostControl(supervisor);
      final secrets = SecretsController(host: ControlSecretsHost(control));
      await secrets.refresh();
      expect(secrets.problem, isNull);
      final row = secrets.rows.singleWhere((each) => each.name == 'deploy_token');
      expect((row.provider, row.kept, row.expiresIn), ('form', 'process', null));
      expect(secrets.shown(row), isNull);
      expect(secrets.keeps.map((keep) => keep.id), contains('close'));
      expect(secrets.settable.map((entry) => entry.name), containsAll(['form', 'system']));

      // The value is asked for when the person shows it, and only then.
      await secrets.toggle(row);
      expect(secrets.shown(row), typed);
      await secrets.toggle(row);
      expect(secrets.shown(row), isNull);

      // Kept until Canoa closes, a second request for the name is answered without asking the person.
      said.clear();
      await client.prompt(opened.id, 'Secret secret_request ${jsonEncode({'message': 'Again', 'schema': schema})}');
      expect(session.pendingSecrets, isEmpty, reason: 'nobody was asked');
      final second = jsonDecode(said.toString()) as Map<String, Object?>;
      expect(second['status'], 'accepted', reason: said.toString());
      expect((second['handles']! as Map)['deploy_token'], isNot((first['handles']! as Map)['deploy_token']));
      expect(said.toString(), isNot(contains(typed)));

      // One is set by hand through the host's sealed form: the value never crosses the link in the clear.
      const byHand = 'by-hand-zq9-not-real-4c1d';
      final run = secrets.set(provider: 'form', name: 'by_hand');
      await _until(() => run.asking != null || !run.running, 'the host to ask for the value');
      expect(run.problem, isNull);
      expect(run.asking!.request.context.schema, contains('Keep it'));
      run.answer(await _answer(run.asking!, '{"keep":"close"}', {'value': byHand}));
      await _until(() => run.done || run.problem != null, 'the host to say it is set');
      expect(run.problem, isNull);
      await _until(() => secrets.rows.any((each) => each.name == 'by_hand'), 'the table to list it');
      final hand = secrets.rows.singleWhere((each) => each.name == 'by_hand');
      await secrets.toggle(hand);
      expect(secrets.shown(hand), byHand);

      // Forgetting takes it from the table, and the host no longer answers for it.
      await secrets.forget(row);
      await secrets.forget(hand);
      expect(secrets.rows, isEmpty);
      await expectLater(ControlSecretsHost(control).reveal('form', 'deploy_token'), throwsA(isA<ControlFailed>()));
      said.clear();
      final again = client.prompt(opened.id, 'Secret secret_request ${jsonEncode({'message': 'After forgetting', 'schema': schema})}');
      await _until(() => session.pendingSecrets.isNotEmpty, 'the host to ask the app again');
      store.answerSecret(session, session.pendingSecrets.single as SecretRequestPrompt, const AcpSecretCancelled());
      await again;
      expect(jsonDecode(said.toString()), containsPair('status', 'cancelled'));

      await listening.cancel();
      await client.close();
      await supervisor.stop();
    });

    test('a provider is set up, listed and taken away through the real host, with its key taken from the form and no secret written to canoa.cfg', () async {
      final agent = exampleBinary('stdio_agent');
      if (agent == null) fail(exampleMissing('stdio_agent')!);
      await File(cfg).writeAsString('${const CanoaConfig().encode()}\n[host.agents.Scripted]\nprogram = ${jsonEncode(agent)}\nargs = ["--scripted"]\nlisten = "127.0.0.1:0"\n');
      final supervisor = HostSupervisor(tool);
      await supervisor.apply(configPath: cfg);
      await _until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final host = ControlSecretsHost(HostControl(supervisor));

      // The host offers Bitwarden with the form it describes, and says the key may be another provider's secret.
      final bitwarden = (await host.providers()).singleWhere((each) => each.id == 'bitwarden');
      final session = (bitwarden.schema['properties']! as Map)['session']! as Map;
      expect((session['writeOnly'], session['x-from-secret']), (true, true));
      expect(bitwarden.hasSettings, isTrue);

      await host.configure(name: 'vault', provider: 'bitwarden', settings: {'command': 'bw'}, from: {'session': const SecretSource('form', 'bw_session')});
      final vault = (await host.entries()).singleWhere((each) => each.name == 'vault');
      expect(vault.settings, {'command': 'bw'});
      expect(vault.from, {'session': const SecretSource('form', 'bw_session')});
      final written = await File(cfg).readAsString();
      expect(written, contains('[secrets.providers.vault]'));

      // A secret typed into the settings is refused, and nothing of it reaches the file.
      const typed = 'typed-key-zq9-not-real';
      await expectLater(host.configure(name: 'bad', provider: 'bitwarden', settings: {'session': typed}), throwsA(isA<ControlFailed>()));
      expect(await File(cfg).readAsString(), isNot(contains(typed)));

      await host.removeEntry('vault');
      expect((await host.entries()).any((each) => each.name == 'vault'), isFalse);
      await supervisor.stop();
    });
  });
}
