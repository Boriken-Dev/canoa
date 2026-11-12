// An in-memory ACP v1 agent, a stand-in like the scripted mode of canoa-host's stdio_agent example: it offers a mode, a model and an effort
// as config options with their ACP categories, refuses the model `refuse-me` with -32602 as ACP says an agent refuses a
// value, answers each prompt with one message, and records every request in order. `start` is shaped as the app's StartAgent,
// so the app starts it in place of a process.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/chat/agent_events.dart' show ConfigOption, parseConfigOptions;

final class OptionsAgent {
  OptionsAgent({this.images = true, this.files = true, this.failStart, this.modes = const [('ask', 'Ask first'), ('edits', 'Accept edits')]})
      : mode = modes.isEmpty ? '' : modes.first.$1;

  /// What it says it takes with a prompt.
  final bool images, files;

  /// Why starting it fails, when it should.
  final String? failStart;

  /// Every request each started copy received, in order, as sent.
  final requests = <Map<String, dynamic>>[];

  /// The commands it was started as.
  final starts = <Object>[];
  final clients = <VerifiedAgent>[];

  /// The links the app opened, each told whether the app has closed it.
  final links = <_WireLink>[];

  String model = 'default', effort = 'low', mode;

  /// The modes it offers, each with its label: Claude's translator offers five, Codex's four with no Bypass.
  final List<(String, String)> modes;

  /// Modes as `coqui-acp-claude` offers them, and as `coqui-acp-codex` does, with no Bypass.
  static const claudeModes = [('manual', 'Manual'), ('accept_edits', 'Accept edits'), ('auto', 'Auto'), ('plan', 'Plan'), ('bypass', 'Bypass')];
  static const codexModes = [('manual', 'Manual'), ('accept_edits', 'Accept edits'), ('auto', 'Auto'), ('plan', 'Plan')];

  /// Whether it also offers `session/resume`, which a reopening then prefers to `session/load`.
  bool offersResume = false;

  /// What `session/load` replays before it answers: updates for the session `o1`, as an agent sends a conversation again.
  final replay = <Map<String, Object?>>[];

  /// Ends the last started copy's connection, as an agent that went away does.
  void disconnect() => _ends.last();
  final _ends = <void Function()>[];

  /// Whether `session/load` is answered only when [releaseLoad] is called, so a test can act while a session opens again.
  bool holdLoad = false;
  final _heldLoad = <void Function()>[];

  void releaseLoad() {
    final held = [..._heldLoad];
    _heldLoad.clear();
    for (final answer in held) {
      answer();
    }
  }

  /// Mode values it refuses, as an agent refuses one it cannot take.
  final refuseModes = <String>{};

  /// What the person answered to each request it asked, by the request's id.
  final answers = <Object?, Map<String, dynamic>>{};
  final _asks = <void Function(Map<String, Object?>)>[];

  /// Asks the last started copy's person for leave, as an agent does: [options] are ACP's, [rawInput] the tool's input.
  void ask(int id, List<Map<String, Object?>> options, {Object? rawInput, String kind = 'edit', String title = 'Edit a.txt'}) {
    _asks.last({
      'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
      'params': {
        'sessionId': 'o1',
        'toolCall': {'toolCallId': 't$id', 'title': title, 'kind': kind, 'rawInput': rawInput ?? {'file_path': 'a.txt'}},
        'options': options,
      },
    });
  }

