// An in-memory ACP v1 agent, a stand-in like integration_test/scripted_agent.mjs: it offers a mode, a model and an effort
// as config options with their ACP categories, refuses the model `refuse-me` with -32602 as ACP says an agent refuses a
// value, answers each prompt with one message, and records every request in order. `start` is shaped as the app's StartAgent,
// so the app starts it in place of a process.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/agent_events.dart' show ConfigOption, parseConfigOptions;

final class OptionsAgent {
  OptionsAgent({this.images = true, this.files = true, this.failStart});

  /// What it says it takes with a prompt.
  final bool images, files;

  /// Why starting it fails, when it should.
  final String? failStart;

  /// Every request each started copy received, in order, as sent.
  final requests = <Map<String, dynamic>>[];

  /// The commands it was started as.
  final starts = <AgentCommand>[];
  final clients = <VerifiedAgent>[];

  String model = 'default', effort = 'low', mode = 'ask';

  /// Whether a prompt's turn ends only when [finishTurn] is called.
  bool holdTurns = false;
  final _held = <void Function()>[];

  /// Whether each answer to `session/set_config_option` waits for [releaseChoices].
  bool holdChoices = false;
  final _heldChoices = <void Function()>[];

  /// Answers every choice held.
  void releaseChoices() {
    final held = [..._heldChoices];
    _heldChoices.clear();
    for (final answer in held) {
      answer();
    }
  }

  List<Map<String, Object?>> get options => [
    {'id': 'mode', 'name': 'Mode', 'category': 'mode', 'type': 'select', 'currentValue': mode, 'options': [
      {'value': 'ask', 'name': 'Ask first'},
      {'value': 'edits', 'name': 'Accept edits'},
    ]},
    {'id': 'model', 'name': 'Model', 'category': 'model', 'type': 'select', 'currentValue': model, 'options': [
      {'value': 'default', 'name': 'Default model'},
      {'value': 'opus', 'name': 'Opus'},
      {'value': 'refuse-me', 'name': 'Refused'},
    ]},
    {'id': 'effort', 'name': 'Effort', 'category': 'thought_level', 'type': 'select', 'currentValue': effort, 'options': [
      {'value': 'low', 'name': 'Low'},
      {'value': 'high', 'name': 'High'},
    ]},
  ];

  /// The methods received, in order.
  List<String> get methods => [for (final request in requests) request['method'] as String];

  Map<String, dynamic> last(String method) => requests.lastWhere((request) => request['method'] == method);

  /// Ends the turn waiting longest.
  void finishTurn() {
    if (_held.isNotEmpty) _held.removeAt(0)();
  }

  Future<VerifiedAgent> start(AgentCommand command, {AgentDecider? decider, AgentElicitor? elicitor, String? workingDirectory,
      Map<String, String> environment = const {}}) async {
    starts.add(command);
    if (failStart != null) throw AcpFailure(failStart!);
    final incoming = StreamController<String>();
    void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
    void update(String session, Map<String, Object?> body) => incoming.add(jsonEncode({
      'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': session, 'update': body},
    }));
    late final VerifiedAgent client;
    client = VerifiedAgent.fromChild(
      label: command.label,
      messages: incoming.stream,
      decider: decider,
      elicitor: elicitor,
      close: () async => incoming.close(),
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final id = message['id'];
        final method = message['method'];
        if (method == null) return;
        requests.add(message);
        final params = message['params'] as Map<String, dynamic>? ?? const {};
        switch (method) {
          case 'initialize':
            reply(id, {'protocolVersion': 1, 'agentCapabilities': {
              'loadSession': true,
              'promptCapabilities': {'image': images, 'embeddedContext': files},
            }, 'agentInfo': {'name': 'options-agent', 'version': '1.0.0'}});
          case 'session/new':
            reply(id, {'sessionId': 'o1', 'configOptions': options});
          case 'session/set_config_option':
            final value = params['value'];
            if (value == 'refuse-me') {
              incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32602, 'message': 'That model is not available'}}));
              return;
            }
            void answer() {
              switch (params['configId']) {
                case 'model':
                  model = value as String;
                case 'effort':
                  effort = value as String;
                case 'mode':
                  mode = value as String;
              }
              reply(id, {'configOptions': options});
            }
            if (holdChoices) {
              _heldChoices.add(answer);
            } else {
              answer();
            }
          case 'session/prompt':
            void end() {
              update('o1', {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Done.'}});
              reply(id, {'stopReason': 'end_turn'});
            }
            if (holdTurns) {
              _held.add(end);
            } else {
              end();
            }
          default:
            if (id != null) reply(id, <String, Object?>{});
        }
      },
    );
    clients.add(client);
    await client.initialize();
    return client;
  }
}

/// What [OptionsAgent] offers at the start, as the app reads it.
abstract final class OptionsAgentOptions {
  static List<ConfigOption> get parsed => parseConfigOptions(OptionsAgent().options)!;
}
