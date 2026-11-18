// A host's control channel that answers the speech methods of a host making speech-to-text offers, for the tests and the sample
// scenes of Settings > Voice: `speech/enable` makes the whisper provider, as the real one does once the Store has installed the copy.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_tool.dart';

const speechEndpointKind = {
  'id': 'speech_endpoint',
  'title': 'Endpoint',
  'description': 'Sends the audio to a speech-to-text address and model you name',
  'schema': {
    'type': 'object',
    'properties': {
      'url': {'type': 'string', 'title': 'Address', 'x-section': 'endpoint'},
      'model': {'type': ['string', 'null'], 'title': 'Model', 'x-section': 'endpoint'},
      'api_key': {'type': ['string', 'null'], 'title': 'Key', 'writeOnly': true, 'x-sensitive': true, 'x-section': 'endpoint'},
    },
    'required': ['url'],
  },
  'form': {
    'sections': [
      {'id': 'endpoint', 'title': 'Endpoint'},
    ],
    'fields': {
      'api_key': {'widget': 'password'},
    },
  },
};

const whisperModels = [
  {'id': 'base', 'label': 'Base, every language (148 MB)'},
  {'id': 'tiny.en', 'label': 'Tiny, English only (78 MB)'},
];

/// The speech methods of a host with the two offers; `speech/enable` makes the whisper provider, as the real one does once the Store
/// has installed the copy.
final class OffersHost implements HostLink {
  OffersHost({this.installed = false, this.more = const []});

  bool installed;
  bool enabled = false;

  /// Offers the host makes besides the two (Foundry Local, an account's OpenAI key); `speech/enable` of one of them makes a provider
  /// of the offer's id and the model asked for.
  final List<Map<String, Object?>> more;
  final made = <String, Map<String, Object?>>{};
  final calls = <String>[];
  final params = <String, List<Map<String, Object?>>>{};
  final configured = <Map<String, Object?>>[];
  final _out = StreamController<String>();

  Map<String, Object?> get _whisper => {
    'id': 'whisper',
    'title': 'whisper.cpp',
    'summary': 'Speech to text on this host\'s own processor.',
    'license': 'MIT',
    'local': true,
    'where': 'The audio goes to your host and is turned into text there: nothing is sent to another service.',
    'store': 'whisper_store',
    'models': whisperModels,
    'default_model': 'base',
    'installed': installed,
    'version': installed ? '1.9.5' : null,
    'provider': enabled ? 'whisper' : null,
  };

  static const _openai = {
    'id': 'openai',
    'title': 'OpenAI',
    'summary': 'Speech to text by OpenAI\'s service, under your own API key.',
    'local': false,
    'where': 'The audio goes over the internet to api.openai.com, under the key you enter here.',
    'kind': 'speech_endpoint',
    'settings': {'url': 'https://api.openai.com/v1/audio/transcriptions', 'model': 'gpt-transcribe'},
    'models': [
      {'id': 'gpt-transcribe', 'label': 'GPT Transcribe (recommended)'},
    ],
    'default_model': 'gpt-transcribe',
    'provider': null,
  };

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    final given = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    calls.add(method);
    params.putIfAbsent(method, () => []).add(given);
    final Object result;
    switch (method) {
      case 'speech/providers':
        result = {
          'types': [speechEndpointKind],
          'providers': [
            for (final entry in made.entries)
              {
                'name': entry.key,
                'provider': 'speech_endpoint',
                'title': 'Endpoint',
                'description': '',
                'settings': <String, Object?>{},
                'secrets_set': <Object?>[],
                'partials': false,
                'offer': entry.value['offer'],
                'account': entry.value['account'],
              },
            if (enabled)
              {
                'name': 'whisper',
                'provider': 'speech_command',
                'title': 'Command',
                'description': 'Runs a program that reads audio and writes text',
                'settings': <String, Object?>{},
                'secrets_set': <Object?>[],
                'partials': false,
                'offer': 'whisper',
              },
          ],
        };
      case 'speech/offers':
        result = {
          'offers': [
            _whisper,
            for (final offer in more) {...offer, 'provider': made.entries.where((e) => e.value['offer'] == offer['id']).firstOrNull?.key},
            _openai,
          ],
        };
      case 'speech/enable':
        final offer = more.where((each) => each['id'] == given['offer']).firstOrNull;
        if (offer == null) {
          enabled = true;
          result = {'name': 'whisper', 'provider': 'speech_command', 'model': 'base', 'saved': true};
        } else {
          final name = (given['name'] as String?) ?? (offer['account'] != null ? 'openai-${offer['account']}' : '${offer['id']}');
          made[name] = {'offer': offer['id'], 'account': offer['account'], 'model': given['model']};
          result = {'name': name, 'provider': 'speech_endpoint', 'model': given['model'], 'saved': true};
        }
      case 'speech/configure':
        configured.add(given);
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
