// Settings > Voice with a host that makes speech-to-text offers: a local engine installed through the host's Store and made a
// provider, and a named service added with the person's own key. The host here is a link that answers what the real one answers
// (the real one's methods are in crates/canoa-host); the Store is test/fixtures/store_link.dart's.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_grants.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/offers_host.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_link.dart';

FakeStoreHost _store(OffersHost speech) {
  final host = FakeStoreHost([
    FakeStore(
      'whisper_store',
      title: 'whisper.cpp',
      runtime: true,
      summary: 'Turns speech into text on this computer.',
      icon: 'chip',
      endpoints: ['https://github.com/ggml-org/whisper.cpp/releases/download/'],
      latest: '1.9.5',
      plan: {
        'install': planOf(
          'install',
          '1.9.5',
          bytes: 156 * 1024 * 1024,
          downloads: [
            {'url': 'https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip', 'bytes': 8 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'ab' * 32}},
            {'url': 'https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c/ggml-tiny.en.bin', 'bytes': 77 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'cd' * 32}},
          ],
        ),
        'uninstall': planOf('uninstall', '1.9.5'),
      },
    ),
  ]);
  host.afterApply = (store, plan) => speech.installed = true;
  return host;
}

Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 30; i++) {
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 10)));
    await tester.pump(const Duration(milliseconds: 50));
  }
}

Future<(OffersHost, HostSpeechCatalog, SettingsHarness)> _page(
  WidgetTester tester, {
  OffersHost? host,
  FakeStoreHost? store,
  HostGrants? grants,
  CanoaConfig config = const CanoaConfig(),
}) async {
  final link = host ?? OffersHost();
  final catalog = HostSpeechCatalog(
    HostControl.dialing(() async => link),
    grants: grants,
    programs: store?.programs(kind: 'programs'),
  );
  await tester.runAsync(catalog.refresh);
  final harness = await openSettings(tester, section: 'voice', hostSpeech: catalog, config: config);
  await _settle(tester);
  return (link, catalog, harness);
}

