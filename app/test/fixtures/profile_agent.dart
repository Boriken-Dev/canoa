// An ACP v1 agent on its standard streams that answers `initialize` as a real adapter does (`--as=claude`, `codex` or `pi`:
// Claude's adapter 0.99.1, Codex's 2.5.0, pi-acp 0.0.34) and writes to `--record=<file>` one JSON line per session request:
// its params, and whether each variable named by `--env=A,B` was set in its environment. It offers a model and an effort.
import 'dart:convert';
import 'dart:io';

Map<String, Object?> _initialize(String kind) => switch (kind) {
  'claude' => {
    'protocolVersion': 1,
    'agentInfo': {'name': '@agentclientprotocol/claude-agent-acp', 'title': 'Claude Agent', 'version': '0.99.1'},
    'agentCapabilities': {
      'loadSession': true,
      'mcpCapabilities': {'http': true, 'sse': true},
      'sessionCapabilities': {
        'additionalDirectories': {},
        'resume': {},
        'systemPrompt': {'mode': {'override': {}}},
      },
    },
  },
  'codex' => {
    'protocolVersion': 1,
    'agentInfo': {'name': '@agentclientprotocol/codex-acp', 'title': 'Codex', 'version': '2.5.0'},
    'agentCapabilities': {
      'loadSession': true,
      'mcpCapabilities': {'acp': false, 'http': true, 'sse': false},
      'sessionCapabilities': {'resume': {}, 'additionalDirectories': {}, 'systemPrompt': {}},
    },
  },
  _ => {
    'protocolVersion': 1,
    'agentInfo': {'name': 'pi-acp', 'title': 'pi ACP adapter', 'version': '0.0.34'},
    'agentCapabilities': {
      'loadSession': true,
      'mcpCapabilities': {'http': false, 'sse': false},
      'sessionCapabilities': {'list': {}, 'delete': {}},
    },
  },
};

List<Map<String, Object?>> _options(String model, String effort) => [
  {
    'id': 'model', 'name': 'Model', 'category': 'model', 'type': 'select', 'currentValue': model,
    'options': [{'value': 'default', 'name': 'Default'}, {'value': 'opus', 'name': 'Opus'}],
  },
  {
    'id': 'effort', 'name': 'Effort', 'category': 'thought_level', 'type': 'select', 'currentValue': effort,
    'options': [{'value': 'low', 'name': 'Low'}, {'value': 'high', 'name': 'High'}],
  },
];

void main(List<String> args) async {
  String? arg(String name) => args.where((arg) => arg.startsWith('--$name=')).firstOrNull?.substring(name.length + 3);
  final kind = arg('as') ?? 'pi';
  final record = arg('record');
  final names = (arg('env') ?? '').split(',').where((name) => name.isNotEmpty);
  var model = 'default', effort = 'low';
  void send(Map<String, Object?> message) => stdout.writeln(jsonEncode({'jsonrpc': '2.0', ...message}));
  await for (final line in stdin.transform(utf8.decoder).transform(const LineSplitter())) {
    final message = jsonDecode(line) as Map<String, dynamic>;
    final id = message['id'];
    final method = message['method'];
    final params = message['params'];
    if (id == null) continue;
    switch (method) {
      case 'initialize':
        send({'id': id, 'result': _initialize(kind)});
      case 'session/new' || 'session/load' || 'session/resume':
        if (record != null) {
          File(record).writeAsStringSync('${jsonEncode({
            'method': method,
            'params': params,
            'env': {for (final name in names) name: Platform.environment.containsKey(name)},
          })}\n', mode: FileMode.append, flush: true);
        }
        send({'id': id, 'result': {
          if (method == 'session/new') 'sessionId': 'profiled',
          'configOptions': _options(model, effort),
        }});
      case 'session/set_config_option':
        final value = (params as Map)['value'] as String;
        if (params['configId'] == 'model') model = value;
        if (params['configId'] == 'effort') effort = value;
        send({'id': id, 'result': {'configOptions': _options(model, effort)}});
      default:
        send({'id': id, 'error': {'code': -32601, 'message': 'Method not found'}});
    }
  }
}
