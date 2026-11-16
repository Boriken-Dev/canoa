// Settings > Voice with a host: the providers its canoa.cfg defines are services to choose, and each one's settings are drawn from
// the schema and form its kind declares, then written back through `speech/configure`. The host here is a link that answers what
// the real one answers (the real one is `host_speech_host_test.dart`).
import 'dart:async';
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';

const _endpoint = {
  'id': 'speech_endpoint',
  'title': 'Endpoint',
  'description': 'Sends the audio to a speech-to-text address and model you name',
  'schema': {
    'type': 'object',
    'properties': {
      'url': {'type': 'string', 'title': 'Address', 'x-section': 'endpoint'},
      'model': {'type': ['string', 'null'], 'title': 'Model', 'x-section': 'endpoint'},
      'api_key': {'type': ['string', 'null'], 'title': 'Key', 'writeOnly': true, 'x-sensitive': true, 'x-section': 'endpoint'},
      'max_seconds': {'type': 'integer', 'title': 'Longest recording', 'x-section': 'endpoint', 'x-advanced': true, 'default': 300},
    },
    'required': ['url'],
  },
  'form': {
    'sections': [
      {'id': 'endpoint', 'title': 'Endpoint'},
    ],
    'fields': {
      'url': {'placeholder': 'https://host/v1/audio/transcriptions'},
      'api_key': {'widget': 'password'},
      'max_seconds': {'unit': 's'},
    },
  },
};

/// A control channel with a list of providers in memory that `speech/configure` changes.
final class _Host implements HostLink {
  final providers = <Map<String, Object?>>[
    {
      'name': 'cloud',
      'provider': 'speech_endpoint',
      'title': 'Endpoint',
      'description': 'Sends the recording to a speech-to-text endpoint',
      'settings': {'url': 'http://127.0.0.1:9/x', 'model': 'whisper-1'},
      'secrets_set': ['api_key'],
      'partials': false,
    },
  ];
  final configured = <Map<String, Object?>>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    final Object result;
    switch (request['method']) {
      case 'speech/providers':
        result = {
          'types': [_endpoint],
          'providers': providers,
        };
      case 'speech/configure':
        configured.add(params);
        if (params['remove'] == true) providers.removeWhere((p) => p['name'] == params['name']);
        result = {'saved': true};
      default:
        _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32601, 'message': 'no'}}));
        return;
    }
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() async {
    await _out.close();
  }
}

void main() {
  test('a [speech] table the app did not write is kept when the app writes its file', () {
    const text = '[speech.cloud]\nprovider = "speech_endpoint"\nurl = "http://127.0.0.1:9/x"\napi_key = "sk-test"\n';
    final config = CanoaConfig.parse(text);
    final back = config.encode();
    expect(back, contains('[speech.cloud]'));
    expect(back, contains('sk-test'));
    expect(CanoaConfig.parse(back).encode(), back);
  });

  testWidgets('a host provider is a service to choose, its settings are drawn from its kind, and saving writes them with no key', (tester) async {
    final link = _Host();
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => link));
    await tester.runAsync(catalog.refresh);
    final harness = await openSettings(tester, section: 'voice', hostSpeech: catalog);
    await tester.pumpAndSettle();
    expect(harness.applied, isEmpty);
    // The provider is listed among the services, and the page says the audio goes to the host.
    expect(find.byKey(const Key('speech-provider-cloud')), findsOneWidget);
    expect(find.textContaining('Sends the recording'), findsOneWidget);

    await tester.tap(find.byKey(const Key('speech-edit-cloud')));
    await tester.pumpAndSettle();
    // The fields come from the schema and form; the key has no value shown, only that it is set.
    expect(find.text('Address'), findsOneWidget);
    expect(find.text('Model'), findsOneWidget);
    expect(find.text('Key'), findsOneWidget);
    expect(find.text('whisper-1'), findsOneWidget);
    expect(find.text('Set. Type to replace'), findsOneWidget);
    expect(find.text('Longest recording'), findsNothing, reason: 'advanced settings are behind a disclosure');
    await tester.tap(find.byKey(const Key('speech-advanced')));
    await tester.pumpAndSettle();
    expect(find.text('Longest recording'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('speech-field-model')), 'whisper-2');
    await tester.tap(find.byKey(const Key('speech-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(link.configured, hasLength(1));
    final sent = link.configured.single;
    expect(sent['name'], 'cloud');
    expect(sent['provider'], 'speech_endpoint');
    final settings = (sent['settings']! as Map).cast<String, Object?>();
    expect(settings['model'], 'whisper-2');
    expect(settings['url'], 'http://127.0.0.1:9/x');
    expect(settings.containsKey('api_key'), isFalse, reason: 'a key the form never showed is not sent, so the host keeps it');
    expect(sent.containsKey('clear'), isFalse);
  });

  testWidgets('a provider can be removed from the page', (tester) async {
    final link = _Host();
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => link));
    await tester.runAsync(catalog.refresh);
    await openSettings(tester, section: 'voice', hostSpeech: catalog);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('speech-remove-cloud')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(link.configured.single, {'name': 'cloud', 'remove': true});
    expect(find.byKey(const Key('speech-provider-cloud')), findsNothing);
  });

  testWidgets('a host that gives this page no grant says so, and the page still opens', (tester) async {
    final catalog = HostSpeechCatalog(HostControl.dialing(() async => throw StateError('the host is not running')));
    await tester.runAsync(catalog.refresh);
    await openSettings(tester, section: 'voice', hostSpeech: catalog);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('speech-host-problem')), findsOneWidget);
  });
}