void main() {
  testWidgets('a page whose host has no provider offers a local engine and a named service, each saying where the audio goes', (tester) async {
    await _page(tester);
    expect(find.byKey(const Key('voice-none')), findsOneWidget);
    expect(find.textContaining('Install a speech provider below'), findsOneWidget, reason: 'it says what to do, not that there is no support');
    expect(find.byKey(const Key('offer-whisper')), findsOneWidget);
    expect(find.textContaining('nothing is sent to another service'), findsOneWidget);
    expect(find.textContaining('Licence: MIT'), findsOneWidget);
    expect(find.byKey(const Key('offer-install-whisper')), findsOneWidget);
    expect(find.byKey(const Key('offer-openai')), findsOneWidget);
    expect(find.textContaining('api.openai.com'), findsWidgets);
    // The models of the engine are listed to choose from.
    await tester.tap(find.byKey(const Key('offer-model-whisper')));
    await _settle(tester);
    expect(find.text('Tiny, English only (78 MB)'), findsWidgets);
    expect(find.text('Base, every language (148 MB)'), findsWidgets);
  });

  testWidgets('installing the engine plans with the chosen model, installs through the Store and makes it the speech service', (tester) async {
    final speech = OffersHost();
    final store = _store(speech);
    final (_, _, harness) = await _page(tester, host: speech, store: store);
    await tester.tap(find.byKey(const Key('offer-model-whisper')));
    await _settle(tester);
    await tester.tap(find.text('Tiny, English only (78 MB)').last);
    await _settle(tester);
    await tester.tap(find.byKey(const Key('offer-install-whisper')));
    await _settle(tester);
    // The plan is the Store's: the files, their sizes and what they are checked against, confirmed before anything is fetched.
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(store.params['store/plan']!.single['channel'], 'tiny.en', reason: 'the model is the plan\'s channel');
    expect(store.calls.where((call) => call.startsWith('store/apply')), isEmpty);
    await tester.tap(find.byKey(const Key('plan-confirm')));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    expect(store.calls.where((call) => call.startsWith('store/apply')), hasLength(1));
    expect(speech.calls, contains('speech/enable'), reason: 'the installed copy is made a provider');
    expect(speech.params['speech/enable']!.single['offer'], 'whisper');
    expect(harness.last.voice.backend, 'host:whisper', reason: 'the person chose to install it, so it is what dictates');
    expect(find.byKey(const Key('offer-in-use')), findsOneWidget);
  });

  testWidgets('declining the plan installs nothing and sets nothing up', (tester) async {
    final speech = OffersHost();
    final store = _store(speech);
    final (_, _, harness) = await _page(tester, host: speech, store: store);
    await tester.tap(find.byKey(const Key('offer-install-whisper')));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('plan-cancel')));
    await _settle(tester);
    expect(store.calls.where((call) => call.startsWith('store/apply')), isEmpty);
    expect(speech.calls, isNot(contains('speech/enable')));
    expect(harness.applied, isEmpty);
  });

  testWidgets('an engine the Store has installed but nobody set up is set up with one press', (tester) async {
    final speech = OffersHost(installed: true);
    final (_, _, harness) = await _page(tester, host: speech);
    expect(find.byKey(const Key('offer-install-whisper')), findsNothing);
    expect(find.textContaining('Installed: 1.9.5'), findsOneWidget);
    await tester.tap(find.byKey(const Key('offer-enable-whisper')));
    await _settle(tester);
    expect(speech.params['speech/enable']!.single['offer'], 'whisper');
    expect(harness.last.voice.backend, 'host:whisper');
  });

  testWidgets('a named service is added with the person\'s key, starts from the settings its offer carries and is not chosen for them', (tester) async {
    final speech = OffersHost();
    final (_, _, harness) = await _page(tester, host: speech);
    await tester.tap(find.byKey(const Key('offer-add-openai')));
    await _settle(tester);
    expect(find.byKey(const Key('offer-form-openai')), findsOneWidget);
    expect(find.text('https://api.openai.com/v1/audio/transcriptions'), findsOneWidget, reason: 'the address is the offer\'s');
    expect(find.text('gpt-transcribe'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('speech-field-api_key')), 'sk-test');
    await tester.tap(find.byKey(const Key('speech-save')));
    await _settle(tester);
    final sent = speech.configured.single;
    expect(sent['name'], 'openai');
    expect(sent['provider'], 'speech_endpoint');
    final settings = (sent['settings']! as Map).cast<String, Object?>();
    expect(settings['url'], 'https://api.openai.com/v1/audio/transcriptions');
    expect(settings['model'], 'gpt-transcribe');
    expect(settings['api_key'], 'sk-test');
    expect(harness.applied, isEmpty, reason: 'audio goes to a named service only when the person chooses it');
  });

  testWidgets('removing a service asks first: Keep it leaves it, Remove takes it away', (tester) async {
    final speech = OffersHost(more: [_account('work')]);
    await _page(tester, host: speech);
    await tester.tap(find.byKey(const Key('offer-enable-openai:work')));
    await _settle(tester);

    await tester.tap(find.byKey(const Key('offer-remove-openai:work')));
    await _settle(tester);
    expect(find.byKey(const Key('confirm-remove')), findsOneWidget);
    await tester.tap(find.byKey(const Key('confirm-remove-no')));
    await _settle(tester);
    expect(speech.configured, isEmpty, reason: 'Keep it sends nothing');
    expect(find.byKey(const Key('offer-remove-openai:work')), findsOneWidget);

    await tester.tap(find.byKey(const Key('offer-remove-openai:work')));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('confirm-remove-yes')));
    await _settle(tester);
    expect(speech.configured.single, {'name': 'openai-work', 'remove': true});
  });

  testWidgets('a key the host has not given the grants says what is missing instead of failing later', (tester) async {
    // The host says this key may list and use providers and nothing else.
    final speech = OffersHost();
    final store = _store(speech);
    store.grants = [HostGrants.voice];
    final client = store.client();
    final held = HostGrants(() async => client);
    await tester.runAsync(held.refresh);
    expect(held.known, isTrue);
    await _page(tester, host: speech, store: store, grants: held);
    final install = tester.widget<FilledButton>(find.byKey(const Key('offer-install-whisper')));
    expect(install.onPressed, isNull);
    expect(find.byKey(const Key('offer-why-whisper')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('offer-why-whisper'))).data, contains('store.programs'));
    final add = tester.widget<FilledButton>(find.byKey(const Key('offer-add-openai')));
    expect(add.onPressed, isNull);
  });

  testWidgets('Foundry Local says what is missing until its service lists a Whisper model', (tester) async {
    final down = OffersHost(more: [_foundry(unavailable: 'Foundry Local\'s service is not running: start it (`foundry server start`), then load a Whisper model.')]);
    await _page(tester, host: down);
    expect(find.byKey(const Key('offer-foundry')), findsOneWidget);
    expect(find.textContaining('nothing is sent to another service'), findsWidgets);
    expect(tester.widget<FilledButton>(find.byKey(const Key('offer-unavailable-foundry'))).onPressed, isNull);
    expect(tester.widget<Text>(find.byKey(const Key('offer-why-foundry'))).data, contains('not running'));
    expect(find.byKey(const Key('offer-model-foundry')), findsNothing, reason: 'there is no model to choose yet');
    expect(down.calls, isNot(contains('speech/enable')));
  });

  testWidgets('a running Foundry Local is set up with a Whisper model the person picks, and becomes the speech service', (tester) async {
    final speech = OffersHost(more: [_foundry(models: ['openai-whisper-tiny-generic-cpu', 'openai-whisper-base-generic-cpu'])]);
    final (_, _, harness) = await _page(tester, host: speech);
    expect(find.byKey(const Key('offer-why-foundry')), findsNothing);
    await tester.tap(find.byKey(const Key('offer-model-foundry')));
    await _settle(tester);
    await tester.tap(find.text('openai-whisper-base-generic-cpu').last);
    await _settle(tester);
    await tester.tap(find.byKey(const Key('offer-enable-foundry')));
    await _settle(tester);
    expect(speech.params['speech/enable']!.single, {'offer': 'foundry', 'model': 'openai-whisper-base-generic-cpu'});
    expect(harness.last.voice.backend, 'host:foundry');
    expect(find.byKey(const Key('offer-in-use')), findsOneWidget);
  });

  testWidgets('each OpenAI API key account is an offer named for the account, one that is no API key says why, and Add uses the account\'s key without asking for one', (tester) async {
    final speech = OffersHost(more: [
      _account('work'),
      _account('login', unavailable: 'a ChatGPT sign-in is not an OpenAI API key, and Canoa has no measured way to send audio to OpenAI with it, so it is not offered'),
    ]);
    final (_, _, harness) = await _page(tester, host: speech);
    expect(find.textContaining('account work'), findsWidgets, reason: 'the offer names the account whose key pays');
    expect(find.textContaining('API key of your account work'), findsWidgets);
    expect(tester.widget<FilledButton>(find.byKey(const Key('offer-unavailable-openai:login'))).onPressed, isNull);
    expect(tester.widget<Text>(find.byKey(const Key('offer-why-openai:login'))).data, contains('not an OpenAI API key'));
    expect(find.byKey(const Key('offer-form-openai:work')), findsNothing, reason: 'the key is the account\'s: no form asks for one');
    await tester.tap(find.byKey(const Key('offer-enable-openai:work')));
    await _settle(tester);
    expect(speech.params['speech/enable']!.single, {'offer': 'openai:work', 'model': 'gpt-transcribe'});
    expect(speech.configured, isEmpty, reason: 'no key was typed or sent');
    expect(harness.last.voice.backend, 'host:openai-work');
    // The provider is listed as the account's, so the person can tell which key pays when they choose it.
    expect(find.textContaining('openai-work (API key, account work)'), findsWidgets);
  });

  test('the model a person chose reaches the provider when the host lists it, and a model of another service does not', () async {
    final link = _ModelHost();
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => link), capture: _Silent());
    await catalog.refresh();
    final backend = catalog.backends.single;
    expect((await backend.models()).map((m) => m.id), ['gpt-transcribe', 'whisper-1']);
    for (final model in ['whisper-1', 'something-else', null]) {
      link.opened.clear();
      final heard = backend.start(model: model);
      await heard.drain<void>();
      expect(link.opened.single['model'], model == 'whisper-1' ? 'whisper-1' : isNull, reason: 'asked for $model');
    }
  });
}