  /// The agent changes its own mode, and says so as an agent does.
  void agentSets(String value) {
    mode = value;
    _asks.last({
      'jsonrpc': '2.0', 'method': 'session/update',
      'params': {'sessionId': 'o1', 'update': {'sessionUpdate': 'config_option_update', 'configOptions': options}},
    });
  }

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
    if (modes.isNotEmpty) {'id': 'mode', 'name': 'Mode', 'category': 'mode', 'type': 'select', 'currentValue': mode, 'options': [
      for (final (value, name) in modes) {'value': value, 'name': name},
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

  /// The usage report it advertises the extension for and answers `_coqui_acp/usage` with, as the model writes it; none, and it
  /// advertises nothing and answers the method as an agent without it does, with -32601.
  Map<String, Object?>? usage;

  /// What the client said of the extension in `initialize`.
  Object? usageOptIn;

  /// Tells the last started copy a report changed, as an agent does: for [session] when one is named.
  void pushUsage(Map<String, Object?> report, {String? session}) {
    usage = report;
    _asks.last({
      'jsonrpc': '2.0', 'method': '_coqui_acp/usage_update',
      'params': {'sessionId': ?session, 'report': report},
    });
  }

  /// The methods received, in order.
  List<String> get methods => [for (final request in requests) request['method'] as String];

  Map<String, dynamic> last(String method) => requests.lastWhere((request) => request['method'] == method);

  /// Ends the turn waiting longest.
  void finishTurn() {
    if (_held.isNotEmpty) _held.removeAt(0)();
  }

  /// Started as a process would be, for the sources the app still starts itself.
  Future<VerifiedAgent> start(AgentCommand command, {AgentDecider? decider, AgentElicitor? elicitor, String? workingDirectory,
      Map<String, String> environment = const {}}) async {
    starts.add(command);
    if (failStart != null) throw AcpFailure(failStart!);
    final wire = _wire();
    final client = VerifiedAgent.fromChild(
      label: command.label, messages: wire.messages, decider: decider, elicitor: elicitor, close: wire.close, send: wire.send,
    );
    clients.add(client);
    await client.initialize();
    return client;
  }

  /// Reached as a source of the host is, over a link: what the app is given in place of the library's.
  HostLink openLink(String name) {
    starts.add(name);
    if (failStart != null) throw StateError(failStart!);
    final wire = _wire();
    final link = _WireLink(wire.messages, wire.send, wire.close);
    links.add(link);
    return link;
  }

  ({Stream<String> messages, Future<void> Function(String) send, Future<void> Function() close}) _wire() {
    final incoming = StreamController<String>();
    void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
    void update(String session, Map<String, Object?> body) => incoming.add(jsonEncode({
      'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': session, 'update': body},
    }));
    _asks.add((message) => incoming.add(jsonEncode(message)));
    _ends.add(() => incoming.close());
    return (
      messages: incoming.stream,
      close: () async => incoming.close(),
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        final id = message['id'];
        final method = message['method'];
        if (method == null) {
          if (id != null) answers[id] = message;
          return;
        }
        requests.add(message);
        final params = message['params'] as Map<String, dynamic>? ?? const {};
        switch (method) {
          case 'initialize':
            usageOptIn = ((params['clientCapabilities'] ?? params['capabilities']) as Map?)?['_meta'];
            reply(id, {'protocolVersion': 1, 'agentCapabilities': {
              'loadSession': true,
              if (offersResume) 'sessionCapabilities': {'resume': <String, Object?>{}},
              'promptCapabilities': {'image': images, 'embeddedContext': files},
              if (usage != null) '_meta': {'dev.boriken.coqui.acp/usage': {'version': 1, 'sources': ['reported'], 'spend': true}},
            }, 'agentInfo': {'name': 'options-agent', 'version': '1.0.0'}});
          case '_coqui_acp/usage':
            if (usage == null) {
              incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'error': {'code': -32601, 'message': 'no such method'}}));
            } else {
              reply(id, usage);
            }
          case 'session/new':
            reply(id, {'sessionId': 'o1', 'configOptions': options});
          case 'session/set_config_option':
            final value = params['value'];
            if (value == 'refuse-me' || (params['configId'] == 'mode' && refuseModes.contains(value))) {
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
          case 'session/load' || 'session/resume':
            if (method == 'session/load') {
              for (final body in replay) {
                update('o1', body);
              }
            }
            void answer() {
              // A reopened agent starts in its own default mode again, whatever the session was left in.
              mode = modes.isEmpty ? '' : modes.first.$1;
              reply(id, {'configOptions': options});
            }
            if (holdLoad) {
              _heldLoad.add(answer);
            } else {
              answer();
            }
          default:
            if (id != null) reply(id, <String, Object?>{});
        }
      },
    );
  }
}

final class _WireLink implements HostLink {
  _WireLink(this.messages, this._send, this._close);

  @override
  final Stream<String> messages;
  final Future<void> Function(String) _send;
  final Future<void> Function() _close;

  @override
  Future<void> send(String message) => _send(message);

  /// Whether the app has closed it, as the host ends an agent when its link ends.
  bool closed = false;

  @override
  Future<void> close() {
    closed = true;
    return _close();
  }
}

/// What [OptionsAgent] offers at the start, as the app reads it.
abstract final class OptionsAgentOptions {
  static List<ConfigOption> get parsed => parseConfigOptions(OptionsAgent().options)!;
}