/// A host with one provider that lists two models and answers an empty recording.
final class _ModelHost implements HostLink {
  final opened = <Map<String, Object?>>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final given = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    final Object result = switch (request['method']) {
      'speech/providers' => {
        'types': <Object?>[],
        'providers': [
          {
            'name': 'cloud',
            'provider': 'speech_endpoint',
            'settings': {'url': 'https://api.openai.com/v1/audio/transcriptions'},
            'secrets_set': <Object?>[],
            'models': [
              {'id': 'gpt-transcribe', 'label': 'GPT Transcribe'},
              {'id': 'whisper-1', 'label': 'Whisper'},
            ],
          },
        ],
      },
      'speech/open' => () {
        opened.add(given);
        return {'session': 's', 'partials': false};
      }(),
      'speech/finish' => {'text': 'hi', 'front_matter': <String, Object?>{}},
      _ => <String, Object?>{},
    };
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() async {
    await _out.close();
  }
}

final class _Silent implements AudioCapture {
  @override
  Stream<Uint8List> open({required int rate}) => const Stream.empty();
}

Map<String, Object?> _foundry({String? unavailable, List<String> models = const []}) => {
  'id': 'foundry',
  'title': 'Foundry Local',
  'summary': 'Speech to text by a Whisper model that Foundry Local runs on this computer.',
  'local': true,
  'where': 'The audio is written to a temporary file on this computer, handed to Foundry Local\'s service on a loopback address and removed afterwards: nothing is sent to another service.',
  'service': 'foundry_local',
  'models': [for (final id in models) {'id': id, 'label': id}],
  'default_model': models.firstOrNull,
  'installed': unavailable == null,
  'unavailable': unavailable,
  'provider': null,
};

Map<String, Object?> _account(String name, {String? unavailable}) => {
  'id': 'openai:$name',
  'title': 'OpenAI, account $name',
  'summary': 'Speech to text by OpenAI\'s service, under the API key of your account $name.',
  'local': false,
  'where': 'The audio goes over the internet to api.openai.com, under the API key of your account $name in Canoa, and OpenAI\'s terms say what it does with it.',
  'account': name,
  'models': [
    {'id': 'gpt-transcribe', 'label': 'GPT Transcribe (recommended)'},
  ],
  'default_model': 'gpt-transcribe',
  'unavailable': unavailable,
  'provider': null,
};
